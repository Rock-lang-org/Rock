//! Bound, descriptor-backed AOT bodies. These types never enter concrete MIR's
//! TypeId locals or LLVM's concrete type mapper.
use super::{MirCallableKey, MirPayloadDropPlan};
use crate::ids::{DefId, TypeId};
use crate::type_services::kind::Kind;
use crate::types::{GenericParamId, NominalTypeKind, ReceiverMode};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MirErasedKey {
    pub definition: DefId,
    pub static_args: Vec<TypeId>,
}

#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum MirErasedType<T = TypeId> {
    Concrete(T),
    /// Index into the enclosing signature, never an unbound ordinary Generic.
    Parameter(u32),
    Reference {
        inner: Box<Self>,
        mutable: bool,
    },
    Pointer(Box<Self>),
    Tuple(Vec<Self>),
    Array {
        element: Box<Self>,
        len: usize,
    },
    Nominal {
        id: DefId,
        flavor: NominalTypeKind,
        args: Vec<Self>,
    },
    Apply {
        constructor: Box<Self>,
        args: Vec<Self>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MirErasedParameter {
    pub source: GenericParamId,
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MirErasedDictionarySchema<T = TypeId> {
    pub subject: MirErasedType<T>,
    pub trait_id: DefId,
    pub trait_args: Vec<MirErasedType<T>>,
    pub members: Vec<MirErasedDictionaryMember<T>>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MirErasedDictionaryMember<T = TypeId> {
    pub member_id: DefId,
    pub receiver: ReceiverMode,
    /// Includes the subject value at index zero; the receiver mode supplies access.
    pub params: Vec<MirErasedType<T>>,
    pub ret: MirErasedType<T>,
}

impl MirErasedDictionarySchema {
    pub(crate) fn substitute(&self, bindings: &[MirErasedType]) -> Result<Self, MirErasedError> {
        Ok(Self {
            subject: self.subject.substitute(bindings)?,
            trait_id: self.trait_id,
            trait_args: self
                .trait_args
                .iter()
                .map(|ty| ty.substitute(bindings))
                .collect::<Result<_, _>>()?,
            members: self
                .members
                .iter()
                .map(|member| {
                    Ok(MirErasedDictionaryMember {
                        member_id: member.member_id,
                        receiver: member.receiver,
                        params: member
                            .params
                            .iter()
                            .map(|ty| ty.substitute(bindings))
                            .collect::<Result<_, _>>()?,
                        ret: member.ret.substitute(bindings)?,
                    })
                })
                .collect::<Result<_, MirErasedError>>()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(bound(deserialize = "T: serde::Deserialize<'de> + Ord"))]
pub struct MirErasedSignature<T = TypeId> {
    pub parameters: Vec<MirErasedParameter>,
    pub params: Vec<MirErasedType<T>>,
    pub ret: MirErasedType<T>,
    pub dictionaries: Vec<MirErasedDictionarySchema<T>>,
    /// Descriptor application witnesses required by the public signature.
    /// Concrete callers validate kinds and supply global descriptors.
    pub layouts: Vec<MirErasedType<T>>,
    /// This slice does not permit an opaque callee to retain input borrows.
    pub require_borrow_free: BTreeSet<MirErasedType<T>>,
}

impl<T> MirErasedType<T> {
    pub fn map_types<U, E>(
        &self,
        map: &mut impl FnMut(&T) -> Result<U, E>,
    ) -> Result<MirErasedType<U>, E> {
        Ok(match self {
            Self::Concrete(ty) => MirErasedType::Concrete(map(ty)?),
            Self::Parameter(index) => MirErasedType::Parameter(*index),
            Self::Reference { inner, mutable } => MirErasedType::Reference {
                inner: Box::new(inner.map_types(map)?),
                mutable: *mutable,
            },
            Self::Pointer(inner) => MirErasedType::Pointer(Box::new(inner.map_types(map)?)),
            Self::Tuple(fields) => MirErasedType::Tuple(
                fields
                    .iter()
                    .map(|ty| ty.map_types(map))
                    .collect::<Result<_, _>>()?,
            ),
            Self::Array { element, len } => MirErasedType::Array {
                element: Box::new(element.map_types(map)?),
                len: *len,
            },
            Self::Nominal { id, flavor, args } => MirErasedType::Nominal {
                id: *id,
                flavor: *flavor,
                args: args
                    .iter()
                    .map(|ty| ty.map_types(map))
                    .collect::<Result<_, _>>()?,
            },
            Self::Apply { constructor, args } => MirErasedType::Apply {
                constructor: Box::new(constructor.map_types(map)?),
                args: args
                    .iter()
                    .map(|ty| ty.map_types(map))
                    .collect::<Result<_, _>>()?,
            },
        })
    }
    pub fn try_remap_nominals<E>(
        &mut self,
        remap: &mut impl FnMut(DefId) -> Result<DefId, E>,
    ) -> Result<(), E> {
        match self {
            Self::Concrete(_) | Self::Parameter(_) => {}
            Self::Reference { inner, .. } | Self::Pointer(inner) => {
                inner.try_remap_nominals(remap)?
            }
            Self::Tuple(fields) => {
                for field in fields {
                    field.try_remap_nominals(remap)?;
                }
            }
            Self::Array { element, .. } => element.try_remap_nominals(remap)?,
            Self::Nominal { id, args, .. } => {
                *id = remap(*id)?;
                for arg in args {
                    arg.try_remap_nominals(remap)?;
                }
            }
            Self::Apply { constructor, args } => {
                constructor.try_remap_nominals(remap)?;
                for arg in args {
                    arg.try_remap_nominals(remap)?;
                }
            }
        }
        Ok(())
    }
}

impl<T> MirErasedSignature<T> {
    pub fn map_types<U: Ord, E>(
        &self,
        map: &mut impl FnMut(&T) -> Result<U, E>,
    ) -> Result<MirErasedSignature<U>, E> {
        Ok(MirErasedSignature {
            parameters: self.parameters.clone(),
            params: self
                .params
                .iter()
                .map(|ty| ty.map_types(map))
                .collect::<Result<_, _>>()?,
            ret: self.ret.map_types(map)?,
            dictionaries: self
                .dictionaries
                .iter()
                .map(|dictionary| {
                    Ok(MirErasedDictionarySchema {
                        subject: dictionary.subject.map_types(map)?,
                        trait_id: dictionary.trait_id,
                        trait_args: dictionary
                            .trait_args
                            .iter()
                            .map(|ty| ty.map_types(map))
                            .collect::<Result<_, _>>()?,
                        members: dictionary
                            .members
                            .iter()
                            .map(|member| {
                                Ok(MirErasedDictionaryMember {
                                    member_id: member.member_id,
                                    receiver: member.receiver,
                                    params: member
                                        .params
                                        .iter()
                                        .map(|ty| ty.map_types(map))
                                        .collect::<Result<_, _>>()?,
                                    ret: member.ret.map_types(map)?,
                                })
                            })
                            .collect::<Result<_, E>>()?,
                    })
                })
                .collect::<Result<_, E>>()?,
            layouts: self
                .layouts
                .iter()
                .map(|ty| ty.map_types(map))
                .collect::<Result<_, _>>()?,
            require_borrow_free: self
                .require_borrow_free
                .iter()
                .map(|ty| ty.map_types(map))
                .collect::<Result<_, _>>()?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MirErasedLocal(pub u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirErasedProjection {
    Field(u32),
    Deref,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirErasedPlace {
    pub local: MirErasedLocal,
    pub projection: Vec<MirErasedProjection>,
}
impl MirErasedPlace {
    pub fn local(local: MirErasedLocal) -> Self {
        Self {
            local,
            projection: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirErasedOperand {
    Move(MirErasedPlace),
    Copy(MirErasedPlace),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirErasedLiteral {
    Int(i64),
    Bool(bool),
    Unit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirErasedRvalue {
    Use(MirErasedOperand),
    Literal(MirErasedLiteral),
    Aggregate(Vec<MirErasedOperand>),
    /// A scoped address, not an implicit owned allocation or metadata value.
    Borrow {
        place: MirErasedPlace,
        mutable: bool,
    },
    Call {
        target: MirErasedKey,
        type_args: Vec<MirErasedType>,
        dictionaries: Vec<u32>,
        args: Vec<MirErasedOperand>,
    },
    DictionaryCall {
        dictionary: u32,
        member: u32,
        receiver: MirErasedPlace,
        args: Vec<MirErasedOperand>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirErasedAssignment {
    pub destination: MirErasedLocal,
    pub value: MirErasedRvalue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirErasedFunction {
    pub key: MirErasedKey,
    pub symbol: String,
    pub linkage: super::MirLinkage,
    pub signature: MirErasedSignature,
    pub body: Option<MirErasedBody>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirErasedBody {
    /// Parameters occupy the first signature.params.len() locals.
    pub locals: Vec<MirErasedType>,
    /// Source-authorized mutable bindings; a dictionary does not grant write access.
    pub mutable_locals: BTreeSet<MirErasedLocal>,
    pub statements: Vec<MirErasedAssignment>,
    pub result: MirErasedOperand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirTypeDescriptor {
    pub ty: TypeId,
    pub drop: MirPayloadDropPlan,
    pub fields: Vec<TypeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MirDictionaryId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirErasedCall {
    pub target: MirErasedCallTarget,
    pub type_args: Vec<TypeId>,
    pub dictionaries: Vec<MirDictionaryId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirErasedCallTarget {
    Direct(MirErasedKey),
    Virtual(super::MirVirtualTarget),
}

impl MirErasedCallTarget {
    pub fn signature<'a>(
        &self,
        contract: &'a super::MirBackendContract,
    ) -> Result<&'a MirErasedSignature, MirErasedError> {
        match self {
            Self::Direct(key) => contract
                .erased
                .functions
                .get(key)
                .map(|entry| &entry.signature)
                .ok_or(MirErasedError::MissingCallable),
            Self::Virtual(target) => contract
                .object_schemas
                .get(&target.object)
                .and_then(|schema| schema.erased_slots.get(target.slot as usize))
                .map(|slot| &slot.signature)
                .ok_or(MirErasedError::MissingCallable),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MirErasedInvocationKey {
    pub object: TypeId,
    pub slot: u32,
    pub type_args: Vec<TypeId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirErasedObjectSlot {
    pub trait_id: DefId,
    pub member_id: DefId,
    pub trait_args: Vec<TypeId>,
    pub receiver: ReceiverMode,
    pub signature: MirErasedSignature,
    pub result: super::MirObjectResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirConcreteDictionary {
    pub subject: TypeId,
    pub trait_id: DefId,
    pub trait_args: Vec<TypeId>,
    pub members: Vec<MirConcreteDictionaryMember>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirConcreteDictionaryMember {
    pub member_id: DefId,
    pub receiver: ReceiverMode,
    pub target: MirCallableKey,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MirErasedContract {
    pub functions: BTreeMap<MirErasedKey, MirErasedFunction>,
    pub descriptors: BTreeMap<TypeId, MirTypeDescriptor>,
    pub dictionaries: BTreeMap<MirDictionaryId, MirConcreteDictionary>,
    pub constructor_kinds: BTreeMap<DefId, Kind>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirErasedError {
    UnboundParameter,
    KindMismatch,
    MissingLayout,
    MissingCallable,
    InvalidDictionary,
    InvalidLocal,
    InvalidType,
    UninitializedOrMoved,
    CopyWithoutProof,
    ProjectedMoveRequiresDropProof,
    BorrowEscape,
    BorrowConflict,
    UnsupportedOperation,
    SelectorUsedAsValue,
}

pub fn validate_erased_contract(
    program: &super::MirProgram,
) -> Vec<(Option<MirErasedKey>, MirErasedError)> {
    let mut errors = Vec::new();
    let contract = &program.backend_contract;
    let tc = &program.type_context;
    for (id, descriptor) in &contract.erased.descriptors {
        let expected_fields = match &descriptor.drop.children {
            super::MirPayloadDropChildren::Fields(fields) => {
                fields.iter().map(|p| p.ty).collect::<Vec<_>>()
            }
            _ => Vec::new(),
        };
        if *id != descriptor.ty
            || descriptor.fields != expected_fields
            || super::payload_drop_plan(contract, &mut tc.clone(), *id)
                .map_or(true, |plan| plan != descriptor.drop)
        {
            errors.push((None, MirErasedError::MissingLayout));
        }
    }
    for dictionary in contract.erased.dictionaries.values() {
        if !tc.type_id_tree_is_valid(dictionary.subject)
            || dictionary
                .trait_args
                .iter()
                .any(|id| !tc.type_id_tree_is_valid(*id))
            || !contract
                .erased
                .descriptors
                .contains_key(&dictionary.subject)
        {
            errors.push((None, MirErasedError::InvalidDictionary));
            continue;
        }
        let subject = tc.type_for(dictionary.subject);
        let mut members = BTreeSet::new();
        for member in &dictionary.members {
            if let MirCallableKey::ObjectAdapter(super::MirObjectAdapterKey::NativeCallable {
                concrete,
                kind,
            }) = &member.target
            {
                let native_valid = contract.callable(&member.target).is_some_and(|callee| {
                    let crate::types::Type::Function { params, ret, callable_kind, safety, .. } = &subject else { return false };
                    let packed = match params.as_slice() { [] => crate::types::Type::Unit, [param] => param.clone(), _ => crate::types::Type::Tuple(params.clone()) };
                    let receiver = match kind { crate::types::CallableKind::Fn => ReceiverMode::Shared, crate::types::CallableKind::FnMut => ReceiverMode::Mut, crate::types::CallableKind::FnOnce => ReceiverMode::Move };
                    *concrete == dictionary.subject && *callable_kind <= *kind && safety.permits_call_without_unsafe() && member.receiver == receiver
                        && dictionary.trait_args.iter().map(|id| tc.type_for(*id)).collect::<Vec<_>>() == vec![packed.clone(), *ret.clone()]
                        && callee.signature.params.len() == 2 && tc.type_id_tree_is_valid(callee.signature.params[1].semantic_ty) && tc.type_for(callee.signature.params[1].semantic_ty) == packed
                        && tc.type_id_tree_is_valid(callee.signature.ret.semantic_ty) && tc.type_for(callee.signature.ret.semantic_ty) == **ret
                        && matches!(&callee.kind, super::MirCallableKind::ObjectAdapter(super::MirObjectAdapter::NativeCallable { concrete: source, kind: mode, trait_id, member_id, .. }) if source == concrete && mode == kind && *trait_id == dictionary.trait_id && *member_id == member.member_id)
                });
                if !native_valid {
                    errors.push((None, MirErasedError::InvalidDictionary));
                }
            }
            let valid = contract.callable(&member.target).is_some_and(|callee| {
                let expected = match member.receiver {
                    ReceiverMode::Shared | ReceiverMode::Mut => crate::types::Type::Reference {
                        inner: Box::new(subject.clone()),
                        mutable: member.receiver == ReceiverMode::Mut,
                    },
                    ReceiverMode::Move
                        if matches!(callee.kind, super::MirCallableKind::ObjectAdapter(_)) =>
                    {
                        crate::types::Type::Pointer(Box::new(subject.clone()))
                    }
                    ReceiverMode::Move => subject.clone(),
                };
                callee
                    .signature
                    .params
                    .iter()
                    .all(|p| tc.type_id_tree_is_valid(p.semantic_ty))
                    && tc.type_id_tree_is_valid(callee.signature.ret.semantic_ty)
                    && tc.type_id_tree_is_valid(callee.signature.ret.abi_ty)
                    && callee
                        .signature
                        .params
                        .first()
                        .is_some_and(|p| tc.type_for(p.semantic_ty) == expected)
                    && callee.signature.ret.abi_ty == callee.signature.ret.semantic_ty
                    && matches!(
                        callee.kind,
                        super::MirCallableKind::LocalBody { .. }
                            | super::MirCallableKind::ObjectProvided
                            | super::MirCallableKind::ObjectAdapter(_)
                    )
            });
            if !members.insert(member.member_id) || !valid {
                errors.push((None, MirErasedError::InvalidDictionary));
            }
        }
    }
    if !errors.is_empty() {
        return errors;
    }
    for (key, function) in &contract.erased.functions {
        if key != &function.key {
            errors.push((Some(key.clone()), MirErasedError::MissingCallable));
        }
        if let Err(error) = validate_erased_function(function, contract, tc) {
            errors.push((Some(key.clone()), error));
        }
    }
    for function in program.functions.values() {
        for block in &function.basic_blocks {
            let check_value = |value: &super::Operand| {
                matches!(
                    value,
                    super::Operand::Constant(super::Constant::ErasedCall(_))
                )
            };
            for statement in &block.statements {
                let invalid = match &statement.kind {
                    super::StatementKind::Assign(
                        _,
                        super::Rvalue::Use(op)
                        | super::Rvalue::Object(op, _)
                        | super::Rvalue::Cast(op, _)
                        | super::Rvalue::UnaryOp(_, op),
                    ) => check_value(op),
                    super::StatementKind::Assign(_, super::Rvalue::BinaryOp(_, left, right)) => {
                        check_value(left) || check_value(right)
                    }
                    super::StatementKind::Assign(_, super::Rvalue::Aggregate(_, values)) => {
                        values.iter().any(check_value)
                    }
                    super::StatementKind::Assert(assertion) => {
                        assertion.operands.iter().any(check_value)
                    }
                    _ => false,
                };
                if invalid {
                    errors.push((None, MirErasedError::SelectorUsedAsValue));
                }
            }
            match &block.terminator {
                Some(super::Terminator::Call {
                    func,
                    args,
                    destination,
                    ..
                }) => {
                    if args.iter().any(check_value) {
                        errors.push((None, MirErasedError::SelectorUsedAsValue));
                    }
                    if let super::Operand::Constant(super::Constant::ErasedCall(call)) = func {
                        if let Err(error) =
                            validate_erased_call(program, function, call, args, destination)
                        {
                            errors.push((
                                match &call.target {
                                    MirErasedCallTarget::Direct(key) => Some(key.clone()),
                                    _ => None,
                                },
                                error,
                            ));
                        }
                    }
                }
                Some(
                    super::Terminator::SwitchInt { discr, .. }
                    | super::Terminator::SwitchIntWithOrigin { discr, .. },
                ) if check_value(discr) => errors.push((None, MirErasedError::SelectorUsedAsValue)),
                _ => {}
            }
        }
    }
    errors
}

pub fn erased_entry_matches_object_slot(
    function: &MirErasedFunction,
    slot: &MirErasedObjectSlot,
    concrete: TypeId,
    tc: &crate::type_context::TypeContext,
) -> bool {
    let signature = &slot.signature;
    if function.signature.parameters != signature.parameters
        || function.signature.layouts.len() != signature.layouts.len()
        || function.signature.require_borrow_free != signature.require_borrow_free
    {
        return false;
    }
    let mut bindings = (0..signature.parameters.len())
        .map(|i| MirErasedType::Parameter(i as u32))
        .collect::<Vec<_>>();
    if bindings.is_empty() {
        return false;
    }
    bindings[0] = MirErasedType::Concrete(concrete);
    let specialize = |ty: &MirErasedType| -> Result<MirErasedType, MirErasedError> {
        let ty = ty.substitute(&bindings)?;
        if let Ok(concrete) = ty.instantiate(&[], tc) {
            return tc
                .id_for_type(&concrete)
                .map(MirErasedType::Concrete)
                .ok_or(MirErasedError::MissingLayout);
        }
        Ok(ty)
    };
    signature
        .params
        .iter()
        .map(specialize)
        .collect::<Result<Vec<_>, _>>()
        .is_ok_and(|params| params == function.signature.params)
        && specialize(&signature.ret).is_ok_and(|ret| ret == function.signature.ret)
        && signature
            .layouts
            .iter()
            .map(|ty| {
                if *ty == MirErasedType::Parameter(0) {
                    Ok(ty.clone())
                } else {
                    specialize(ty)
                }
            })
            .collect::<Result<Vec<_>, _>>()
            .is_ok_and(|layouts| layouts == function.signature.layouts)
        && signature
            .dictionaries
            .iter()
            .map(|dictionary| dictionary.substitute(&bindings))
            .collect::<Result<Vec<_>, _>>()
            .is_ok_and(|dictionaries| dictionaries == function.signature.dictionaries)
}

fn validate_erased_call(
    program: &super::MirProgram,
    function: &super::MirFunction,
    call: &MirErasedCall,
    args: &[super::Operand],
    destination: &super::Place,
) -> Result<(), MirErasedError> {
    validate_erased_call_inner(program, function, call, args, destination, None)
}

pub(crate) fn validate_owned_erased_call(
    program: &super::MirProgram,
    function: &super::MirFunction,
    key: &super::MirOwnedObjectKey,
    args: &[super::Operand],
    destination: &super::Place,
) -> Result<(), MirErasedError> {
    let plan = program
        .backend_contract
        .owned_object_calls
        .get(key)
        .ok_or(MirErasedError::MissingCallable)?;
    if !super::object::owned_object_plan_is_valid(program, key, plan) {
        return Err(MirErasedError::InvalidType);
    }
    let call = key
        .erased_evidence(plan)
        .ok_or(MirErasedError::InvalidType)?;
    validate_erased_call_inner(program, function, &call, args, destination, Some(key))
}

pub(crate) fn validate_erased_environment(
    contract: &super::MirBackendContract,
    tc: &crate::type_context::TypeContext,
    call: &MirErasedCall,
) -> Result<Vec<Option<crate::types::Type>>, MirErasedError> {
    let signature = call.target.signature(contract)?;
    let witness = matches!(call.target, MirErasedCallTarget::Virtual(_));
    let offset = usize::from(witness);
    if call.type_args.len() + offset != signature.parameters.len()
        || call.dictionaries.len() != signature.dictionaries.len()
    {
        return Err(MirErasedError::InvalidType);
    }
    let mut bindings = if witness { vec![None] } else { Vec::new() };
    for (arg, parameter) in call
        .type_args
        .iter()
        .zip(signature.parameters.iter().skip(offset))
    {
        if !tc.type_id_tree_is_valid(*arg) || tc.kind(*arg) != &parameter.kind {
            return Err(MirErasedError::KindMismatch);
        }
        bindings.push(Some(tc.type_for(*arg)));
    }
    for requirement in &signature.require_borrow_free {
        let ty = requirement.instantiate_optional(&bindings, tc)?;
        let id = tc.id_for_type(&ty).ok_or(MirErasedError::MissingLayout)?;
        if super::object::virtual_argument_requires_summary(contract, tc, id) {
            return Err(MirErasedError::UnsupportedOperation);
        }
    }
    for ty in &signature.layouts {
        if witness && *ty == MirErasedType::Parameter(0) {
            continue;
        }
        let id = tc
            .id_for_type(&ty.instantiate_optional(&bindings, tc)?)
            .ok_or(MirErasedError::MissingLayout)?;
        if !contract.erased.descriptors.contains_key(&id) {
            return Err(MirErasedError::MissingLayout);
        }
    }
    for (id, required) in call.dictionaries.iter().zip(&signature.dictionaries) {
        let dictionary = contract
            .erased
            .dictionaries
            .get(id)
            .ok_or(MirErasedError::InvalidDictionary)?;
        if !tc.type_id_tree_is_valid(dictionary.subject)
            || dictionary
                .trait_args
                .iter()
                .any(|id| !tc.type_id_tree_is_valid(*id))
        {
            return Err(MirErasedError::InvalidDictionary);
        }
        if dictionary.trait_id != required.trait_id
            || tc.type_for(dictionary.subject)
                != required.subject.instantiate_optional(&bindings, tc)?
            || dictionary.members.len() != required.members.len()
        {
            return Err(MirErasedError::InvalidDictionary);
        }
        let trait_args = required
            .trait_args
            .iter()
            .map(|ty| ty.instantiate_optional(&bindings, tc))
            .collect::<Result<Vec<_>, _>>()?;
        if dictionary
            .trait_args
            .iter()
            .map(|id| tc.type_for(*id))
            .collect::<Vec<_>>()
            != trait_args
        {
            return Err(MirErasedError::InvalidDictionary);
        }
        for (entry, expected) in dictionary.members.iter().zip(&required.members) {
            let callee = contract
                .callable(&entry.target)
                .ok_or(MirErasedError::MissingCallable)?;
            if !tc.type_id_tree_is_valid(callee.signature.ret.semantic_ty)
                || callee
                    .signature
                    .params
                    .iter()
                    .any(|p| !tc.type_id_tree_is_valid(p.semantic_ty))
            {
                return Err(MirErasedError::InvalidDictionary);
            }
            if entry.member_id != expected.member_id
                || entry.receiver != expected.receiver
                || callee.signature.params.len() != expected.params.len()
                || tc.type_for(callee.signature.ret.semantic_ty)
                    != expected.ret.instantiate_optional(&bindings, tc)?
            {
                return Err(MirErasedError::InvalidDictionary);
            }
            for (actual, expected) in callee
                .signature
                .params
                .iter()
                .skip(1)
                .zip(expected.params.iter().skip(1))
            {
                if tc.type_for(actual.semantic_ty)
                    != expected.instantiate_optional(&bindings, tc)?
                {
                    return Err(MirErasedError::InvalidDictionary);
                }
            }
        }
    }
    Ok(bindings)
}

fn validate_erased_call_inner(
    program: &super::MirProgram,
    function: &super::MirFunction,
    call: &MirErasedCall,
    args: &[super::Operand],
    destination: &super::Place,
    owned: Option<&super::MirOwnedObjectKey>,
) -> Result<(), MirErasedError> {
    let contract = &program.backend_contract;
    let tc = &program.type_context;
    let signature = call.target.signature(contract)?;
    let bindings = validate_erased_environment(contract, tc, call)?;
    let witness = matches!(call.target, MirErasedCallTarget::Virtual(_));
    if args.len() != signature.params.len() {
        return Err(MirErasedError::InvalidType);
    }
    for (index, (arg, expected)) in args.iter().zip(&signature.params).enumerate() {
        let (super::Operand::Copy(place) | super::Operand::Move(place)) = arg else {
            return Err(MirErasedError::InvalidLocal);
        };
        let actual = contract
            .place_type_id(tc, function, place)
            .ok_or(MirErasedError::InvalidType)?;
        if index == 0 && witness {
            let MirErasedCallTarget::Virtual(target) = &call.target else {
                unreachable!()
            };
            let slot = &contract.object_schemas[&target.object].erased_slots[target.slot as usize];
            if let Some(key) = owned {
                if key.object != target.object
                    || slot.receiver != ReceiverMode::Move
                    || slot.result != super::MirObjectResult::Direct
                    || expected != &MirErasedType::Parameter(0)
                    || actual != key.owner
                    || !matches!(arg, super::Operand::Move(_))
                {
                    return Err(MirErasedError::InvalidType);
                }
            } else if slot.receiver == ReceiverMode::Move
                || !matches!(tc.type_for(actual), crate::types::Type::Reference { inner, mutable } if tc.id_for_type(&inner) == Some(target.object) && mutable == (slot.receiver == ReceiverMode::Mut))
            {
                return Err(MirErasedError::InvalidType);
            }
        } else if tc.type_for(actual) != expected.instantiate_optional(&bindings, tc)? {
            return Err(MirErasedError::InvalidType);
        }
        if matches!(arg, super::Operand::Copy(_)) && !tc.type_for(actual).is_copy() {
            return Err(MirErasedError::CopyWithoutProof);
        }
    }
    let result = contract
        .place_type_id(tc, function, destination)
        .ok_or(MirErasedError::InvalidType)?;
    let result_ty = if let MirErasedCallTarget::Virtual(target) = &call.target {
        let slot = &contract.object_schemas[&target.object].erased_slots[target.slot as usize];
        match slot.result {
            super::MirObjectResult::BorrowedSelf { mutable } => crate::types::Type::Reference {
                inner: Box::new(tc.type_for(target.object)),
                mutable,
            },
            super::MirObjectResult::Direct => signature.ret.instantiate_optional(&bindings, tc)?,
        }
    } else {
        signature.ret.instantiate_optional(&bindings, tc)?
    };
    if tc.type_for(result) != result_ty {
        return Err(MirErasedError::InvalidType);
    }
    Ok(())
}

impl MirErasedType {
    pub fn from_type(
        ty: &crate::types::Type,
        parameters: &BTreeMap<GenericParamId, u32>,
        witness: Option<u32>,
        tc: &mut crate::type_context::TypeContext,
        env: &crate::type_services::normalize::TypeNormalizationEnv,
    ) -> Result<Self, MirErasedError> {
        use crate::types::Type;
        if !crate::type_services::visit::type_any(ty, |ty| matches!(ty, Type::Generic(_)))
            && !crate::type_services::substitution::has_free_object_self(ty)
        {
            return Ok(Self::Concrete(
                tc.intern_normalized_type(ty, env)
                    .map_err(|_| MirErasedError::KindMismatch)?,
            ));
        }
        Ok(match ty {
            Type::Generic(id) => {
                Self::Parameter(*parameters.get(id).ok_or(MirErasedError::UnboundParameter)?)
            }
            Type::ObjectSelf { depth: 0 } => {
                Self::Parameter(witness.ok_or(MirErasedError::UnboundParameter)?)
            }
            Type::Reference { inner, mutable } => Self::Reference {
                inner: Box::new(Self::from_type(inner, parameters, witness, tc, env)?),
                mutable: *mutable,
            },
            Type::Pointer(inner) => Self::Pointer(Box::new(Self::from_type(
                inner, parameters, witness, tc, env,
            )?)),
            Type::Tuple(fields) => Self::Tuple(
                fields
                    .iter()
                    .map(|ty| Self::from_type(ty, parameters, witness, tc, env))
                    .collect::<Result<_, _>>()?,
            ),
            Type::Array(element, len) => Self::Array {
                element: Box::new(Self::from_type(element, parameters, witness, tc, env)?),
                len: *len,
            },
            Type::Struct { id, args } | Type::Enum { id, args } => Self::Nominal {
                id: *id,
                flavor: if matches!(ty, Type::Struct { .. }) {
                    NominalTypeKind::Struct
                } else {
                    NominalTypeKind::Enum
                },
                args: args
                    .iter()
                    .map(|ty| Self::from_type(ty, parameters, witness, tc, env))
                    .collect::<Result<_, _>>()?,
            },
            Type::Apply { constructor, args } => Self::Apply {
                constructor: Box::new(Self::from_type(constructor, parameters, witness, tc, env)?),
                args: args
                    .iter()
                    .map(|ty| Self::from_type(ty, parameters, witness, tc, env))
                    .collect::<Result<_, _>>()?,
            },
            _ => return Err(MirErasedError::UnsupportedOperation),
        })
    }

    pub fn contains_parameter(&self, index: u32) -> bool {
        match self {
            Self::Parameter(parameter) => *parameter == index,
            Self::Concrete(_) => false,
            Self::Reference { inner, .. } | Self::Pointer(inner) => inner.contains_parameter(index),
            Self::Tuple(fields) | Self::Nominal { args: fields, .. } => {
                fields.iter().any(|ty| ty.contains_parameter(index))
            }
            Self::Array { element, .. } => element.contains_parameter(index),
            Self::Apply { constructor, args } => {
                constructor.contains_parameter(index)
                    || args.iter().any(|ty| ty.contains_parameter(index))
            }
        }
    }

    pub fn collect_layouts(&self, layouts: &mut Vec<Self>, witness: bool) {
        let receiver_reference = witness
            && matches!(self, Self::Reference { inner, .. } if **inner == Self::Parameter(0));
        if !matches!(self, Self::Concrete(_)) && !receiver_reference && !layouts.contains(self) {
            layouts.push(self.clone());
        }
        match self {
            Self::Reference { inner, .. } | Self::Pointer(inner) => {
                inner.collect_layouts(layouts, witness)
            }
            Self::Tuple(fields) | Self::Nominal { args: fields, .. } => {
                for field in fields {
                    field.collect_layouts(layouts, witness);
                }
            }
            Self::Array { element, .. } => element.collect_layouts(layouts, witness),
            Self::Apply { args, .. } => {
                for arg in args {
                    arg.collect_layouts(layouts, witness);
                }
            }
            _ => {}
        }
    }

    pub fn instantiate_optional(
        &self,
        arguments: &[Option<crate::types::Type>],
        tc: &crate::type_context::TypeContext,
    ) -> Result<crate::types::Type, MirErasedError> {
        self.instantiate_using(
            &|index| arguments.get(index as usize).cloned().flatten(),
            tc,
        )
    }
    pub fn kind(
        &self,
        signature: &MirErasedSignature,
        contract: &super::MirBackendContract,
        tc: &crate::type_context::TypeContext,
    ) -> Result<Kind, MirErasedError> {
        fn apply(
            mut kind: Kind,
            args: &[MirErasedType],
            signature: &MirErasedSignature,
            contract: &super::MirBackendContract,
            tc: &crate::type_context::TypeContext,
        ) -> Result<Kind, MirErasedError> {
            for arg in args {
                let Kind::Arrow(input, output) = kind else {
                    return Err(MirErasedError::KindMismatch);
                };
                if *input != arg.kind(signature, contract, tc)? {
                    return Err(MirErasedError::KindMismatch);
                }
                kind = *output;
            }
            Ok(kind)
        }
        match self {
            Self::Concrete(id) => tc
                .type_id_tree_is_valid(*id)
                .then(|| tc.kind(*id).clone())
                .ok_or(MirErasedError::InvalidType),
            Self::Parameter(index) => signature
                .parameters
                .get(*index as usize)
                .map(|p| p.kind.clone())
                .ok_or(MirErasedError::UnboundParameter),
            Self::Reference { inner, .. } | Self::Pointer(inner) => {
                if inner.kind(signature, contract, tc)? == Kind::Type {
                    Ok(Kind::Type)
                } else {
                    Err(MirErasedError::KindMismatch)
                }
            }
            Self::Tuple(fields) => {
                for field in fields {
                    if field.kind(signature, contract, tc)? != Kind::Type {
                        return Err(MirErasedError::KindMismatch);
                    }
                }
                Ok(Kind::Type)
            }
            Self::Array { element, .. } => {
                if element.kind(signature, contract, tc)? == Kind::Type {
                    Ok(Kind::Type)
                } else {
                    Err(MirErasedError::KindMismatch)
                }
            }
            Self::Nominal { id, args, .. } => apply(
                contract
                    .erased
                    .constructor_kinds
                    .get(id)
                    .cloned()
                    .ok_or(MirErasedError::MissingLayout)?,
                args,
                signature,
                contract,
                tc,
            ),
            Self::Apply { constructor, args } => apply(
                constructor.kind(signature, contract, tc)?,
                args,
                signature,
                contract,
                tc,
            ),
        }
    }

    pub fn instantiate(
        &self,
        arguments: &[crate::types::Type],
        tc: &crate::type_context::TypeContext,
    ) -> Result<crate::types::Type, MirErasedError> {
        self.instantiate_using(&|index| arguments.get(index as usize).cloned(), tc)
    }
    fn instantiate_using(
        &self,
        argument: &impl Fn(u32) -> Option<crate::types::Type>,
        tc: &crate::type_context::TypeContext,
    ) -> Result<crate::types::Type, MirErasedError> {
        use crate::types::Type;
        Ok(match self {
            Self::Concrete(id) => {
                if !tc.type_id_tree_is_valid(*id) {
                    return Err(MirErasedError::InvalidType);
                }
                tc.type_for(*id)
            }
            Self::Parameter(index) => argument(*index).ok_or(MirErasedError::UnboundParameter)?,
            Self::Reference { inner, mutable } => Type::Reference {
                inner: Box::new(inner.instantiate_using(argument, tc)?),
                mutable: *mutable,
            },
            Self::Pointer(inner) => Type::Pointer(Box::new(inner.instantiate_using(argument, tc)?)),
            Self::Tuple(fields) => Type::Tuple(
                fields
                    .iter()
                    .map(|ty| ty.instantiate_using(argument, tc))
                    .collect::<Result<_, _>>()?,
            ),
            Self::Array { element, len } => {
                Type::Array(Box::new(element.instantiate_using(argument, tc)?), *len)
            }
            Self::Nominal { id, flavor, args } => {
                let args = args
                    .iter()
                    .map(|ty| ty.instantiate_using(argument, tc))
                    .collect::<Result<_, _>>()?;
                match flavor {
                    NominalTypeKind::Struct => Type::Struct { id: *id, args },
                    NominalTypeKind::Enum => Type::Enum { id: *id, args },
                    NominalTypeKind::Alias => return Err(MirErasedError::UnsupportedOperation),
                }
            }
            Self::Apply { constructor, args } => {
                let args = args
                    .iter()
                    .map(|ty| ty.instantiate_using(argument, tc))
                    .collect::<Result<Vec<_>, _>>()?;
                match constructor.instantiate_using(argument, tc)? {
                    Type::Constructor {
                        id,
                        flavor: NominalTypeKind::Struct,
                    } => Type::Struct { id, args },
                    Type::Constructor {
                        id,
                        flavor: NominalTypeKind::Enum,
                    } => Type::Enum { id, args },
                    Type::Lambda { params, body } if params.len() == args.len() => {
                        crate::type_services::substitution::substitute_lambda_group(
                            &body,
                            &args,
                            args.len(),
                            true,
                        )
                        .map_err(|_| MirErasedError::KindMismatch)?
                    }
                    _ => return Err(MirErasedError::UnsupportedOperation),
                }
            }
        })
    }

    pub fn substitute(&self, bindings: &[Self]) -> Result<Self, MirErasedError> {
        Ok(match self {
            Self::Parameter(index) => bindings
                .get(*index as usize)
                .cloned()
                .ok_or(MirErasedError::UnboundParameter)?,
            Self::Concrete(id) => Self::Concrete(*id),
            Self::Reference { inner, mutable } => Self::Reference {
                inner: Box::new(inner.substitute(bindings)?),
                mutable: *mutable,
            },
            Self::Pointer(inner) => Self::Pointer(Box::new(inner.substitute(bindings)?)),
            Self::Tuple(fields) => Self::Tuple(
                fields
                    .iter()
                    .map(|t| t.substitute(bindings))
                    .collect::<Result<_, _>>()?,
            ),
            Self::Array { element, len } => Self::Array {
                element: Box::new(element.substitute(bindings)?),
                len: *len,
            },
            Self::Nominal { id, flavor, args } => Self::Nominal {
                id: *id,
                flavor: *flavor,
                args: args
                    .iter()
                    .map(|t| t.substitute(bindings))
                    .collect::<Result<_, _>>()?,
            },
            Self::Apply { constructor, args } => Self::Apply {
                constructor: Box::new(constructor.substitute(bindings)?),
                args: args
                    .iter()
                    .map(|t| t.substitute(bindings))
                    .collect::<Result<_, _>>()?,
            },
        })
    }
}

pub fn erased_place_type(
    function: &MirErasedFunction,
    place: &MirErasedPlace,
    contract: &super::MirBackendContract,
    tc: &crate::type_context::TypeContext,
) -> Result<MirErasedType, MirErasedError> {
    let mut ty = function
        .body
        .as_ref()
        .ok_or(MirErasedError::MissingCallable)?
        .locals
        .get(place.local.0 as usize)
        .cloned()
        .ok_or(MirErasedError::InvalidLocal)?;
    for projection in &place.projection {
        ty = match (projection, &ty) {
            (
                MirErasedProjection::Deref,
                MirErasedType::Reference { inner, .. } | MirErasedType::Pointer(inner),
            ) => *inner.clone(),
            (MirErasedProjection::Field(index), MirErasedType::Tuple(fields)) => fields
                .get(*index as usize)
                .cloned()
                .ok_or(MirErasedError::InvalidType)?,
            (MirErasedProjection::Field(index), MirErasedType::Concrete(id)) => {
                let descriptor = contract
                    .erased
                    .descriptors
                    .get(id)
                    .ok_or(MirErasedError::MissingLayout)?;
                MirErasedType::Concrete(
                    *descriptor
                        .fields
                        .get(*index as usize)
                        .ok_or(MirErasedError::InvalidType)?,
                )
            }
            (MirErasedProjection::Deref, MirErasedType::Concrete(id)) => match tc.type_for(*id) {
                crate::types::Type::Reference { inner, .. }
                | crate::types::Type::Pointer(inner) => MirErasedType::Concrete(
                    tc.id_for_type(&inner)
                        .ok_or(MirErasedError::MissingLayout)?,
                ),
                _ => return Err(MirErasedError::InvalidType),
            },
            _ => return Err(MirErasedError::UnsupportedOperation),
        };
    }
    Ok(ty)
}

fn copy_proven(ty: &MirErasedType, tc: &crate::type_context::TypeContext) -> bool {
    match ty {
        MirErasedType::Concrete(id) => tc.type_id_tree_is_valid(*id) && tc.type_for(*id).is_copy(),
        MirErasedType::Reference { mutable: false, .. } | MirErasedType::Pointer(_) => true,
        MirErasedType::Tuple(fields) => fields.iter().all(|field| copy_proven(field, tc)),
        _ => false,
    }
}

fn leaves(
    ty: &MirErasedType,
    contract: &super::MirBackendContract,
    tc: &crate::type_context::TypeContext,
) -> Result<(), MirErasedError> {
    use crate::types::Type;
    match ty {
        MirErasedType::Concrete(id) => {
            if !tc.type_id_tree_is_valid(*id) {
                return Err(MirErasedError::InvalidType);
            }
            let ty = tc.type_for(*id);
            if crate::type_services::visit::type_any(&ty, |ty| {
                matches!(ty, Type::Generic(_) | Type::TypeVar(_) | Type::Error)
            }) || crate::type_services::substitution::has_free_object_self(&ty)
            {
                return Err(MirErasedError::InvalidType);
            }
            if tc.kind(*id) == &Kind::Type
                && !super::object_runtime_type_is_valid(contract, tc, *id)
            {
                return Err(MirErasedError::InvalidType);
            }
        }
        MirErasedType::Parameter(_) => {}
        MirErasedType::Reference { inner, .. } | MirErasedType::Pointer(inner) => {
            leaves(inner, contract, tc)?
        }
        MirErasedType::Tuple(fields) | MirErasedType::Nominal { args: fields, .. } => {
            for field in fields {
                leaves(field, contract, tc)?;
            }
        }
        MirErasedType::Array { element, .. } => leaves(element, contract, tc)?,
        MirErasedType::Apply { constructor, args } => {
            leaves(constructor, contract, tc)?;
            for arg in args {
                leaves(arg, contract, tc)?;
            }
        }
    }
    Ok(())
}

pub fn validate_erased_signature(
    signature: &MirErasedSignature,
    contract: &super::MirBackendContract,
    tc: &crate::type_context::TypeContext,
) -> Result<(), MirErasedError> {
    for (index, parameter) in signature.parameters.iter().enumerate() {
        if parameter.kind == Kind::Type
            && !signature
                .layouts
                .contains(&MirErasedType::Parameter(index as u32))
        {
            return Err(MirErasedError::MissingLayout);
        }
    }
    let mut dictionaries = BTreeSet::new();
    for dictionary in &signature.dictionaries {
        if !dictionaries.insert((
            &dictionary.subject,
            dictionary.trait_id,
            &dictionary.trait_args,
        )) {
            return Err(MirErasedError::InvalidDictionary);
        }
        leaves(&dictionary.subject, contract, tc)?;
        if dictionary.subject.kind(signature, contract, tc)? != Kind::Type {
            return Err(MirErasedError::InvalidDictionary);
        }
        for argument in &dictionary.trait_args {
            leaves(argument, contract, tc)?;
            argument.kind(signature, contract, tc)?;
        }
        let mut members = BTreeSet::new();
        for member in &dictionary.members {
            if !members.insert(member.member_id)
                || member.params.first() != Some(&dictionary.subject)
            {
                return Err(MirErasedError::InvalidDictionary);
            }
            for ty in member.params.iter().chain(std::iter::once(&member.ret)) {
                leaves(ty, contract, tc)?;
                if ty.kind(signature, contract, tc)? != Kind::Type {
                    return Err(MirErasedError::KindMismatch);
                }
            }
        }
    }
    for ty in signature
        .params
        .iter()
        .chain(std::iter::once(&signature.ret))
        .chain(&signature.layouts)
        .chain(&signature.require_borrow_free)
    {
        leaves(ty, contract, tc)?;
        if ty.kind(signature, contract, tc)? != Kind::Type {
            return Err(MirErasedError::KindMismatch);
        }
        if let MirErasedType::Concrete(id) = ty {
            if !super::object_runtime_type_is_valid(contract, tc, *id) {
                return Err(MirErasedError::InvalidType);
            }
        }
    }
    Ok(())
}

/// Validate the bound environment and linear value transfers before lowering.
pub fn validate_erased_function(
    function: &MirErasedFunction,
    contract: &super::MirBackendContract,
    tc: &crate::type_context::TypeContext,
) -> Result<(), MirErasedError> {
    use crate::types::Type;
    let signature = &function.signature;
    validate_erased_signature(signature, contract, tc)?;
    let Some(body) = &function.body else {
        return Ok(());
    };
    if body
        .mutable_locals
        .iter()
        .any(|local| local.0 as usize >= body.locals.len())
    {
        return Err(MirErasedError::InvalidLocal);
    }
    if body.locals.get(..signature.params.len()) != Some(signature.params.as_slice()) {
        return Err(MirErasedError::InvalidLocal);
    }
    for ty in &body.locals {
        leaves(ty, contract, tc)?;
        if ty.kind(signature, contract, tc)? != Kind::Type {
            return Err(MirErasedError::KindMismatch);
        }
        if !signature.layouts.contains(ty)
            && !matches!(ty, MirErasedType::Concrete(id) if contract.erased.descriptors.contains_key(id))
        {
            return Err(MirErasedError::MissingLayout);
        }
    }
    let mut initialized = vec![false; body.locals.len()];
    initialized[..signature.params.len()].fill(true);
    let mut origins = vec![BTreeSet::<usize>::new(); body.locals.len()];
    for (index, origin) in origins.iter_mut().take(signature.params.len()).enumerate() {
        origin.insert(index);
    }
    let mut local_borrows = vec![BTreeSet::<u32>::new(); body.locals.len()];
    fn operand(
        op: &MirErasedOperand,
        function: &MirErasedFunction,
        contract: &super::MirBackendContract,
        tc: &crate::type_context::TypeContext,
        initialized: &mut [bool],
        borrows: &[BTreeSet<u32>],
    ) -> Result<MirErasedType, MirErasedError> {
        let (place, moving) = match op {
            MirErasedOperand::Move(place) => (place, true),
            MirErasedOperand::Copy(place) => (place, false),
        };
        if !initialized
            .get(place.local.0 as usize)
            .copied()
            .unwrap_or(false)
        {
            return Err(MirErasedError::UninitializedOrMoved);
        }
        let ty = erased_place_type(function, place, contract, tc)?;
        if moving {
            if !place.projection.is_empty() {
                return Err(MirErasedError::ProjectedMoveRequiresDropProof);
            }
            if borrows
                .iter()
                .zip(initialized.iter())
                .any(|(borrow, live)| *live && borrow.contains(&place.local.0))
            {
                return Err(MirErasedError::BorrowConflict);
            }
            initialized[place.local.0 as usize] = false;
        } else if !copy_proven(&ty, tc) {
            return Err(MirErasedError::CopyWithoutProof);
        }
        Ok(ty)
    }
    for statement in &body.statements {
        let dest = statement.destination.0 as usize;
        let expected = body.locals.get(dest).ok_or(MirErasedError::InvalidLocal)?;
        if initialized[dest] {
            return Err(MirErasedError::InvalidLocal);
        }
        let mut result_origins = BTreeSet::new();
        let mut result_borrows = BTreeSet::new();
        let receiver_initialized = match &statement.value {
            MirErasedRvalue::DictionaryCall { receiver, .. } => initialized
                .get(receiver.local.0 as usize)
                .copied()
                .unwrap_or(false),
            _ => true,
        };
        let receiver_borrowed = match &statement.value {
            MirErasedRvalue::DictionaryCall { receiver, .. } => local_borrows
                .iter()
                .zip(&initialized)
                .any(|(borrow, live)| *live && borrow.contains(&receiver.local.0)),
            _ => false,
        };
        let mut use_operand = |op: &MirErasedOperand| {
            let place = match op {
                MirErasedOperand::Move(place) | MirErasedOperand::Copy(place) => place,
            };
            let ty = operand(op, function, contract, tc, &mut initialized, &local_borrows)?;
            result_origins.extend(origins[place.local.0 as usize].iter().copied());
            result_borrows.extend(local_borrows[place.local.0 as usize].iter().copied());
            Ok::<_, MirErasedError>(ty)
        };
        let actual = match &statement.value {
            MirErasedRvalue::Use(op) => use_operand(op)?,
            MirErasedRvalue::Literal(value) => MirErasedType::Concrete(
                tc.id_for_type(&match value {
                    MirErasedLiteral::Int(_) => Type::I64,
                    MirErasedLiteral::Bool(_) => Type::Bool,
                    MirErasedLiteral::Unit => Type::Unit,
                })
                .ok_or(MirErasedError::InvalidType)?,
            ),
            MirErasedRvalue::Aggregate(values) => MirErasedType::Tuple(
                values
                    .iter()
                    .map(&mut use_operand)
                    .collect::<Result<_, _>>()?,
            ),
            MirErasedRvalue::Borrow { place, mutable } => {
                if !initialized
                    .get(place.local.0 as usize)
                    .copied()
                    .unwrap_or(false)
                {
                    return Err(MirErasedError::UninitializedOrMoved);
                }
                if *mutable {
                    return Err(MirErasedError::UnsupportedOperation);
                }
                result_borrows.insert(place.local.0);
                MirErasedType::Reference {
                    inner: Box::new(erased_place_type(function, place, contract, tc)?),
                    mutable: false,
                }
            }
            MirErasedRvalue::Call {
                target,
                type_args,
                dictionaries,
                args,
            } => {
                let callee = contract
                    .erased
                    .functions
                    .get(target)
                    .ok_or(MirErasedError::MissingCallable)?;
                if type_args.len() != callee.signature.parameters.len()
                    || dictionaries.len() != callee.signature.dictionaries.len()
                    || args.len() != callee.signature.params.len()
                {
                    return Err(MirErasedError::InvalidType);
                }
                for (ty, parameter) in type_args.iter().zip(&callee.signature.parameters) {
                    if ty.kind(signature, contract, tc)? != parameter.kind {
                        return Err(MirErasedError::KindMismatch);
                    }
                }
                for requirement in &callee.signature.require_borrow_free {
                    if erased_type_may_borrow(
                        &requirement.substitute(type_args)?,
                        signature,
                        contract,
                        tc,
                    ) {
                        return Err(MirErasedError::UnsupportedOperation);
                    }
                }
                for (index, required) in dictionaries.iter().zip(&callee.signature.dictionaries) {
                    if signature.dictionaries.get(*index as usize)
                        != Some(&required.substitute(type_args)?)
                    {
                        return Err(MirErasedError::InvalidDictionary);
                    }
                }
                for layout in &callee.signature.layouts {
                    let layout = layout.substitute(type_args)?;
                    if !signature.layouts.contains(&layout)
                        && !matches!(&layout, MirErasedType::Concrete(id) if contract.erased.descriptors.contains_key(id))
                    {
                        return Err(MirErasedError::MissingLayout);
                    }
                }
                for (arg, expected) in args.iter().zip(&callee.signature.params) {
                    if use_operand(arg)? != expected.substitute(type_args)? {
                        return Err(MirErasedError::InvalidType);
                    }
                }
                callee.signature.ret.substitute(type_args)?
            }
            MirErasedRvalue::DictionaryCall {
                dictionary,
                member,
                receiver,
                args,
            } => {
                let dictionary = signature
                    .dictionaries
                    .get(*dictionary as usize)
                    .ok_or(MirErasedError::InvalidDictionary)?;
                let member = dictionary
                    .members
                    .get(*member as usize)
                    .ok_or(MirErasedError::InvalidDictionary)?;
                if !receiver_initialized {
                    return Err(MirErasedError::UninitializedOrMoved);
                }
                if erased_place_type(function, receiver, contract, tc)? != dictionary.subject {
                    return Err(MirErasedError::InvalidDictionary);
                }
                if member.receiver != ReceiverMode::Move && args.iter().any(|arg| matches!(arg, MirErasedOperand::Move(place) if place.local == receiver.local)) { return Err(MirErasedError::BorrowConflict); }
                if member.receiver == ReceiverMode::Mut {
                    let mut writable = body.mutable_locals.contains(&receiver.local);
                    let mut prefix = MirErasedPlace::local(receiver.local);
                    for projection in &receiver.projection {
                        if *projection == MirErasedProjection::Deref {
                            writable = match erased_place_type(function, &prefix, contract, tc)? {
                                MirErasedType::Reference { mutable, .. } => mutable,
                                MirErasedType::Concrete(id) => {
                                    matches!(tc.type_for(id), Type::Reference { mutable: true, .. })
                                }
                                _ => false,
                            };
                        }
                        prefix.projection.push(projection.clone());
                    }
                    if !writable {
                        return Err(MirErasedError::BorrowConflict);
                    }
                    if receiver_borrowed {
                        return Err(MirErasedError::BorrowConflict);
                    }
                }
                if member.receiver == ReceiverMode::Move {
                    use_operand(&MirErasedOperand::Move(receiver.clone()))?;
                }
                if args.len() + 1 != member.params.len() {
                    return Err(MirErasedError::InvalidDictionary);
                }
                for (arg, expected) in args.iter().zip(member.params.iter().skip(1)) {
                    if erased_type_may_borrow(expected, signature, contract, tc) {
                        return Err(MirErasedError::UnsupportedOperation);
                    }
                    if use_operand(arg)? != *expected {
                        return Err(MirErasedError::InvalidDictionary);
                    }
                }
                if member.receiver != ReceiverMode::Move {
                    result_origins.extend(origins[receiver.local.0 as usize].iter().copied());
                }
                if erased_type_may_borrow(&member.ret, signature, contract, tc) {
                    result_borrows.insert(receiver.local.0);
                }
                member.ret.clone()
            }
        };
        if &actual != expected
            && !actual
                .instantiate(&[], tc)
                .ok()
                .zip(expected.instantiate(&[], tc).ok())
                .is_some_and(|(actual, expected)| actual == expected)
        {
            return Err(MirErasedError::InvalidType);
        }
        initialized[dest] = true;
        origins[dest] = result_origins;
        local_borrows[dest] = result_borrows;
    }
    let returned = match &body.result {
        MirErasedOperand::Move(place) | MirErasedOperand::Copy(place) => place,
    };
    if !local_borrows
        .get(returned.local.0 as usize)
        .ok_or(MirErasedError::InvalidLocal)?
        .is_empty()
    {
        return Err(MirErasedError::BorrowEscape);
    }
    if operand(
        &body.result,
        function,
        contract,
        tc,
        &mut initialized,
        &local_borrows,
    )? != signature.ret
    {
        return Err(MirErasedError::InvalidType);
    }
    Ok(())
}

fn erased_type_may_borrow(
    ty: &MirErasedType,
    signature: &MirErasedSignature,
    contract: &super::MirBackendContract,
    tc: &crate::type_context::TypeContext,
) -> bool {
    if signature.require_borrow_free.contains(ty) {
        return false;
    }
    match ty {
        MirErasedType::Concrete(id) => {
            super::borrowck::liveness::type_id_contains_reference_with_contract(
                crate::type_context::TypeView::new(tc),
                contract,
                *id,
            )
        }
        MirErasedType::Pointer(_) => false,
        MirErasedType::Tuple(fields) => fields
            .iter()
            .any(|ty| erased_type_may_borrow(ty, signature, contract, tc)),
        MirErasedType::Array { element, .. } => {
            erased_type_may_borrow(element, signature, contract, tc)
        }
        _ => true,
    }
}

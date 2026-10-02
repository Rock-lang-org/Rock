//! Typed object-handle and unique-owner call ABI. Slots retain producer order.
use crate::ids::{DefId, TypeId};
use crate::types::ReceiverMode;

use super::{MirCallableKey, MirCallableSignature};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirObjectError {
    InvalidRuntimeType(TypeId),
    InvalidSchema(TypeId),
    InvalidSlot(MirVirtualTarget),
    UnsupportedBorrowEffects(MirVirtualTarget),
    InvalidVtable(MirVtableId),
    InvalidAdapter(MirCallableKey),
    InvalidOwnedPlan(MirOwnedObjectKey),
    InvalidConversion(super::MirFunctionId),
    InvalidCall(super::MirFunctionId),
    InvalidSelectorUse(super::MirFunctionId),
}

/// Only indirect handles with a declared schema cross the concrete runtime barrier.
/// The replacement is validation-only; LLVM never sees a fabricated payload type.
pub fn object_runtime_type_is_valid(
    contract: &super::MirBackendContract,
    tc: &crate::type_context::TypeContext,
    id: TypeId,
) -> bool {
    if object_runtime_type_is_valid_for_schemas(&contract.object_schemas, tc, id) {
        return true;
    }
    if !tc.type_id_tree_is_valid(id) {
        return false;
    }
    // Nominal arguments are layout metadata, not stored values. Admit an
    // object argument only after checking every instantiated physical field;
    // an owning wrapper with *T is valid, a wrapper storing T by value is not.
    fn metadata(
        ty: &crate::types::Type,
        contract: &super::MirBackendContract,
        tc: &crate::type_context::TypeContext,
    ) -> bool {
        use crate::type_services::visit::{fold_type, fold_type_children, TypeFolder};
        use crate::types::Type;
        struct Evidence<'a> {
            contract: &'a super::MirBackendContract,
            tc: &'a crate::type_context::TypeContext,
            valid: bool,
        }
        impl TypeFolder for Evidence<'_> {
            fn fold_type(&mut self, ty: Type) -> Type {
                if matches!(ty, Type::Object(_)) {
                    self.valid &= object_signature_is_closed(&ty)
                        && self
                            .tc
                            .id_for_type(&ty)
                            .is_some_and(|id| self.contract.object_schemas.contains_key(&id));
                    return Type::Unit;
                }
                fold_type_children(ty, self)
            }
        }
        let mut evidence = Evidence {
            contract,
            tc,
            valid: true,
        };
        let checked = fold_type(ty.clone(), &mut evidence);
        evidence.valid
            && crate::type_services::layout::TypeLayout::validate_runtime_type(&checked).is_ok()
    }
    fn value(
        ty: &crate::types::Type,
        contract: &super::MirBackendContract,
        tc: &crate::type_context::TypeContext,
        visiting: &mut std::collections::HashSet<crate::types::Type>,
        indirect: bool,
    ) -> bool {
        use crate::types::Type;
        match ty {
            Type::Object(_) | Type::ObjectSelf { .. } | Type::Witness(_) => false,
            Type::Reference { inner, .. } | Type::Pointer(inner) => {
                if matches!(inner.as_ref(), Type::Object(_)) {
                    metadata(inner, contract, tc)
                } else {
                    value(inner, contract, tc, visiting, true)
                }
            }
            Type::Struct { id, args } | Type::Enum { id, args } => {
                if !args.iter().all(|arg| metadata(arg, contract, tc)) {
                    return false;
                }
                let Some(layout) = contract.nominal_layouts.get(id) else {
                    return false;
                };
                let (parameters, fields): (&[_], Vec<_>) = match (ty, layout) {
                    (
                        Type::Struct { .. },
                        super::MirNominalLayout::Struct {
                            generic_params,
                            fields,
                            ..
                        },
                    ) => (generic_params, fields.iter().map(|(_, ty)| *ty).collect()),
                    (
                        Type::Enum { .. },
                        super::MirNominalLayout::Enum {
                            generic_params,
                            variants,
                            ..
                        },
                    ) => (
                        generic_params,
                        variants
                            .iter()
                            .flat_map(|variant| match &variant.fields {
                                super::MirVariantLayoutFields::Unit => vec![],
                                super::MirVariantLayoutFields::Positional(fields) => fields.clone(),
                                super::MirVariantLayoutFields::Named(fields) => {
                                    fields.iter().map(|(_, ty)| *ty).collect()
                                }
                            })
                            .collect(),
                    ),
                    _ => return false,
                };
                if parameters.len() != args.len() {
                    return false;
                }
                if !visiting.insert(ty.clone()) {
                    return indirect;
                }
                let substitution = parameters
                    .iter()
                    .copied()
                    .zip(args.iter().cloned())
                    .collect();
                let valid = fields.iter().all(|field| {
                    tc.type_id_tree_is_valid(*field)
                        && value(
                            &tc.type_for(*field).substitute_generics(&substitution),
                            contract,
                            tc,
                            visiting,
                            false,
                        )
                });
                visiting.remove(ty);
                valid
            }
            Type::Tuple(fields) => fields
                .iter()
                .all(|field| value(field, contract, tc, visiting, false)),
            Type::Array(element, _) | Type::Slice(element) => {
                value(element, contract, tc, visiting, false)
            }
            Type::Function {
                params,
                ret,
                captures,
                ..
            } => params
                .iter()
                .chain(std::iter::once(ret.as_ref()))
                .chain(captures.iter().map(|capture| &capture.ty))
                .all(|ty| value(ty, contract, tc, visiting, false)),
            _ => crate::type_services::layout::TypeLayout::validate_runtime_type(ty).is_ok(),
        }
    }
    value(
        &tc.type_for(id),
        contract,
        tc,
        &mut Default::default(),
        false,
    )
}

pub(crate) fn object_runtime_type_is_valid_for_schemas(
    schemas: &std::collections::BTreeMap<TypeId, MirObjectSchema>,
    tc: &crate::type_context::TypeContext,
    id: TypeId,
) -> bool {
    use crate::type_services::visit::{fold_type, fold_type_children, TypeFolder};
    use crate::types::Type;
    if !tc.type_id_tree_is_valid(id) {
        return false;
    }
    struct Handles<'a>(
        &'a std::collections::BTreeMap<TypeId, MirObjectSchema>,
        &'a crate::type_context::TypeContext,
    );
    impl TypeFolder for Handles<'_> {
        fn fold_type(&mut self, ty: Type) -> Type {
            if let Type::Reference { inner, .. } | Type::Pointer(inner) = &ty {
                if matches!(inner.as_ref(), Type::Object(_))
                    && self
                        .1
                        .id_for_type(inner)
                        .is_some_and(|id| self.0.contains_key(&id))
                {
                    return Type::Pointer(Box::new(Type::Unit));
                }
            }
            fold_type_children(ty, self)
        }
    }
    crate::type_services::layout::TypeLayout::validate_runtime_type(&fold_type(
        tc.type_for(id),
        &mut Handles(schemas, tc),
    ))
    .is_ok()
}

fn object_signature_is_closed(ty: &crate::types::Type) -> bool {
    use crate::type_services::visit::{visit_type, visit_type_children, TypeVisitor};
    use crate::types::Type;
    #[derive(Default)]
    struct Scope {
        objects: u32,
        binders: Vec<Vec<crate::type_services::kind::Kind>>,
        invalid: bool,
    }
    impl TypeVisitor for Scope {
        fn enter_object(&mut self) {
            self.objects += 1;
        }
        fn exit_object(&mut self) {
            self.objects -= 1;
        }
        fn enter_binders(&mut self, params: &[crate::type_services::kind::Kind]) {
            self.binders.push(params.to_vec());
        }
        fn exit_binders(&mut self) {
            self.binders.pop();
        }
        fn visit_type(&mut self, ty: &Type) {
            self.invalid |= match ty {
                Type::Witness(_) | Type::Generic(_) | Type::TypeVar(_) | Type::Error => true,
                Type::ObjectSelf { depth } => *depth >= self.objects,
                Type::BoundVar { depth, index, kind } => {
                    self.binders
                        .iter()
                        .rev()
                        .nth(*depth as usize)
                        .and_then(|params| params.get(*index as usize))
                        != Some(kind)
                }
                _ => false,
            };
            visit_type_children(ty, self);
        }
    }
    let mut scope = Scope::default();
    visit_type(ty, &mut scope);
    !scope.invalid
}

pub(crate) fn virtual_argument_requires_summary(
    contract: &super::MirBackendContract,
    tc: &crate::type_context::TypeContext,
    id: TypeId,
) -> bool {
    if super::borrowck::liveness::type_id_contains_reference_with_contract(
        crate::type_context::TypeView::new(tc),
        contract,
        id,
    ) {
        return true;
    }
    // A function signature does not prove that every supplied environment is
    // borrow-free. Inspect nominal fields too, using the same concrete layouts.
    fn contains_callable(
        plan: &super::MirPayloadDropPlan,
        tc: &crate::type_context::TypeContext,
    ) -> bool {
        if matches!(
            tc.try_ty(plan.ty),
            Some(crate::type_context::Ty::Function { .. })
        ) {
            return true;
        }
        match &plan.children {
            super::MirPayloadDropChildren::None => false,
            super::MirPayloadDropChildren::Fields(fields)
            | super::MirPayloadDropChildren::Variants(fields)
            | super::MirPayloadDropChildren::ClosureEnvironment { fields, .. } => {
                fields.iter().any(|field| contains_callable(field, tc))
            }
            super::MirPayloadDropChildren::Array { len, element } => {
                *len != 0 && contains_callable(element, tc)
            }
        }
    }
    let mut types = tc.clone();
    super::payload_drop_plan(contract, &mut types, id)
        .map_or(true, |plan| contains_callable(&plan, &types))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MirVtableId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MirVirtualTarget {
    /// Interned object signature (not the reference wrapping it).
    pub object: TypeId,
    pub slot: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MirOwnedObjectKey {
    pub owner: TypeId,
    pub object: TypeId,
    pub slot: MirOwnedObjectSlot,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MirOwnedObjectSlot {
    Concrete(u32),
    Erased { slot: u32, type_args: Vec<TypeId> },
}

impl MirOwnedObjectKey {
    /// Evidence only: consumption is authorized by this owner plan, never by a
    /// raw virtual selector constructed from the returned pointer.
    pub(crate) fn erased_evidence(
        &self,
        plan: &MirOwnedObjectPlan,
    ) -> Option<super::MirErasedCall> {
        let MirOwnedObjectSlot::Erased { slot, type_args } = &self.slot else {
            return None;
        };
        Some(super::MirErasedCall {
            target: super::MirErasedCallTarget::Virtual(MirVirtualTarget {
                object: self.object,
                slot: *slot,
            }),
            type_args: type_args.clone(),
            dictionaries: plan.dictionaries.clone(),
        })
    }
}

/// Canonically selected unsafe owner protocol calls, validated together with
/// the owning operand; raw object handles never authorize consumption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirOwnedObjectPlan {
    pub state: TypeId,
    pub into_parts: MirCallableKey,
    pub release: MirCallableKey,
    /// Exact dictionary evidence for the erased instantiation; empty for concrete slots.
    pub dictionaries: Vec<super::MirDictionaryId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirObjectSlot {
    pub trait_id: DefId,
    pub member_id: DefId,
    pub trait_args: Vec<TypeId>,
    pub receiver: ReceiverMode,
    /// Call-site types. Borrowed receivers are object references; consuming
    /// descriptors use Pointer(Object) but cannot be invoked by borrowed MIR.
    /// The entry ABI erases the receiver and applies `result` explicitly.
    pub signature: MirCallableSignature,
    pub result: MirObjectResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MirObjectResult {
    Direct,
    /// The actual entry returns a thin reference to its hidden concrete Self.
    /// Pair that returned address (not the old address) with receiver metadata.
    BorrowedSelf {
        mutable: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirObjectSchema {
    pub object: TypeId,
    pub slots: Vec<MirObjectSlot>,
    pub erased_slots: Vec<super::MirErasedObjectSlot>,
    /// Explicit target views, in producer-defined order.
    pub views: Vec<TypeId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirVtable {
    pub object: TypeId,
    pub concrete: TypeId,
    /// Concrete methods have the same ABI except for the erased thin receiver.
    pub methods: Vec<MirCallableKey>,
    pub erased_methods: Vec<super::MirErasedKey>,
    pub views: Vec<MirVtableId>,
    /// Payload destruction only; never owner deallocation.
    pub drop: Option<MirCallableKey>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirObjectConversion {
    Concrete(MirVtableId),
    Upcast { source: TypeId, view: u32 },
}

/// Validate complete runtime evidence before either borrow checking or LLVM.
pub fn validate_object_contract(program: &super::MirProgram) -> Vec<MirObjectError> {
    use super::{MirCallableKind, MirPassMode};
    use crate::types::Type;
    let contract = &program.backend_contract;
    let tc = &program.type_context;
    let ty = |id| tc.type_id_tree_is_valid(id).then(|| tc.type_for(id));
    let mut errors = Vec::new();
    for declaration in contract.callables.values() {
        if !super::object_adapter_is_valid(contract, tc, declaration) {
            errors.push(MirObjectError::InvalidAdapter(declaration.key.clone()));
        }
    }
    for (id, schema) in &contract.object_schemas {
        let view_proofs_valid = match ty(*id) {
            Some(Type::Object(source)) => {
                let guarantees = std::iter::once(&source.principal)
                    .chain(source.guarantees.iter())
                    .collect::<std::collections::BTreeSet<_>>();
                schema.views.iter().all(|view| match ty(*view) {
                    Some(Type::Object(target)) => {
                        std::iter::once(&target.principal)
                            .chain(target.guarantees.iter())
                            .all(|bound| guarantees.contains(bound))
                            && target
                                .bindings
                                .iter()
                                .all(|binding| source.bindings.contains(binding))
                    }
                    _ => false,
                })
            }
            _ => false,
        };
        if schema.object != *id
            || !view_proofs_valid
            || schema
                .views
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != schema.views.len()
            || !matches!(ty(*id), Some(Type::Object(_)))
            || ty(*id).is_some_and(|ty| !object_signature_is_closed(&ty))
            || schema
                .views
                .iter()
                .any(|view| !contract.object_schemas.contains_key(view))
        {
            errors.push(MirObjectError::InvalidSchema(*id));
        }
        let mut members = std::collections::HashSet::new();
        for (index, slot) in schema.slots.iter().enumerate() {
            let guarantee_exists = match tc.try_ty(*id) {
                Some(crate::type_context::Ty::Object(object)) => std::iter::once(&object.principal)
                    .chain(object.guarantees.iter())
                    .any(|bound| {
                        bound.trait_id == slot.trait_id && bound.type_args == slot.trait_args
                    }),
                _ => false,
            };
            // Until symbolic store summaries exist, reject signatures that could
            // retain a second input borrow inside the erased payload.
            if slot.signature.params.iter().skip(1).any(|param| {
                tc.type_id_tree_is_valid(param.semantic_ty)
                    && virtual_argument_requires_summary(contract, tc, param.semantic_ty)
            }) {
                errors.push(MirObjectError::UnsupportedBorrowEffects(MirVirtualTarget {
                    object: *id,
                    slot: index as u32,
                }));
            }
            let receiver_ok = slot.signature.params.first().is_some_and(|param| {
                if slot.receiver == ReceiverMode::Move {
                    return param.pass_mode == MirPassMode::Pointer && matches!(ty(param.semantic_ty), Some(Type::Pointer(inner)) if tc.id_for_type(&inner) == Some(*id));
                }
                param.pass_mode == MirPassMode::FatDirect
                    && matches!(ty(param.semantic_ty),
                    Some(Type::Reference { inner, mutable }) if tc.id_for_type(&inner) == Some(*id)
                    && mutable == (slot.receiver == ReceiverMode::Mut))
            });
            let concrete_types = slot
                .signature
                .params
                .iter()
                .skip(1)
                .map(|p| p.semantic_ty)
                .chain([slot.signature.ret.semantic_ty, slot.signature.ret.abi_ty]);
            if !receiver_ok
                || !guarantee_exists
                || slot
                    .trait_args
                    .iter()
                    .any(|id| !tc.type_id_tree_is_valid(*id))
                || !members.insert((slot.trait_id, slot.member_id, slot.trait_args.clone()))
                || concrete_types.into_iter().any(|id| {
                    ty(id).is_none_or(|ty| {
                        if matches!(slot.result, MirObjectResult::BorrowedSelf { .. })
                            && id == slot.signature.ret.semantic_ty
                        {
                            !object_runtime_type_is_valid(contract, tc, id)
                        } else {
                            crate::type_services::layout::TypeLayout::validate_runtime_type(&ty)
                                .is_err()
                        }
                    })
                })
                || slot
                    .signature
                    .params
                    .iter()
                    .skip(1)
                    .any(|p| p.pass_mode != MirPassMode::Direct)
                || slot.signature.ret.semantic_ty != slot.signature.ret.abi_ty
                || super::payload_drop_plan(
                    contract,
                    &mut tc.clone(),
                    slot.signature.ret.semantic_ty,
                )
                .is_err()
            {
                errors.push(MirObjectError::InvalidSlot(MirVirtualTarget {
                    object: *id,
                    slot: index as u32,
                }));
            }
            if let MirObjectResult::BorrowedSelf { mutable } = slot.result {
                if !matches!(ty(slot.signature.ret.semantic_ty), Some(Type::Reference { inner, mutable: actual }) if tc.id_for_type(&inner) == Some(*id) && actual == mutable)
                    || (mutable && slot.receiver != ReceiverMode::Mut)
                {
                    errors.push(MirObjectError::InvalidSlot(MirVirtualTarget {
                        object: *id,
                        slot: index as u32,
                    }));
                }
            }
        }
        for (index, slot) in schema.erased_slots.iter().enumerate() {
            let bound_exists = matches!(tc.try_ty(*id), Some(crate::type_context::Ty::Object(object)) if std::iter::once(&object.principal).chain(object.guarantees.iter()).any(|bound| bound.trait_id == slot.trait_id && bound.type_args == slot.trait_args));
            let expected_receiver = if slot.receiver == ReceiverMode::Move {
                super::MirErasedType::Parameter(0)
            } else {
                super::MirErasedType::Reference {
                    inner: Box::new(super::MirErasedType::Parameter(0)),
                    mutable: slot.receiver == ReceiverMode::Mut,
                }
            };
            if !bound_exists
                || !members.insert((slot.trait_id, slot.member_id, slot.trait_args.clone()))
                || slot.signature.params.first() != Some(&expected_receiver)
                || super::validate_erased_signature(&slot.signature, contract, tc).is_err()
            {
                errors.push(MirObjectError::InvalidSlot(MirVirtualTarget {
                    object: *id,
                    slot: index as u32,
                }));
            }
        }
    }
    for (id, table) in &contract.vtables {
        let Some(schema) = contract.object_schemas.get(&table.object) else {
            errors.push(MirObjectError::InvalidVtable(*id));
            continue;
        };
        let mut valid = ty(table.concrete).is_some_and(|ty| {
            object_runtime_type_is_valid(contract, tc, table.concrete)
                && !matches!(ty, Type::Slice(_) | Type::Str)
        }) && table.methods.len() == schema.slots.len()
            && table.views.len() == schema.views.len()
            && table.erased_methods.len() == schema.erased_slots.len();
        for (key, slot) in table.erased_methods.iter().zip(&schema.erased_slots) {
            valid &= contract.erased.descriptors.contains_key(&table.concrete)
                && key.static_args.first() == Some(&table.concrete)
                && contract.erased.functions.get(key).is_some_and(|function| {
                    super::erased_entry_matches_object_slot(function, slot, table.concrete, tc)
                });
        }
        for (key, slot) in table.methods.iter().zip(&schema.slots) {
            valid &= match contract.callable(key).map(|decl| &decl.kind) {
                Some(MirCallableKind::ObjectAdapter(super::MirObjectAdapter::NativeCallable {
                    trait_id,
                    member_id,
                    kind,
                    ..
                })) => {
                    *trait_id == slot.trait_id
                        && *member_id == slot.member_id
                        && slot.receiver
                            == match kind {
                                crate::types::CallableKind::Fn => ReceiverMode::Shared,
                                crate::types::CallableKind::FnMut => ReceiverMode::Mut,
                                crate::types::CallableKind::FnOnce => ReceiverMode::Move,
                            }
                }
                Some(MirCallableKind::ObjectAdapter(
                    super::MirObjectAdapter::ConsumingMethod { .. },
                )) => slot.receiver == ReceiverMode::Move,
                Some(MirCallableKind::ObjectAdapter(super::MirObjectAdapter::PayloadDrop(_))) => {
                    false
                }
                _ => slot.receiver != ReceiverMode::Move,
            };
            valid &= contract.callable(key).is_some_and(|decl| {
                matches!(
                    decl.kind,
                    MirCallableKind::LocalBody { .. } | MirCallableKind::ObjectProvided | MirCallableKind::ObjectAdapter(_)
                ) && match slot.result {
                    MirObjectResult::Direct => decl.signature.ret == slot.signature.ret,
                    MirObjectResult::BorrowedSelf { mutable } => decl.signature.ret.semantic_ty == decl.signature.ret.abi_ty
                        && matches!(ty(decl.signature.ret.semantic_ty), Some(Type::Reference { inner, mutable: actual }) if tc.id_for_type(&inner) == Some(table.concrete) && actual == mutable),
                }
                    && decl.signature.params.len() == slot.signature.params.len()
                    && decl.signature.params.get(1..) == slot.signature.params.get(1..)
                    && decl.signature.params.first().is_some_and(|p| {
                        if slot.receiver == ReceiverMode::Move {
                            return p.pass_mode == MirPassMode::Pointer && matches!(ty(p.semantic_ty), Some(Type::Pointer(inner)) if tc.id_for_type(&inner) == Some(table.concrete));
                        }
                        p.pass_mode == MirPassMode::Pointer
                            && matches!(ty(p.semantic_ty), Some(Type::Reference { inner, mutable })
                            if tc.id_for_type(&inner) == Some(table.concrete)
                                && mutable == (slot.receiver == ReceiverMode::Mut))
                    })
            });
        }
        for (view, target) in table.views.iter().zip(&schema.views) {
            valid &= contract
                .vtables
                .get(view)
                .is_some_and(|view| view.object == *target && view.concrete == table.concrete);
        }
        if let Some(drop) = &table.drop {
            let full_adapter = matches!(contract.callable(drop).map(|d| &d.kind), Some(MirCallableKind::ObjectAdapter(super::MirObjectAdapter::PayloadDrop(plan))) if plan.ty == table.concrete);
            valid &= full_adapter
                || (contract.drop_glue.get(&table.concrete) == Some(drop)
                    && super::payload_drop_plan(contract, &mut tc.clone(), table.concrete)
                        .is_ok_and(|mut plan| {
                            plan.user_drop = None;
                            !plan.has_drop()
                        }));
            valid &= contract.callable(drop).is_some_and(|declaration| {
                declaration.signature.params.len() == 1
                    && declaration.signature.params.first().is_some_and(|param| param.pass_mode == MirPassMode::Pointer
                        && matches!(ty(param.semantic_ty), Some(Type::Reference { inner, .. }) if tc.id_for_type(&inner) == Some(table.concrete)))
                    && matches!(ty(declaration.signature.ret.semantic_ty), Some(Type::Unit))
                    && declaration.signature.ret.abi_ty == declaration.signature.ret.semantic_ty
            });
        } else {
            valid &= super::payload_drop_plan(contract, &mut tc.clone(), table.concrete)
                .is_ok_and(|plan| !plan.has_drop());
        }
        if !valid {
            errors.push(MirObjectError::InvalidVtable(*id));
        }
    }
    for (key, plan) in &contract.owned_object_calls {
        if !owned_object_plan_is_valid(program, key, plan) {
            errors.push(MirObjectError::InvalidOwnedPlan(key.clone()));
        }
    }
    for function in program.functions.values() {
        for id in function
            .local_decls
            .iter()
            .map(|local| local.ty)
            .chain(std::iter::once(function.ret_type))
        {
            if tc.type_id_tree_is_valid(id)
                && crate::type_services::visit::type_any(&tc.type_for(id), |ty| {
                    matches!(ty, Type::Object(_) | Type::ObjectSelf { .. })
                })
                && !object_runtime_type_is_valid(contract, tc, id)
            {
                errors.push(MirObjectError::InvalidRuntimeType(id));
            }
        }
        validate_object_operations(program, function, &mut errors);
    }
    errors
}

fn operand_type(
    program: &super::MirProgram,
    function: &super::MirFunction,
    operand: &super::Operand,
) -> Option<TypeId> {
    use super::{Constant, Operand};
    use crate::types::Type;
    match operand {
        Operand::Copy(place) | Operand::Move(place) => {
            program
                .backend_contract
                .place_type_id(&program.type_context, function, place)
        }
        Operand::Constant(value) => program.type_context.id_for_type(&match value {
            Constant::Int(_) => Type::I64,
            Constant::Float(_) => Type::F64,
            Constant::Bool(_) => Type::Bool,
            Constant::Char(_) => Type::Char,
            Constant::Unit => Type::Unit,
            _ => return None,
        }),
    }
}

fn validate_object_operations(
    program: &super::MirProgram,
    function: &super::MirFunction,
    errors: &mut Vec<MirObjectError>,
) {
    use super::{Constant, Operand, Rvalue, StatementKind, Terminator};
    use crate::types::Type;
    let tc = &program.type_context;
    let contract = &program.backend_contract;
    let ref_type = |id: TypeId| {
        if !tc.type_id_tree_is_valid(id) {
            return None;
        }
        match tc.type_for(id) {
            Type::Reference { inner, mutable } => Some((tc.id_for_type(&inner)?, mutable)),
            Type::Pointer(inner) => Some((tc.id_for_type(&inner)?, true)),
            _ => None,
        }
    };
    for block in &function.basic_blocks {
        for statement in &block.statements {
            if let StatementKind::Assign(destination, value) = &statement.kind {
                let destination_ty = contract.place_type_id(tc, function, destination);
                if destination_ty.and_then(ref_type).is_some_and(|(inner, _)| {
                    matches!(tc.try_ty(inner), Some(crate::type_context::Ty::Object(_)))
                }) {
                    let valid = match value {
                        Rvalue::Object(_, _) => true, // checked below against its evidence
                        Rvalue::Use(source) => {
                            operand_type(program, function, source) == destination_ty
                        }
                        Rvalue::Ref(mutability, place) => {
                            let mut handle = place.clone();
                            destination_ty
                                .is_some_and(|id| matches!(tc.type_for(id), Type::Reference { .. }))
                                && matches!(handle.projection.pop(), Some(super::Projection::Deref))
                                && contract
                                    .place_type_id(tc, function, &handle)
                                    .and_then(ref_type)
                                    .zip(destination_ty.and_then(ref_type))
                                    .is_some_and(|((source, source_mut), (target, target_mut))| {
                                        source == target
                                            && (!target_mut || source_mut)
                                            && target_mut == (*mutability == super::Mutability::Mut)
                                    })
                        }
                        _ => false,
                    };
                    if !valid {
                        errors.push(MirObjectError::InvalidConversion(function.id.clone()));
                    }
                }
                if let Rvalue::Cast(source, target) = value {
                    if operand_type(program, function, source)
                        .and_then(ref_type)
                        .is_some_and(|(inner, _)| {
                            matches!(tc.try_ty(inner), Some(crate::type_context::Ty::Object(_)))
                        })
                        && operand_type(program, function, source) != Some(*target)
                    {
                        errors.push(MirObjectError::InvalidConversion(function.id.clone()));
                    }
                }
            }
            if let StatementKind::Assign(destination, Rvalue::Object(source, conversion)) =
                &statement.kind
            {
                let source_ty = operand_type(program, function, source).and_then(ref_type);
                let target_ty = contract
                    .place_type_id(tc, function, destination)
                    .and_then(ref_type);
                let same_handle_kind = operand_type(program, function, source)
                    .zip(contract.place_type_id(tc, function, destination))
                    .is_some_and(|(source, target)| {
                        matches!(tc.type_for(source), Type::Pointer(_))
                            == matches!(tc.type_for(target), Type::Pointer(_))
                    });
                let valid = same_handle_kind
                    && source_ty.zip(target_ty).is_some_and(
                        |((source, source_mut), (target, target_mut))| {
                            if target_mut && !source_mut {
                                return false;
                            }
                            match conversion {
                                MirObjectConversion::Concrete(id) => {
                                    contract.vtables.get(id).is_some_and(|table| {
                                        table.concrete == source && table.object == target
                                    })
                                }
                                MirObjectConversion::Upcast {
                                    source: expected,
                                    view,
                                } => {
                                    source == *expected
                                        && contract
                                            .object_schemas
                                            .get(expected)
                                            .and_then(|schema| schema.views.get(*view as usize))
                                            == Some(&target)
                                }
                            }
                        },
                    );
                if !valid {
                    errors.push(MirObjectError::InvalidConversion(function.id.clone()));
                }
            }
            // A virtual selector is not an address and cannot escape its call site.
            let mut inspect = |operand: &Operand| {
                if matches!(
                    operand,
                    Operand::Constant(
                        Constant::VirtualTarget(_)
                            | Constant::OwnedObjectCall(_)
                            | Constant::Callable(super::MirCallable::Resolved(
                                MirCallableKey::ObjectAdapter(_)
                            ))
                    )
                ) {
                    errors.push(MirObjectError::InvalidSelectorUse(function.id.clone()));
                }
            };
            match &statement.kind {
                StatementKind::Assign(
                    _,
                    Rvalue::Use(op)
                    | Rvalue::Cast(op, _)
                    | Rvalue::Object(op, _)
                    | Rvalue::UnaryOp(_, op),
                ) => inspect(op),
                StatementKind::Assign(_, Rvalue::BinaryOp(_, a, b)) => {
                    inspect(a);
                    inspect(b);
                }
                StatementKind::Assign(_, Rvalue::Aggregate(_, ops)) => ops.iter().for_each(inspect),
                StatementKind::Assert(assertion) => assertion.operands.iter().for_each(inspect),
                _ => {}
            }
        }
        if let Some(
            Terminator::SwitchInt { discr, .. } | Terminator::SwitchIntWithOrigin { discr, .. },
        ) = &block.terminator
        {
            if matches!(
                discr,
                Operand::Constant(
                    Constant::VirtualTarget(_)
                        | Constant::OwnedObjectCall(_)
                        | Constant::Callable(super::MirCallable::Resolved(
                            MirCallableKey::ObjectAdapter(_)
                        ))
                )
            ) {
                errors.push(MirObjectError::InvalidSelectorUse(function.id.clone()));
            }
        }
        if let Some(Terminator::Call {
            func,
            args,
            destination,
            target: continuation,
            ..
        }) = &block.terminator
        {
            if let Operand::Constant(Constant::Callable(super::MirCallable::Resolved(
                MirCallableKey::Intrinsic(intrinsic),
            ))) = func
            {
                let has_object_handle = args.iter().any(|arg| {
                    operand_type(program, function, arg)
                        .and_then(ref_type)
                        .is_some_and(|(inner, _)| {
                            matches!(tc.try_ty(inner), Some(crate::type_context::Ty::Object(_)))
                        })
                });
                if has_object_handle
                    && !matches!(
                        intrinsic,
                        super::MirIntrinsicId::Forget
                            | super::MirIntrinsicId::SizeOf
                            | super::MirIntrinsicId::AlignOf
                            | super::MirIntrinsicId::SizeOfValue
                            | super::MirIntrinsicId::AlignOfValue
                            | super::MirIntrinsicId::DropInPlace
                    )
                {
                    errors.push(MirObjectError::InvalidCall(function.id.clone()));
                }
                if matches!(
                    intrinsic,
                    super::MirIntrinsicId::SizeOfValue
                        | super::MirIntrinsicId::AlignOfValue
                        | super::MirIntrinsicId::DropInPlace
                ) {
                    let valid = args.len() == 1
                        && operand_type(program, function, &args[0]).is_some_and(|id| {
                            if !tc.type_id_tree_is_valid(id) {
                                return false;
                            }
                            match tc.type_for(id) {
                                Type::Pointer(inner) => {
                                    *intrinsic != super::MirIntrinsicId::DropInPlace
                                        || matches!(*inner, Type::Object(_))
                                }
                                Type::Reference { .. } => {
                                    *intrinsic != super::MirIntrinsicId::DropInPlace
                                }
                                _ => false,
                            }
                        })
                        && contract
                            .place_type_id(tc, function, destination)
                            .is_some_and(|id| {
                                tc.type_for(id)
                                    == if *intrinsic == super::MirIntrinsicId::DropInPlace {
                                        Type::Unit
                                    } else {
                                        Type::I64
                                    }
                            });
                    if !valid {
                        errors.push(MirObjectError::InvalidCall(function.id.clone()));
                    }
                }
            }
            if matches!(
                func,
                Operand::Constant(Constant::Callable(super::MirCallable::Resolved(
                    MirCallableKey::ObjectAdapter(_)
                )))
            ) {
                errors.push(MirObjectError::InvalidSelectorUse(function.id.clone()));
            }
            if args.iter().any(|arg| {
                matches!(
                    arg,
                    Operand::Constant(
                        Constant::VirtualTarget(_)
                            | Constant::OwnedObjectCall(_)
                            | Constant::Callable(super::MirCallable::Resolved(
                                MirCallableKey::ObjectAdapter(_)
                            ))
                    )
                )
            }) {
                errors.push(MirObjectError::InvalidSelectorUse(function.id.clone()));
            }
            if let Operand::Constant(Constant::VirtualTarget(target)) = func {
                let valid = contract
                    .object_schemas
                    .get(&target.object)
                    .and_then(|s| s.slots.get(target.slot as usize))
                    .is_some_and(|slot| {
                        slot.receiver != ReceiverMode::Move
                            && args.len() == slot.signature.params.len()
                            && args.iter().zip(&slot.signature.params).all(|(arg, param)| {
                                operand_type(program, function, arg) == Some(param.semantic_ty)
                            })
                            && if matches!(
                                tc.try_ty(slot.signature.ret.semantic_ty),
                                Some(crate::type_context::Ty::Never)
                            ) {
                                function
                                    .basic_blocks
                                    .get(continuation.0)
                                    .is_some_and(|block| {
                                        block.statements.is_empty()
                                            && matches!(
                                                block.terminator,
                                                Some(Terminator::Unreachable { .. })
                                            )
                                    })
                            } else {
                                contract.place_type_id(tc, function, destination)
                                    == Some(slot.signature.ret.semantic_ty)
                            }
                    });
                if !valid {
                    errors.push(MirObjectError::InvalidCall(function.id.clone()));
                }
            }
            if let Operand::Constant(Constant::OwnedObjectCall(key)) = func {
                let valid = (|| {
                    let plan = contract.owned_object_calls.get(key)?;
                    if !owned_object_plan_is_valid(program, key, plan)
                        || !matches!(args.first(), Some(Operand::Move(_)))
                        || operand_type(program, function, args.first()?) != Some(key.owner)
                    {
                        return None;
                    }
                    let result = match &key.slot {
                        MirOwnedObjectSlot::Concrete(slot) => {
                            let slot = contract
                                .object_schemas
                                .get(&key.object)?
                                .slots
                                .get(*slot as usize)?;
                            if args.len() != slot.signature.params.len()
                                || !args
                                    .iter()
                                    .skip(1)
                                    .zip(slot.signature.params.iter().skip(1))
                                    .all(|(arg, param)| {
                                        operand_type(program, function, arg)
                                            == Some(param.semantic_ty)
                                    })
                            {
                                return None;
                            }
                            let result = tc.type_for(slot.signature.ret.semantic_ty);
                            if result != Type::Never
                                && contract.place_type_id(tc, function, destination)
                                    != Some(slot.signature.ret.semantic_ty)
                            {
                                return None;
                            }
                            result
                        }
                        MirOwnedObjectSlot::Erased { slot, type_args } => {
                            super::erased::validate_owned_erased_call(
                                program,
                                function,
                                key,
                                args,
                                destination,
                            )
                            .ok()?;
                            let signature = &contract
                                .object_schemas
                                .get(&key.object)?
                                .erased_slots
                                .get(*slot as usize)?
                                .signature;
                            let bindings = std::iter::once(None)
                                .chain(type_args.iter().map(|id| Some(tc.type_for(*id))))
                                .collect::<Vec<_>>();
                            signature.ret.instantiate_optional(&bindings, tc).ok()?
                        }
                    };
                    if result == Type::Never {
                        if !function
                            .basic_blocks
                            .get(continuation.0)
                            .is_some_and(|block| {
                                block.statements.is_empty()
                                    && matches!(
                                        block.terminator,
                                        Some(Terminator::Unreachable { .. })
                                    )
                            })
                        {
                            return None;
                        }
                    }
                    Some(())
                })()
                .is_some();
                if !valid {
                    errors.push(MirObjectError::InvalidCall(function.id.clone()));
                }
            }
        }
    }
}

pub(crate) fn owned_object_plan_is_valid(
    program: &super::MirProgram,
    key: &MirOwnedObjectKey,
    plan: &MirOwnedObjectPlan,
) -> bool {
    use crate::types::Type;
    let tc = &program.type_context;
    let contract = &program.backend_contract;
    let Some(schema) = contract.object_schemas.get(&key.object) else {
        return false;
    };
    let (receiver, result) = match &key.slot {
        MirOwnedObjectSlot::Concrete(index) => {
            let Some(slot) = schema.slots.get(*index as usize) else {
                return false;
            };
            if !plan.dictionaries.is_empty() {
                return false;
            }
            (slot.receiver, slot.result)
        }
        MirOwnedObjectSlot::Erased { slot, .. } => {
            let Some(slot) = schema.erased_slots.get(*slot as usize) else {
                return false;
            };
            if slot.signature.params.first() != Some(&super::MirErasedType::Parameter(0))
                || slot.signature.ret.contains_parameter(0)
            {
                return false;
            }
            let Some(evidence) = key.erased_evidence(plan) else {
                return false;
            };
            if super::erased::validate_erased_environment(contract, tc, &evidence).is_err() {
                return false;
            }
            (slot.receiver, slot.result)
        }
    };
    let Some(split) = contract.callable(&plan.into_parts) else {
        return false;
    };
    let Some(release) = contract.callable(&plan.release) else {
        return false;
    };
    if !matches!(plan.into_parts, MirCallableKey::Instance(_))
        || !matches!(plan.release, MirCallableKey::Instance(_))
        || !matches!(
            split.kind,
            super::MirCallableKind::LocalBody { .. } | super::MirCallableKind::ObjectProvided
        )
        || !matches!(
            release.kind,
            super::MirCallableKind::LocalBody { .. } | super::MirCallableKind::ObjectProvided
        )
    {
        return false;
    }
    let split = &split.signature;
    let release = &release.signature;
    if ![
        key.owner,
        key.object,
        plan.state,
        split.ret.semantic_ty,
        release.ret.semantic_ty,
    ]
    .into_iter()
    .chain(
        split
            .params
            .iter()
            .chain(&release.params)
            .map(|param| param.semantic_ty),
    )
    .all(|id| tc.type_id_tree_is_valid(id))
    {
        return false;
    }
    if !matches!(
        tc.try_ty(key.owner),
        Some(crate::type_context::Ty::Struct { .. } | crate::type_context::Ty::Enum { .. })
    ) || receiver != ReceiverMode::Move
        || result != MirObjectResult::Direct
        || split.params.len() != 1
        || split.params[0].semantic_ty != key.owner
        || split.params[0].pass_mode != super::MirPassMode::Direct
        || split.ret.abi_ty != split.ret.semantic_ty
        || release.params.len() != 2
        || release
            .params
            .iter()
            .any(|param| param.pass_mode != super::MirPassMode::Direct)
        || release.ret.abi_ty != release.ret.semantic_ty
        || release.params[1].semantic_ty != plan.state
        || tc.type_for(release.params[0].semantic_ty) != Type::Pointer(Box::new(Type::U8))
        || tc.type_for(release.ret.semantic_ty) != Type::Unit
    {
        return false;
    }
    tc.type_for(split.ret.semantic_ty)
        == Type::Tuple(vec![
            Type::Pointer(Box::new(tc.type_for(key.object))),
            tc.type_for(plan.state),
        ])
}

#[cfg(test)]
mod runtime_type_tests {
    use super::{object_runtime_type_is_valid, MirObjectSchema};
    use crate::ids::{CrateId, DefId, HirLocalId, LocalDefId};
    use crate::mir::{MirBackendContract, MirNominalLayout};
    use crate::type_context::TypeContext;
    use crate::types::{GenericParamId, ObjectType, TraitBound, Type, WitnessId};

    #[test]
    fn nominal_object_arguments_require_schema_and_indirect_physical_fields() {
        let mut tc = TypeContext::new();
        let owner = DefId::new(CrateId(0), LocalDefId(100));
        let protocol = DefId::new(CrateId(0), LocalDefId(101));
        let parameter = GenericParamId { owner, index: 0 };
        let object_ty = Type::Object(Box::new(ObjectType {
            principal: TraitBound {
                trait_id: protocol,
                type_args: vec![],
            },
            guarantees: vec![],
            bindings: vec![],
        }));
        let object = tc.intern_type(&object_ty);
        let wrapper = tc.intern_type(&Type::Struct {
            id: owner,
            args: vec![object_ty.clone()],
        });
        let raw = tc.intern_type(&Type::Pointer(Box::new(Type::Generic(parameter))));
        let generic = tc.intern_type(&Type::Generic(parameter));
        let parts = tc.intern_type(&Type::Tuple(vec![
            Type::Pointer(Box::new(object_ty)),
            Type::Unit,
        ]));
        let mut contract = MirBackendContract::default();
        contract.nominal_layouts.insert(
            owner,
            MirNominalLayout::Struct {
                id: owner,
                generic_params: vec![parameter],
                fields: vec![("payload".into(), raw)],
            },
        );
        assert!(!object_runtime_type_is_valid(&contract, &tc, wrapper));
        contract.object_schemas.insert(
            object,
            MirObjectSchema {
                object,
                slots: vec![],
                erased_slots: vec![],
                views: vec![],
            },
        );
        assert!(object_runtime_type_is_valid(&contract, &tc, wrapper));
        assert!(object_runtime_type_is_valid(&contract, &tc, parts));
        assert!(!object_runtime_type_is_valid(&contract, &tc, object));
        let witness_wrapper = tc.intern_type(&Type::Struct {
            id: owner,
            args: vec![Type::Witness(WitnessId {
                owner,
                local: HirLocalId(0),
            })],
        });
        assert!(!object_runtime_type_is_valid(
            &contract,
            &tc,
            witness_wrapper
        ));
        contract.nominal_layouts.insert(
            owner,
            MirNominalLayout::Struct {
                id: owner,
                generic_params: vec![parameter],
                fields: vec![("payload".into(), generic)],
            },
        );
        assert!(!object_runtime_type_is_valid(&contract, &tc, wrapper));
        contract.nominal_layouts.remove(&owner);
        assert!(!object_runtime_type_is_valid(&contract, &tc, wrapper));
    }
}

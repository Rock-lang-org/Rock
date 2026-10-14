//! Bound, descriptor-backed AOT bodies. These types never enter concrete MIR's
//! TypeId locals or LLVM's concrete type mapper.
use super::{MirCallableKey, MirPayloadDropPlan};
use crate::ids::{DefId, TypeId};
use crate::type_services::kind::Kind;
use crate::types::{GenericParamId, NominalTypeKind, ReceiverMode, Type};
use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
#[path = "erased_borrowing_tests.rs"]
mod borrowing_tests;

#[cfg(test)]
#[path = "erased_layout_factory_tests.rs"]
mod layout_factory_tests;

#[path = "native_retention.rs"]
mod native_retention;

pub(super) fn capture_free_closure_target(
    program: &super::MirProgram,
    owner: &super::MirFunction,
    closure: &super::MirClosure,
    ty: TypeId,
) -> Option<MirCallableKey> {
    native_retention::capture_free_closure_target(program, owner, closure, ty)
}

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
    /// Rigid body-local binder, supplied by a dominating Open, never a public ABI parameter.
    /// Keep body-only extensions last to preserve serialized public signature tags.
    Scoped(u32),
    /// Depth-zero binder of a dictionary member; Parameter is its parent scope.
    /// Never valid in an ordinary erased body/signature until explicitly opened.
    MemberParameter(u32),
}

#[cfg(test)]
mod wire_tests {
    use bincode::Options;

    use super::MirErasedType;
    use crate::ids::{CrateId, DefId, LocalDefId};
    use crate::types::NominalTypeKind;

    #[test]
    fn erased_type_public_wire_tags_survive_body_only_extensions() {
        let cases: Vec<(u32, MirErasedType<u32>)> = vec![
            (0, MirErasedType::Concrete(17)),
            (1, MirErasedType::Parameter(2)),
            (
                2,
                MirErasedType::Reference {
                    inner: Box::new(MirErasedType::Parameter(2)),
                    mutable: false,
                },
            ),
            (
                3,
                MirErasedType::Pointer(Box::new(MirErasedType::Concrete(17))),
            ),
            (4, MirErasedType::Tuple(vec![MirErasedType::Concrete(17)])),
            (
                5,
                MirErasedType::Array {
                    element: Box::new(MirErasedType::Concrete(17)),
                    len: 3,
                },
            ),
            (
                6,
                MirErasedType::Nominal {
                    id: DefId::new(CrateId(1), LocalDefId(7)),
                    flavor: NominalTypeKind::Struct,
                    args: vec![MirErasedType::Parameter(2)],
                },
            ),
            (
                7,
                MirErasedType::Apply {
                    constructor: Box::new(MirErasedType::Parameter(0)),
                    args: vec![MirErasedType::Concrete(17)],
                },
            ),
        ];
        for (tag, ty) in cases {
            let bytes = bincode::DefaultOptions::new()
                .with_fixint_encoding()
                .serialize(&ty)
                .unwrap();
            assert_eq!(&bytes[..4], &tag.to_le_bytes());
            let restored: MirErasedType<u32> = bincode::DefaultOptions::new()
                .with_fixint_encoding()
                .reject_trailing_bytes()
                .deserialize(&bytes)
                .unwrap();
            assert_eq!(restored, ty);
        }
    }
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
    pub binder: super::MirErasedMemberBinder<T>,
    pub receiver: Option<ReceiverMode>,
    /// Instance params include the subject at zero; static params do not.
    pub params: Vec<MirErasedType<T>>,
    pub ret: MirErasedType<T>,
    pub borrowing: crate::hir::borrowing::BorrowContract,
}

/// Derive declaration authority without exposing a hidden nominal/runtime payload.
/// Visible reference/tuple/array structure is normalized before this derivation;
/// nominal/runtime payloads retain an opaque origin envelope.
fn derive_erased_borrow_contract(
    params: &[MirErasedType<Type>],
    ret: &MirErasedType<Type>,
    receiver: Option<ReceiverMode>,
) -> crate::hir::borrowing::BorrowContract {
    use crate::hir::borrowing::{
        BorrowContract, BorrowPath, BorrowProjection, BorrowRetention, BorrowReturn, BorrowSource,
        BorrowSourceKind,
    };
    fn paths<T>(ty: &MirErasedType<T>, path: &mut BorrowPath, out: &mut Vec<BorrowPath>) {
        match ty {
            MirErasedType::Pointer(_) => {}
            MirErasedType::Tuple(fields) => {
                for (index, field) in fields.iter().enumerate() {
                    path.push(BorrowProjection::Field(index as u32));
                    paths(field, path, out);
                    path.pop();
                }
            }
            MirErasedType::Array { element, .. } => {
                path.push(BorrowProjection::Element);
                paths(element, path, out);
                path.pop();
            }
            _ => out.push(path.clone()),
        }
    }
    fn at<'a, T>(mut ty: &'a MirErasedType<T>, path: &BorrowPath) -> &'a MirErasedType<T> {
        for projection in path {
            ty = match (projection, ty) {
                (BorrowProjection::Field(index), MirErasedType::Tuple(fields)) => {
                    &fields[*index as usize]
                }
                (BorrowProjection::Element, MirErasedType::Array { element, .. }) => element,
                _ => unreachable!("paths were derived from this type"),
            };
        }
        ty
    }
    let mut sources = Vec::new();
    let mut destinations = Vec::new();
    for (index, ty) in params.iter().enumerate() {
        let mut input_paths = Vec::new();
        paths(ty, &mut Vec::new(), &mut input_paths);
        for path in input_paths {
            sources.push(BorrowSource {
                argument: index as u32,
                path: path.clone(),
                kind: BorrowSourceKind::Value,
            });
            let field = at(ty, &path);
            if matches!(field, MirErasedType::Reference { .. }) {
                sources.push(BorrowSource {
                    argument: index as u32,
                    path: path.clone(),
                    kind: BorrowSourceKind::Contents,
                });
            }
            if matches!(field, MirErasedType::Reference { mutable: true, .. })
                || (!(index == 0 && receiver.is_some())
                    && matches!(
                        field,
                        MirErasedType::Parameter(_)
                            | MirErasedType::Scoped(_)
                            | MirErasedType::Nominal { .. }
                            | MirErasedType::Apply { .. }
                            | MirErasedType::Concrete(
                                Type::Struct { .. }
                                    | Type::Enum { .. }
                                    | Type::Object(_)
                                    | Type::Exists(_)
                                    | Type::Function { .. }
                            )
                    ))
            {
                destinations.push(BorrowSource {
                    argument: index as u32,
                    path,
                    kind: BorrowSourceKind::Contents,
                });
            }
        }
        if index == 0 && matches!(receiver, Some(ReceiverMode::Mut | ReceiverMode::Move)) {
            destinations.push(BorrowSource {
                argument: index as u32,
                path: Vec::new(),
                kind: BorrowSourceKind::Contents,
            });
        }
    }
    // Receiver storage is only borrowed for shared/mutable calls; a consumed
    // closure can return its contained references, never a reference to its frame.
    if receiver == Some(ReceiverMode::Move) {
        sources.retain(|source| source.argument != 0 || source.kind == BorrowSourceKind::Value);
    }
    let mut output_paths = Vec::new();
    paths(ret, &mut Vec::new(), &mut output_paths);
    BorrowContract {
        returns: output_paths
            .into_iter()
            .map(|path| BorrowReturn {
                path,
                sources: sources.clone(),
            })
            .collect(),
        retentions: destinations
            .into_iter()
            .map(|destination| BorrowRetention {
                sources: sources
                    .iter()
                    .filter(|source| {
                        source.argument != destination.argument
                            || source.path != destination.path
                            || (!(destination.argument == 0 && receiver.is_some())
                                && !matches!(
                                    at(&params[destination.argument as usize], &destination.path),
                                    MirErasedType::Reference { .. }
                                ))
                    })
                    .cloned()
                    .collect(),
                destination,
            })
            .collect(),
    }
}

fn borrow_shape(ty: &Type) -> MirErasedType<Type> {
    match ty {
        Type::Reference { inner, mutable } => MirErasedType::Reference {
            inner: Box::new(borrow_shape(inner)),
            mutable: *mutable,
        },
        Type::Pointer(inner) => MirErasedType::Pointer(Box::new(borrow_shape(inner))),
        Type::Tuple(fields) => MirErasedType::Tuple(fields.iter().map(borrow_shape).collect()),
        Type::Array(element, len) => MirErasedType::Array {
            element: Box::new(borrow_shape(element)),
            len: *len,
        },
        _ => MirErasedType::Concrete(ty.clone()),
    }
}

fn expand_borrow_leaves(ty: MirErasedType<Type>) -> MirErasedType<Type> {
    match ty {
        MirErasedType::Concrete(ty) => borrow_shape(&ty),
        MirErasedType::Reference { inner, mutable } => MirErasedType::Reference {
            inner: Box::new(expand_borrow_leaves(*inner)),
            mutable,
        },
        MirErasedType::Tuple(fields) => {
            MirErasedType::Tuple(fields.into_iter().map(expand_borrow_leaves).collect())
        }
        MirErasedType::Array { element, len } => MirErasedType::Array {
            element: Box::new(expand_borrow_leaves(*element)),
            len,
        },
        ty => ty,
    }
}

pub(crate) fn erased_borrow_contract_types(
    params: &[MirErasedType<Type>],
    ret: &MirErasedType<Type>,
    receiver: Option<ReceiverMode>,
) -> crate::hir::borrowing::BorrowContract {
    let params = params
        .iter()
        .cloned()
        .map(expand_borrow_leaves)
        .collect::<Vec<_>>();
    let ret = expand_borrow_leaves(ret.clone());
    derive_erased_borrow_contract(&params, &ret, receiver)
}

pub(crate) fn erased_borrow_contract(
    params: &[MirErasedType],
    ret: &MirErasedType,
    receiver: Option<ReceiverMode>,
    tc: &crate::type_context::TypeContext,
) -> Result<crate::hir::borrowing::BorrowContract, MirErasedError> {
    let params = params
        .iter()
        .map(|ty| {
            ty.map_types(&mut |id| {
                if !tc.type_id_tree_is_valid(*id) {
                    return Err(MirErasedError::InvalidType);
                }
                Ok(tc.type_for(*id))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let ret = ret.map_types(&mut |id| {
        if !tc.type_id_tree_is_valid(*id) {
            return Err(MirErasedError::InvalidType);
        }
        Ok(tc.type_for(*id))
    })?;
    Ok(erased_borrow_contract_types(&params, &ret, receiver))
}

/// A shared/mutable callback owned by this frame cannot lend its storage to an
/// escaping result. Keep that lifetime obligation at concrete instantiation;
/// borrowed receivers and consuming value transport do not need this restriction.
pub(crate) fn erased_callback_storage_requirements<T: Clone + Ord>(
    params: &[MirErasedType<T>],
    dictionaries: &[MirErasedDictionarySchema<T>],
) -> BTreeSet<MirErasedType<T>> {
    dictionaries
        .iter()
        .filter(|dictionary| params.contains(&dictionary.subject))
        .flat_map(|dictionary| &dictionary.members)
        .filter(|member| {
            member
                .receiver
                .is_some_and(|mode| mode != ReceiverMode::Move)
                && member.binder.parameters.is_empty()
        })
        .map(|member| member.ret.clone())
        .collect()
}

fn reference_slots(
    ty: &Type,
    contract: &super::MirBackendContract,
    tc: &crate::type_context::TypeContext,
) -> bool {
    match ty {
        // An empty signature capture list is not exact environment authority:
        // callable coercion/indirection can conceal a supplied environment.
        Type::Function { .. } => true,
        Type::Reference { .. } => true,
        Type::Pointer(_) => false,
        Type::Tuple(fields) => fields
            .iter()
            .any(|field| reference_slots(field, contract, tc)),
        Type::Array(element, len) => *len != 0 && reference_slots(element, contract, tc),
        _ => tc.id_for_type(ty).map_or(true, |id| {
            super::borrowck::liveness::type_id_contains_reference_with_contract(
                crate::type_context::TypeView::new(tc),
                contract,
                id,
            )
        }),
    }
}

fn writable_reference_slots(
    ty: &Type,
    contract: &super::MirBackendContract,
    tc: &crate::type_context::TypeContext,
) -> bool {
    match ty {
        Type::Reference {
            mutable: true,
            inner,
        } => reference_slots(inner, contract, tc),
        Type::Reference { mutable: false, .. } | Type::Pointer(_) => false,
        Type::Tuple(fields) => fields
            .iter()
            .any(|field| writable_reference_slots(field, contract, tc)),
        Type::Array(element, len) => *len != 0 && writable_reference_slots(element, contract, tc),
        // A nominal/hidden value can contain a writable borrowed destination.
        // Without a field-level envelope proof, keep its incoming loans restricted.
        _ => reference_slots(ty, contract, tc),
    }
}

fn borrow_path_type<'a>(
    mut ty: &'a Type,
    path: &crate::hir::borrowing::BorrowPath,
) -> Option<&'a Type> {
    use crate::hir::borrowing::BorrowProjection;
    for projection in path {
        ty = match (projection, ty) {
            (BorrowProjection::Field(index), Type::Tuple(fields)) => fields.get(*index as usize)?,
            (BorrowProjection::Element, Type::Array(element, _)) => element,
            (BorrowProjection::Deref, Type::Reference { inner, .. }) => inner,
            _ => return None,
        };
    }
    Some(ty)
}

fn validate_concrete_borrow_effects(
    signature: &MirErasedSignature,
    bindings: &[Option<Type>],
    contract: &super::MirBackendContract,
    tc: &crate::type_context::TypeContext,
) -> Result<(), MirErasedError> {
    validate_concrete_borrow_effects_with_proof(signature, bindings, contract, tc, None)
}

fn validate_concrete_borrow_effects_with_proof(
    signature: &MirErasedSignature,
    bindings: &[Option<Type>],
    contract: &super::MirBackendContract,
    tc: &crate::type_context::TypeContext,
    proof: Option<&native_retention::NativeRetentionProof>,
) -> Result<(), MirErasedError> {
    for (dictionary_index, dictionary) in signature.dictionaries.iter().enumerate() {
        for (member_index, member) in dictionary.members.iter().enumerate() {
            if !member.binder.parameters.is_empty() {
                // Member-local forwarding has a checked input-origin envelope, not
                // a native closure storage seal or authority for opaque retention.
                if !member.borrowing.retentions.is_empty() {
                    return Err(MirErasedError::UnsupportedOperation);
                }
                if member.borrowing
                    != super::dictionary_member_borrow_contract(
                        &member.binder,
                        &member.params,
                        &member.ret,
                        member.receiver,
                        tc,
                    )?
                {
                    return Err(MirErasedError::InvalidDictionary);
                }
            }
            for effect in &member.borrowing.retentions {
                let index = effect.destination.argument as usize;
                if index == 0
                    && member
                        .receiver
                        .is_some_and(|mode| mode != ReceiverMode::Shared)
                {
                    let subject = &member.params[0];
                    let mut found = false;
                    let mut writable = false;
                    for parameter in &signature.params {
                        if parameter == subject {
                            found = true;
                            writable = true;
                        } else if let MirErasedType::Reference { inner, mutable } = parameter {
                            if inner.as_ref() == subject {
                                found = true;
                                writable |= *mutable && member.receiver == Some(ReceiverMode::Mut);
                            }
                        }
                    }
                    // Implied dictionaries do not upgrade a shared formal parameter
                    // to mutable/consuming access. The body validator enforces access.
                    if found && !writable {
                        continue;
                    }
                }
                let destination = member
                    .params
                    .get(index)
                    .ok_or(MirErasedError::InvalidDictionary)?
                    .instantiate_optional(bindings, tc)?;
                let destination = borrow_path_type(&destination, &effect.destination.path)
                    .ok_or(MirErasedError::InvalidDictionary)?;
                let can_retain = if index == 0
                    && effect.destination.path.is_empty()
                    && member
                        .receiver
                        .is_some_and(|mode| mode != ReceiverMode::Shared)
                {
                    reference_slots(destination, contract, tc)
                        && !proof.is_some_and(|proof| {
                            proof.covers_subject(
                                signature,
                                dictionary_index,
                                member_index,
                                destination,
                                contract,
                                tc,
                            )
                        })
                } else {
                    writable_reference_slots(destination, contract, tc)
                };
                if !can_retain {
                    continue;
                }
                for source in &effect.sources {
                    let source_ty = member
                        .params
                        .get(source.argument as usize)
                        .ok_or(MirErasedError::InvalidDictionary)?
                        .instantiate_optional(bindings, tc)?;
                    let source_ty = borrow_path_type(&source_ty, &source.path)
                        .ok_or(MirErasedError::InvalidDictionary)?;
                    if reference_slots(source_ty, contract, tc) {
                        // No concrete body or ABI row proves this input outlives the
                        // mutable destination envelope. Reject, including unit returns.
                        return Err(MirErasedError::BorrowEscape);
                    }
                }
            }
        }
    }
    Ok(())
}

impl MirErasedDictionarySchema {
    pub(crate) fn substitute(
        &self,
        bindings: &[MirErasedType],
        tc: &crate::type_context::TypeContext,
    ) -> Result<Self, MirErasedError> {
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
                    let params = member
                        .params
                        .iter()
                        .map(|ty| ty.substitute(bindings))
                        .collect::<Result<Vec<_>, _>>()?;
                    let ret = member.ret.substitute(bindings)?;
                    let borrowing = super::dictionary_member_borrow_contract(
                        &member.binder,
                        &params,
                        &ret,
                        member.receiver,
                        tc,
                    )?;
                    Ok(MirErasedDictionaryMember {
                        member_id: member.member_id,
                        binder: member.binder.clone(),
                        receiver: member.receiver,
                        params,
                        ret,
                        borrowing,
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
            Self::Scoped(index) => MirErasedType::Scoped(*index),
            Self::MemberParameter(index) => MirErasedType::MemberParameter(*index),
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
            Self::Concrete(_) | Self::Parameter(_) | Self::Scoped(_) | Self::MemberParameter(_) => {
            }
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
                                    binder: member.binder.map_types(map)?,
                                    receiver: member.receiver,
                                    params: member
                                        .params
                                        .iter()
                                        .map(|ty| ty.map_types(map))
                                        .collect::<Result<_, _>>()?,
                                    ret: member.ret.map_types(map)?,
                                    borrowing: member.borrowing.clone(),
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
    NominalField {
        owner: DefId,
        field: crate::ids::FieldId,
    },
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
    OwnerOpen {
        scope: u32,
        source: MirErasedOperand,
        plan: MirErasedOwnerOpening,
    },
    OwnerCall {
        scope: u32,
        owner: MirErasedOperand,
        split: MirErasedOwnerOperation,
        release: MirErasedOwnerOperation,
        slot: MirErasedOwnedSlot,
        args: Vec<MirErasedOperand>,
    },
    Forget(MirErasedOperand),
    DropInPlace { pointer: MirErasedOperand, pointee: MirErasedType },
    PackageOpen {
        scope: u32,
        source: MirErasedOperand,
    },
    PackagePack {
        payload: MirErasedOperand,
        plan: super::MirPackagePackPlan,
    },
    Use(MirErasedOperand),
    Literal(MirErasedLiteral),
    Aggregate(Vec<MirErasedOperand>),
    NominalAggregate {
        ty: MirErasedType,
        fields: Vec<(crate::ids::FieldId, MirErasedOperand)>,
    },
    NativeAggregate {
        ty: TypeId,
        fields: Vec<MirErasedOperand>,
    },
    /// A scoped address, not an implicit owned allocation or metadata value.
    Borrow {
        place: MirErasedPlace,
        mutable: bool,
    },
    Open {
        scope: u32,
        source: MirErasedOperand,
    },
    /// Uses the table retained by Open; no implementation search or forged metadata.
    Reclose {
        scope: u32,
        value: MirErasedOperand,
        object: TypeId,
    },
    OpenedCall {
        scope: u32,
        slot: u32,
        receiver: MirErasedPlace,
        args: Vec<MirErasedOperand>,
    },
    /// An erased producer slot instantiated by a dominating scoped descriptor.
    OpenedErasedCall {
        scope: u32,
        slot: u32,
        type_args: Vec<MirErasedType>,
        dictionaries: Vec<u32>,
        receiver: MirErasedPlace,
        args: Vec<MirErasedOperand>,
    },
    /// Exact concrete callable selected while producing a native erased bridge.
    NativeCall {
        target: MirCallableKey,
        args: Vec<MirErasedOperand>,
    },
    /// An ordinary concrete call from a scoped body, never a native erased
    /// bridge or evidence that permits specializing a runtime witness.
    NativeApply {
        target: MirCallableKey,
        args: Vec<MirErasedOperand>,
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
        type_args: Vec<MirErasedType>,
        dictionaries: Vec<u32>,
        receiver: Option<MirErasedPlace>,
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
    pub scopes: Vec<MirErasedScope>,
    /// Entry is block zero. Every edge carries initialization, move and loan state.
    pub blocks: Vec<MirErasedBasicBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirErasedScope {
    pub witness: crate::types::WitnessId,
    pub object: TypeId,
    pub source: MirErasedScopeSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirErasedScopeSource {
    Object { mutable: bool },
    Package { package: TypeId },
    Owner { opening: MirErasedOwnerOpening },
}

/// Selected protocol operations keep declaration identity independently of the ABI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirErasedOwnerOperation {
    pub constructor: TypeId,
    pub implementation: DefId,
    pub trait_id: DefId,
    pub member: DefId,
    pub definition: DefId,
    pub target: MirErasedOwnerTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirErasedOwnerTarget {
    Native(MirCallableKey),
    Erased { key: MirErasedKey, type_args: Vec<MirErasedType> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirErasedOwnerOpening {
    pub closed: TypeId,
    pub opened: MirErasedType,
    pub state: MirErasedType,
    pub split: MirErasedOwnerOperation,
    pub rebuild: MirErasedOwnerOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirErasedOwnedSlot {
    Concrete(u32),
    Erased { slot: u32, type_args: Vec<MirErasedType>, dictionaries: Vec<u32> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirOwnerProtocolAuthority {
    pub transport: DefId,
    pub split: DefId,
    pub rebuild: DefId,
    pub unique: Option<(DefId, DefId)>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MirScopedEntryKey {
    pub definition: DefId,
    pub params: Vec<TypeId>,
    pub ret: TypeId,
}

impl MirErasedScope {
    /// Package access is checked against the actual place; no package-wide
    /// mutability permission is manufactured from witness evidence.
    fn allows_mutable_place(&self) -> bool {
        match self.source {
            MirErasedScopeSource::Object { mutable } => mutable,
            MirErasedScopeSource::Package { .. } => true,
            MirErasedScopeSource::Owner { .. } => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MirErasedBlock(pub u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirErasedBasicBlock {
    pub statements: Vec<MirErasedAssignment>,
    pub terminator: MirErasedTerminator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirErasedTerminator {
    Goto(MirErasedBlock),
    /// End local storage after conditional destruction; moved locals stay dead.
    Cleanup {
        locals: Vec<MirErasedLocal>,
        target: MirErasedBlock,
    },
    Branch {
        condition: MirErasedOperand,
        then_block: MirErasedBlock,
        else_block: MirErasedBlock,
    },
    Return(MirErasedOperand),
    Unreachable,
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
    pub receiver: Option<ReceiverMode>,
    pub target: super::MirDictionaryMemberTarget,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MirErasedContract {
    /// Only a selected owner rebuild may carry payload ownership through a raw
    /// pointer input; ordinary pointer calls acquire no loan-return permission.
    pub owner_rebuild_entries: BTreeMap<MirErasedKey, (DefId, DefId)>,
    pub move_paths: BTreeMap<MirErasedKey, super::erased_moves::MirErasedMovePaths>,
    pub nominal_drop_entries: BTreeMap<DefId, super::MirNominalDropSelection>,
    pub imported_member_links: BTreeMap<MirErasedKey, (String, super::MirErasedMemberEntry)>,
    pub member_entries: BTreeMap<MirErasedKey, super::MirErasedMemberEntry>,
    pub dictionary_templates: BTreeMap<DefId, super::MirDictionaryFactoryTemplate>,
    pub dictionary_factories: BTreeMap<MirErasedKey, Vec<super::MirDictionaryFactoryRequirement>>,
    pub dictionary_witnesses: BTreeMap<MirDictionaryId, super::MirDictionaryWitness>,
    pub nominal_factories: BTreeMap<MirErasedKey, Vec<super::MirNominalFactoryRequirement>>,
    pub tuple_factories: BTreeMap<MirErasedKey, Vec<super::MirTupleFactoryRequirement>>,
    pub thin_handle_factories: BTreeMap<MirErasedKey, Vec<super::MirThinHandleFactoryRequirement>>,
    pub scoped_factories: BTreeMap<MirErasedKey, BTreeMap<u32, Vec<super::MirBodyFactoryRequirement>>>,
    /// Diagnostic provenance only; never type, layout, or loan authority.
    pub statement_origins: BTreeMap<MirErasedKey, BTreeMap<(MirErasedBlock, usize), crate::lexer::Span>>,
    pub imported_runtime_support: BTreeSet<crate::products::ProductRuntimeSupport>,
    /// Session remapping to actual defining product identity; never a runtime key.
    pub descriptor_origins: BTreeMap<crate::ids::CrateId, crate::products::ProductCrateIdentity>,
    pub functions: BTreeMap<MirErasedKey, MirErasedFunction>,
    pub descriptors: BTreeMap<TypeId, MirTypeDescriptor>,
    pub dictionaries: BTreeMap<MirDictionaryId, MirConcreteDictionary>,
    pub constructor_kinds: BTreeMap<DefId, Kind>,
    /// Concrete wrappers delegate their whole body, including early exits, to this entry.
    pub scoped_entries: BTreeMap<MirScopedEntryKey, MirErasedKey>,
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
    for key in contract.erased.owner_rebuild_entries.keys() {
        if let Err(error) = super::owner_open::validate_rebuild_entry(key, contract, tc) {
            errors.push((Some(key.clone()), error));
        }
    }
    for key in contract.erased.move_paths.keys() {
        if !contract.erased.functions.contains_key(key) {
            errors.push((Some(key.clone()), MirErasedError::MissingCallable));
        }
    }
    for (owner, authority) in &contract.nominal_drop_authority {
        let super::MirNominalDropAuthority::Selected(selection) = authority else { continue; };
        let Some(super::MirNominalLayout::Struct { generic_params, .. }) = contract.nominal_layouts.get(owner) else {
            errors.push((Some(selection.entry.clone()), MirErasedError::MissingLayout));
            continue;
        };
        let subject = MirErasedType::Nominal {
            id: *owner, flavor: NominalTypeKind::Struct,
            args: (0..generic_params.len()).map(|index| MirErasedType::Parameter(index as u32)).collect(),
        };
        if let Err(error) = super::nominal_requirement(contract, tc, &subject) {
            errors.push((Some(selection.entry.clone()), error));
        }
        if !selection.entry.static_args.is_empty()
            || contract.erased.functions.get(&selection.entry).is_none_or(|entry| entry.body.is_none())
        {
            errors.push((Some(selection.entry.clone()), MirErasedError::MissingCallable));
        }
    }
    for (owner, selection) in &contract.erased.nominal_drop_entries {
        if contract.nominal_drop_authority.get(owner)
            != Some(&super::MirNominalDropAuthority::Selected(selection.clone()))
            || !contract.dictionary_impl_members.get(&selection.implementation).is_some_and(|authority| {
                authority.members.get(&selection.member) == Some(&selection.entry.definition)
            })
            || !contract.erased.functions.contains_key(&selection.entry)
        {
            errors.push((Some(selection.entry.clone()), MirErasedError::MissingCallable));
        }
    }
    if contract
        .erased
        .imported_runtime_support
        .iter()
        .any(|support| support.validate().is_err())
    {
        errors.push((None, MirErasedError::InvalidType));
    }
    for (key, requirements) in &contract.erased.tuple_factories {
        if !contract.erased.functions.contains_key(key) {
            errors.push((Some(key.clone()), MirErasedError::MissingCallable));
        }
        if requirements.is_empty() {
            errors.push((Some(key.clone()), MirErasedError::InvalidType));
        }
    }
    for (key, requirements) in &contract.erased.nominal_factories {
        if !contract.erased.functions.contains_key(key) {
            errors.push((Some(key.clone()), MirErasedError::MissingCallable));
        }
        if requirements.is_empty() {
            errors.push((Some(key.clone()), MirErasedError::InvalidType));
        }
    }
    for (key, requirements) in &contract.erased.thin_handle_factories {
        if !contract.erased.functions.contains_key(key) {
            errors.push((Some(key.clone()), MirErasedError::MissingCallable));
        }
        if requirements.is_empty() {
            errors.push((Some(key.clone()), MirErasedError::InvalidType));
        }
    }
    for (key, requirements) in &contract.erased.scoped_factories {
        if !contract.erased.functions.contains_key(key) {
            errors.push((Some(key.clone()), MirErasedError::MissingCallable));
        }
        if requirements.is_empty() {
            errors.push((Some(key.clone()), MirErasedError::InvalidType));
        }
    }
    for (key, requirements) in &contract.erased.dictionary_factories {
        if !contract.erased.functions.contains_key(key) {
            errors.push((Some(key.clone()), MirErasedError::MissingCallable));
        }
        if requirements.is_empty() {
            errors.push((Some(key.clone()), MirErasedError::InvalidDictionary));
        }
    }
    for (id, template) in &contract.erased.dictionary_templates {
        if template.implementation != *id
            || !contract
                .erased
                .dictionary_factories
                .values()
                .flatten()
                .any(|r| r.implementation == *id)
            || super::dictionary_template_key(contract, tc, template).is_err()
        {
            errors.push((None, MirErasedError::InvalidDictionary));
        }
    }
    for key in contract.erased.member_entries.keys() {
        if let Err(error) = super::validate_member_entry(key, contract, tc) {
            errors.push((Some(key.clone()), error));
        }
    }
    for (key, (symbol, authority)) in &contract.erased.imported_member_links {
        if contract.erased.member_entries.get(key) != Some(authority)
            || contract.erased.functions.get(key).is_none_or(|entry| {
                entry.body.is_some()
                    || entry.linkage != super::MirLinkage::External
                    || &entry.symbol != symbol
            })
        {
            errors.push((Some(key.clone()), MirErasedError::InvalidDictionary));
        }
    }
    if !super::descriptor_identity::validate_descriptor_identities(contract, tc) {
        errors.push((None, MirErasedError::InvalidType));
    }
    for package in contract.packages.keys() {
        if !super::package_schema_is_valid(contract, tc, *package) {
            errors.push((None, MirErasedError::InvalidType));
        }
    }
    for function in program.functions.values() {
        for statement in function
            .basic_blocks
            .iter()
            .flat_map(|block| &block.statements)
        {
            if let super::StatementKind::Assign(destination, value) = &statement.kind {
                if let Some(package) = contract
                    .place_type_id(tc, function, destination)
                    .filter(|id| matches!(tc.try_ty(*id), Some(crate::type_context::Ty::Exists(_))))
                {
                    if !matches!(value, super::Rvalue::Package(_, _)) {
                        let valid = match value {
                            super::Rvalue::Use(
                                super::Operand::Move(place) | super::Operand::Copy(place),
                            ) => contract.place_type_id(tc, function, place) == Some(package),
                            _ => false,
                        };
                        if !valid {
                            errors.push((None, MirErasedError::InvalidType));
                        }
                    }
                }
            }
            let super::StatementKind::Assign(destination, super::Rvalue::Package(source, plan)) =
                &statement.kind
            else {
                continue;
            };
            let source_type = match source {
                super::Operand::Move(place) | super::Operand::Copy(place) => {
                    contract.place_type_id(tc, function, place)
                }
                _ => None,
            };
            let valid = super::package_plan_is_valid(contract, tc, plan, &[], &[])
                && contract.place_type_id(tc, function, destination) == Some(plan.package)
                && match plan.witness {
                    super::MirPackageWitnessEvidence::Concrete { witness, .. } => {
                        super::package_payload_type(plan.package, &tc.type_for(witness), tc)
                            .is_ok_and(|payload| source_type == tc.id_for_type(&payload))
                    }
                    _ => false,
                };
            if !valid {
                errors.push((None, MirErasedError::InvalidType));
            }
        }
    }
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
    for (id, dictionary) in &contract.erased.dictionaries {
        if dictionary.members.iter().any(|member| {
            member.receiver.is_none()
                || matches!(
                    member.target,
                    super::MirDictionaryMemberTarget::Polymorphic(_)
                )
        }) && super::native_dictionary_keys(contract, tc, *id).is_err()
        {
            errors.push((None, MirErasedError::InvalidDictionary));
        }
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
            if let super::MirDictionaryMemberTarget::Polymorphic(key) = &member.target {
                let valid = contract
                    .erased
                    .member_entries
                    .get(key)
                    .is_some_and(|entry| {
                        entry.trait_id == dictionary.trait_id
                            && entry.member.member_id == member.member_id
                            && entry.member.receiver == member.receiver
                            && entry.member.params.first()
                                == Some(&MirErasedType::Concrete(dictionary.subject))
                            && super::validate_member_entry(key, contract, tc).is_ok()
                    });
                if !members.insert(member.member_id) || !valid {
                    errors.push((None, MirErasedError::InvalidDictionary));
                }
                continue;
            }
            let Some(target) = member.target.native() else {
                errors.push((None, MirErasedError::InvalidDictionary));
                continue;
            };
            if let MirCallableKey::ObjectAdapter(super::MirObjectAdapterKey::NativeCallable {
                concrete,
                kind,
            }) = target
            {
                let native_valid = contract.callable(target).is_some_and(|callee| {
                    let crate::types::Type::Function { params, ret, callable_kind, safety, .. } = &subject else { return false };
                    let packed = match params.as_slice() { [] => crate::types::Type::Unit, [param] => param.clone(), _ => crate::types::Type::Tuple(params.clone()) };
                    let receiver = match kind { crate::types::CallableKind::Fn => ReceiverMode::Shared, crate::types::CallableKind::FnMut => ReceiverMode::Mut, crate::types::CallableKind::FnOnce => ReceiverMode::Move };
                    *concrete == dictionary.subject && *callable_kind <= *kind && safety.permits_call_without_unsafe() && member.receiver == Some(receiver)
                        && dictionary.trait_args.iter().map(|id| tc.type_for(*id)).collect::<Vec<_>>() == vec![packed.clone(), *ret.clone()]
                        && callee.signature.params.len() == 2 && tc.type_id_tree_is_valid(callee.signature.params[1].semantic_ty) && tc.type_for(callee.signature.params[1].semantic_ty) == packed
                        && tc.type_id_tree_is_valid(callee.signature.ret.semantic_ty) && tc.type_for(callee.signature.ret.semantic_ty) == **ret
                        && matches!(&callee.kind, super::MirCallableKind::ObjectAdapter(super::MirObjectAdapter::NativeCallable { concrete: source, kind: mode, trait_id, member_id, .. }) if source == concrete && mode == kind && *trait_id == dictionary.trait_id && *member_id == member.member_id)
                });
                if !native_valid {
                    errors.push((None, MirErasedError::InvalidDictionary));
                }
            }
            let valid = contract.callable(target).is_some_and(|callee| {
                let expected = member.receiver.map(|receiver| match receiver {
                    ReceiverMode::Shared | ReceiverMode::Mut => crate::types::Type::Reference {
                        inner: Box::new(subject.clone()),
                        mutable: receiver == ReceiverMode::Mut,
                    },
                    ReceiverMode::Move
                        if matches!(callee.kind, super::MirCallableKind::ObjectAdapter(_)) =>
                    {
                        crate::types::Type::Pointer(Box::new(subject.clone()))
                    }
                    ReceiverMode::Move => subject.clone(),
                });
                callee
                    .signature
                    .params
                    .iter()
                    .all(|p| tc.type_id_tree_is_valid(p.semantic_ty))
                    && tc.type_id_tree_is_valid(callee.signature.ret.semantic_ty)
                    && tc.type_id_tree_is_valid(callee.signature.ret.abi_ty)
                    && expected.is_none_or(|expected| {
                        callee
                            .signature
                            .params
                            .first()
                            .is_some_and(|p| tc.type_for(p.semantic_ty) == expected)
                    })
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
        if let Some(body) = &function.body {
            for statement in body.blocks.iter().flat_map(|block| &block.statements) {
                let (target, bridge) = match &statement.value {
                    MirErasedRvalue::NativeCall { target, .. } => (target, true),
                    MirErasedRvalue::NativeApply { target, .. } => (target, false),
                    _ => continue,
                };
                let valid = contract.callable(target).is_some_and(|declaration| {
                    let params = declaration
                        .signature
                        .params
                        .iter()
                        .map(|param| param.semantic_ty)
                        .collect::<Vec<_>>();
                    if params.iter().chain(std::iter::once(&declaration.signature.ret.semantic_ty))
                        .any(|id| !tc.type_id_tree_is_valid(*id))
                        || ![false, true].into_iter().any(|is_method| {
                            declaration.signature
                                == super::MirCallableSignature::from_instance_type_ids(
                                    &params,
                                    declaration.signature.ret.semantic_ty,
                                    is_method,
                                    tc,
                                )
                        })
                    {
                        return false;
                    }
                    if matches!(declaration.kind, super::MirCallableKind::ObjectProvided) {
                        // The provider's body is in its linked object, not this
                        // program. Require its exact canonical native link row.
                        let definition = if bridge { Some(key.definition) } else { declaration.source_def_id };
                        return definition.is_some_and(|definition| imported_native_link_is_valid(definition, target, declaration, contract))
                            && matches!(target, super::MirCallableKey::Instance(id)
                                if !program.functions.contains_key(&super::MirFunctionId::Instance(*id)));
                    }
                    let super::MirCallableKind::LocalBody { function_id: id } = &declaration.kind
                    else {
                        return false;
                    };
                    let Some(native) = program.functions.get(id) else {
                        return false;
                    };
                    if contract.function_bodies.get(id) != Some(target)
                        || native.arg_count != declaration.signature.params.len()
                        || native
                            .local_decls
                            .get(1..1 + native.arg_count)
                            .map(|locals| locals.iter().map(|local| local.ty).collect::<Vec<_>>())
                            != Some(
                                declaration
                                    .signature
                                    .params
                                    .iter()
                                    .map(|param| param.semantic_ty)
                                    .collect(),
                            )
                        || native.ret_type != declaration.signature.ret.semantic_ty
                    {
                        return false;
                    }
                    true
                });
                if !valid {
                    errors.push((Some(key.clone()), MirErasedError::MissingCallable));
                }
            }
        }
    }
    for function in program.functions.values() {
        for (block_index, block) in function.basic_blocks.iter().enumerate() {
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
                        | super::Rvalue::Package(op, _)
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
                        if let Err(error) = validate_erased_call(
                            program,
                            function,
                            call,
                            args,
                            destination,
                            block_index,
                        ) {
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
    if native_bridge_source(function).is_some_and(|source| source != concrete) {
        return false;
    }
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
            .map(|dictionary| dictionary.substitute(&bindings, tc))
            .collect::<Result<Vec<_>, _>>()
            .is_ok_and(|dictionaries| dictionaries == function.signature.dictionaries)
}

fn validate_erased_call(
    program: &super::MirProgram,
    function: &super::MirFunction,
    call: &MirErasedCall,
    args: &[super::Operand],
    destination: &super::Place,
    block: usize,
) -> Result<(), MirErasedError> {
    let proof = native_retention::NativeRetentionProof::for_call(program, function, block);
    validate_erased_call_inner(
        program,
        function,
        call,
        args,
        destination,
        None,
        Some(&proof),
    )
}

/// Unsupported retention is a source borrowing failure, not an agreement
/// invariant. Use exactly the same checked entry path as strict MIR agreement.
pub(crate) fn native_retention_diagnostics(
    program: &super::MirProgram,
) -> crate::diagnostic::Diagnostics {
    let mut diagnostics = crate::diagnostic::Diagnostics::default();
    for function in program.functions.values() {
        for (index, block) in function.basic_blocks.iter().enumerate() {
            let Some(
                term @ super::Terminator::Call {
                    func: super::Operand::Constant(super::Constant::ErasedCall(call)),
                    args,
                    destination,
                    ..
                },
            ) = &block.terminator
            else {
                continue;
            };
            if validate_erased_call(program, function, call, args, destination, index)
                != Err(MirErasedError::BorrowEscape)
            {
                continue;
            }
            let diagnostic = match term.origin().source_span().cloned() {
                Some(span) => super::borrowck::diagnostics::borrow_error(
                    "erased callback may retain an input borrow in an unproved destination environment", span,
                ),
                None => crate::diagnostic::Diagnostic::for_internal("erased retention failure is missing its call origin"),
            };
            diagnostics.push(diagnostic);
        }
    }
    diagnostics
}

pub(crate) fn erased_body_borrow_diagnostics(
    program: &super::MirProgram,
) -> crate::diagnostic::Diagnostics {
    let mut diagnostics = crate::diagnostic::Diagnostics::default();
    for (key, function) in &program.backend_contract.erased.functions {
        let Some(origins) = program.backend_contract.erased.statement_origins.get(key) else {
            continue;
        };
        let mut failure = None;
        let error = validate_erased_function_at(
            function, &program.backend_contract, &program.type_context, &mut failure,
        );
        let message = match error {
            Err(MirErasedError::BorrowConflict) => "borrow conflict on this value in an erased body",
            Err(MirErasedError::BorrowEscape) => "borrowed value escapes its erased-body lifetime",
            Err(MirErasedError::UninitializedOrMoved) => "use of an uninitialized or moved value in an erased body",
            _ => continue,
        };
        // Corrupt or provenance-free MIR still fails the strict agreement
        // boundary; do not invent a source location for it.
        if let Some(span) = failure.and_then(|site| origins.get(&site)) {
            diagnostics.push(super::borrowck::diagnostics::borrow_error(message, span.clone()));
        }
    }
    diagnostics
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
    validate_erased_call_inner(program, function, &call, args, destination, Some(key), None)
}

pub(crate) fn validate_erased_environment(
    contract: &super::MirBackendContract,
    tc: &crate::type_context::TypeContext,
    call: &MirErasedCall,
) -> Result<Vec<Option<crate::types::Type>>, MirErasedError> {
    validate_erased_environment_with_proof(contract, tc, call, None)
}

fn validate_erased_environment_with_proof(
    contract: &super::MirBackendContract,
    tc: &crate::type_context::TypeContext,
    call: &MirErasedCall,
    proof: Option<&native_retention::NativeRetentionProof>,
) -> Result<Vec<Option<crate::types::Type>>, MirErasedError> {
    let proof = proof.filter(|proof| proof.matches_call(call));
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
    if let Some(proof) = proof {
        validate_concrete_borrow_effects_with_proof(
            signature,
            &bindings,
            contract,
            tc,
            Some(proof),
        )?;
    } else {
        validate_concrete_borrow_effects(signature, &bindings, contract, tc)?;
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
        let expected_subject = required.subject.instantiate_optional(&bindings, tc)?;
        let actual_subject = tc.type_for(dictionary.subject);
        let canonical_static_subject = matches!(
            &expected_subject,
            crate::types::Type::StructRepresentation { .. }
                | crate::types::Type::EnumRepresentation { .. }
        ) && required
            .members
            .iter()
            .all(|member| member.receiver.is_none())
            && actual_subject == contract.normalize_type(tc, &expected_subject);
        if dictionary.trait_id != required.trait_id
            || (actual_subject != expected_subject && !canonical_static_subject)
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
        for (member_index, (entry, expected)) in
            dictionary.members.iter().zip(&required.members).enumerate()
        {
            let target = match &entry.target {
                super::MirDictionaryMemberTarget::Native(target) => {
                    if !expected.binder.parameters.is_empty() {
                        return Err(MirErasedError::InvalidDictionary);
                    }
                    target
                }
                super::MirDictionaryMemberTarget::Polymorphic(key) => {
                    if expected.binder.parameters.is_empty() {
                        return Err(MirErasedError::InvalidDictionary);
                    }
                    if !expected.borrowing.retentions.is_empty() {
                        return Err(MirErasedError::UnsupportedOperation);
                    }
                    super::validate_member_entry(key, contract, tc)?;
                    super::native_dictionary_keys(contract, tc, *id)?;
                    // Substitute only parents. Local U and its complete BQ01
                    // input-origin/access contract remain universally quantified.
                    let parents = bindings
                        .iter()
                        .enumerate()
                        .map(|(index, binding)| match binding {
                            Some(ty) => tc
                                .id_for_type(ty)
                                .map(MirErasedType::Concrete)
                                .ok_or(MirErasedError::MissingLayout),
                            None => Ok(MirErasedType::Parameter(index as u32)),
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let selected = required.substitute(&parents, tc)?;
                    let authority = &contract.erased.member_entries[key];
                    if selected.members.get(member_index) != Some(&authority.member) {
                        return Err(MirErasedError::InvalidDictionary);
                    }
                    let function = &contract.erased.functions[key];
                    if function.body.is_some() {
                        validate_erased_function(function, contract, tc)?;
                    }
                    // Bodyless entries require the exact typed producer link in
                    // validate_member_entry, never a fabricated native callable.
                    continue;
                }
            };
            let callee = contract
                .callable(target)
                .ok_or(MirErasedError::MissingCallable)?;
            if !tc.type_id_tree_is_valid(callee.signature.ret.semantic_ty)
                || !tc.type_id_tree_is_valid(contract.callable_result_storage(callee))
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
                || contract.normalize_type(tc, &tc.type_for(callee.signature.ret.semantic_ty))
                    != contract
                        .normalize_type(tc, &expected.ret.instantiate_optional(&bindings, tc)?)
                || tc.type_for(contract.callable_result_storage(callee))
                    != expected.ret.instantiate_optional(&bindings, tc)?
            {
                return Err(MirErasedError::InvalidDictionary);
            }
            for (actual, expected) in callee
                .signature
                .params
                .iter()
                .skip(usize::from(expected.receiver.is_some()))
                .zip(
                    expected
                        .params
                        .iter()
                        .skip(usize::from(expected.receiver.is_some())),
                )
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
    proof: Option<&native_retention::NativeRetentionProof>,
) -> Result<(), MirErasedError> {
    let contract = &program.backend_contract;
    let tc = &program.type_context;
    let signature = call.target.signature(contract)?;
    let bindings = validate_erased_environment_with_proof(contract, tc, call, proof)?;
    if let MirErasedCallTarget::Direct(key) = &call.target {
        let callee = contract
            .erased
            .functions
            .get(key)
            .ok_or(MirErasedError::MissingCallable)?;
        if native_bridge_source(callee)
            .is_some_and(|source| call.type_args.first() != Some(&source))
        {
            return Err(MirErasedError::InvalidType);
        }
    }
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
        Self::from_type_scoped(ty, parameters, witness, &BTreeMap::new(), tc, env)
    }

    pub fn from_type_scoped(
        ty: &crate::types::Type,
        parameters: &BTreeMap<GenericParamId, u32>,
        witness: Option<u32>,
        scopes: &BTreeMap<crate::types::WitnessId, u32>,
        tc: &mut crate::type_context::TypeContext,
        env: &crate::type_services::normalize::TypeNormalizationEnv,
    ) -> Result<Self, MirErasedError> {
        use crate::types::Type;
        if !crate::type_services::visit::type_any(ty, |ty| {
            matches!(ty, Type::Generic(_) | Type::Witness(_))
        }) && !crate::type_services::substitution::has_free_object_self(ty)
        {
            return Ok(Self::Concrete(
                tc.intern_normalized_type(ty, env)
                    .map_err(|_| MirErasedError::KindMismatch)?,
            ));
        }
        Ok(match ty {
            Type::Witness(id) => {
                Self::Scoped(*scopes.get(id).ok_or(MirErasedError::UnboundParameter)?)
            }
            Type::Generic(id) => {
                Self::Parameter(*parameters.get(id).ok_or(MirErasedError::UnboundParameter)?)
            }
            Type::ObjectSelf { depth: 0 } => {
                Self::Parameter(witness.ok_or(MirErasedError::UnboundParameter)?)
            }
            Type::Reference { inner, mutable } => Self::Reference {
                inner: Box::new(Self::from_type_scoped(
                    inner, parameters, witness, scopes, tc, env,
                )?),
                mutable: *mutable,
            },
            Type::Pointer(inner) => Self::Pointer(Box::new(Self::from_type_scoped(
                inner, parameters, witness, scopes, tc, env,
            )?)),
            Type::Tuple(fields) => Self::Tuple(
                fields
                    .iter()
                    .map(|ty| Self::from_type_scoped(ty, parameters, witness, scopes, tc, env))
                    .collect::<Result<_, _>>()?,
            ),
            Type::Array(element, len) => Self::Array {
                element: Box::new(Self::from_type_scoped(
                    element, parameters, witness, scopes, tc, env,
                )?),
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
                    .map(|ty| Self::from_type_scoped(ty, parameters, witness, scopes, tc, env))
                    .collect::<Result<_, _>>()?,
            },
            Type::Apply { constructor, args } => Self::Apply {
                constructor: Box::new(Self::from_type_scoped(
                    constructor,
                    parameters,
                    witness,
                    scopes,
                    tc,
                    env,
                )?),
                args: args
                    .iter()
                    .map(|ty| Self::from_type_scoped(ty, parameters, witness, scopes, tc, env))
                    .collect::<Result<_, _>>()?,
            },
            _ => return Err(MirErasedError::UnsupportedOperation),
        })
    }

    pub fn contains_parameter(&self, index: u32) -> bool {
        match self {
            Self::Parameter(parameter) => *parameter == index,
            Self::Scoped(_) | Self::MemberParameter(_) => false,
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

    pub fn scoped_binders(&self, binders: &mut BTreeSet<u32>) {
        match self {
            Self::Scoped(index) => {
                binders.insert(*index);
            }
            Self::Reference { inner, .. } | Self::Pointer(inner) => inner.scoped_binders(binders),
            Self::Tuple(fields) | Self::Nominal { args: fields, .. } => {
                for field in fields {
                    field.scoped_binders(binders);
                }
            }
            Self::Array { element, .. } => element.scoped_binders(binders),
            Self::Apply { constructor, args } => {
                constructor.scoped_binders(binders);
                for argument in args {
                    argument.scoped_binders(binders);
                }
            }
            Self::Concrete(_) | Self::Parameter(_) | Self::MemberParameter(_) => {}
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
            Self::Scoped(_) => Ok(Kind::Type),
            Self::MemberParameter(_) => Err(MirErasedError::UnboundParameter),
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
            Self::Scoped(_) | Self::MemberParameter(_) => {
                return Err(MirErasedError::UnboundParameter)
            }
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
            Self::Scoped(index) => Self::Scoped(*index),
            Self::MemberParameter(index) => Self::MemberParameter(*index),
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
            (MirErasedProjection::NominalField { owner, field }, _) => {
                super::nominal_field(contract, tc, &ty, *owner, *field)?.1
            }
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

pub(crate) fn copy_proven(ty: &MirErasedType, tc: &crate::type_context::TypeContext) -> bool {
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
            if !matches!(ty, Type::Exists(_))
                && crate::type_services::visit::type_any(&ty, |ty| {
                    matches!(
                        ty,
                        Type::Witness(_) | Type::Generic(_) | Type::TypeVar(_) | Type::Error
                    )
                })
                || crate::type_services::substitution::has_free_object_self(&ty)
            {
                return Err(MirErasedError::InvalidType);
            }
            if tc.kind(*id) == &Kind::Type
                && !super::object_runtime_type_is_valid(contract, tc, *id)
            {
                return Err(MirErasedError::InvalidType);
            }
        }
        MirErasedType::Parameter(_) | MirErasedType::Scoped(_) => {}
        MirErasedType::MemberParameter(_) => return Err(MirErasedError::UnboundParameter),
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
    let mut scoped = BTreeSet::new();
    for ty in
        signature
            .params
            .iter()
            .chain(std::iter::once(&signature.ret))
            .chain(&signature.layouts)
            .chain(&signature.require_borrow_free)
            .chain(signature.dictionaries.iter().flat_map(|dictionary| {
                std::iter::once(&dictionary.subject)
                    .chain(&dictionary.trait_args)
                    .chain(dictionary.members.iter().flat_map(|member| {
                        member.params.iter().chain(std::iter::once(&member.ret))
                    }))
            }))
    {
        ty.scoped_binders(&mut scoped);
    }
    if !scoped.is_empty() {
        return Err(MirErasedError::UnboundParameter);
    }
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
                || (member.receiver.is_some() && member.params.first() != Some(&dictionary.subject))
                || (member.receiver.is_none()
                    && (!member.binder.parameters.is_empty()
                        || !super::static_dictionary_shape_supported(
                            &member.params,
                            &member.ret,
                            &dictionary.subject,
                            tc,
                        )))
                || member.borrowing
                    != super::dictionary_member_borrow_contract(
                        &member.binder,
                        &member.params,
                        &member.ret,
                        member.receiver,
                        tc,
                    )?
            {
                return Err(MirErasedError::InvalidDictionary);
            }
            let nested = member.entry_signature(&signature.parameters)?;
            for ty in nested.params.iter().chain(std::iter::once(&nested.ret)) {
                leaves(ty, contract, tc)?;
                if ty.kind(&nested, contract, tc)? != Kind::Type {
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

/// Validate the bound environment and CFG value transfers before lowering.
pub fn validate_erased_function(
    function: &MirErasedFunction,
    contract: &super::MirBackendContract,
    tc: &crate::type_context::TypeContext,
) -> Result<(), MirErasedError> {
    validate_erased_function_at(function, contract, tc, &mut None)
}

fn validate_erased_function_at(
    function: &MirErasedFunction,
    contract: &super::MirBackendContract,
    tc: &crate::type_context::TypeContext,
    failure: &mut Option<(MirErasedBlock, usize)>,
) -> Result<(), MirErasedError> {
    use super::dataflow::analyses::init::{InitState, MoveInfo};
    use super::dataflow::analyses::loans::{LoanAnalysis, LoanKind, LoanState, LoanTable};
    use super::dataflow::Lattice;
    use super::origins::OriginSet;
    use super::{Local, ReferenceOrigin as Origin};
    use crate::types::Type;

    fn loan_kind(ty: &MirErasedType, tc: &crate::type_context::TypeContext) -> LoanKind {
        if erased_type_has_mutable_reference(ty, tc) {
            LoanKind::Mut
        } else {
            LoanKind::Shared
        }
    }

    let signature = &function.signature;
    let move_paths = super::erased_moves::validated_erased_move_paths(function, contract, tc)?;
    validate_erased_signature(signature, contract, tc)?;
    super::validate_scoped_factories(function, contract, tc)?;
    let mut factory_layouts = super::validate_body_factories(function, contract, tc)?;
    for handles in contract.erased.scoped_factories.get(&function.key).into_iter().flat_map(|scopes| scopes.values()) {
        factory_layouts.extend(handles.iter().map(|handle| handle.result().clone()));
    }
    let body_dictionaries =
        super::validate_dictionary_factories(function, contract, tc, &factory_layouts)?;
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
        let mut scoped = BTreeSet::new();
        ty.scoped_binders(&mut scoped);
        if scoped
            .iter()
            .any(|index| *index as usize >= body.scopes.len())
        {
            return Err(MirErasedError::UnboundParameter);
        }
        let scoped_layout = match ty {
            MirErasedType::Scoped(_) => true,
            _ => {
                super::erased_uniform_layout(contract, &mut tc.clone(), ty).is_ok()
                    || scoped.iter().any(|index| {
                        body.scopes.get(*index as usize).is_some_and(|scope| {
                            let MirErasedScopeSource::Package { package } = scope.source else {
                                return false;
                            };
                            super::scoped_package_payload(
                                package,
                                *index,
                                scope.witness,
                                &mut tc.clone(),
                            )
                            .is_ok_and(|payload| payload == *ty)
                        })
                    })
            }
        };
        if !scoped.is_empty() && !scoped_layout && !factory_layouts.contains(ty) {
            return Err(MirErasedError::MissingLayout);
        }
        if scoped.is_empty()
            && !signature.layouts.contains(ty)
            && !factory_layouts.contains(ty)
            && !matches!(ty, MirErasedType::Concrete(id) if contract.erased.descriptors.contains_key(id))
        {
            return Err(MirErasedError::MissingLayout);
        }
    }
    let mut witnesses = BTreeSet::new();
    for (index, scope) in body.scopes.iter().enumerate() {
        if scope.witness.owner != function.key.definition
            || !witnesses.insert(scope.witness)
            || !contract.object_schemas.contains_key(&scope.object)
        {
            return Err(MirErasedError::InvalidType);
        }
        if let MirErasedScopeSource::Package { package } = scope.source {
            if !super::package_schema_is_valid(contract, tc, package)
                || contract.packages[&package].object != scope.object
            {
                return Err(MirErasedError::InvalidType);
            }
        }
        if let MirErasedScopeSource::Owner { opening } = &scope.source {
            super::owner_open::validate_opening(opening, index as u32, scope, contract, tc)?;
        }
    }
    if body.blocks.is_empty() {
        return Err(MirErasedError::InvalidLocal);
    }
    let mut reachable = BTreeSet::new();
    let mut pending_blocks = vec![MirErasedBlock(0)];
    while let Some(index) = pending_blocks.pop() {
        if !reachable.insert(index) {
            continue;
        }
        let block = body
            .blocks
            .get(index.0 as usize)
            .ok_or(MirErasedError::InvalidLocal)?;
        match &block.terminator {
            MirErasedTerminator::Goto(target) | MirErasedTerminator::Cleanup { target, .. } => {
                pending_blocks.push(*target);
            }
            MirErasedTerminator::Branch {
                then_block,
                else_block,
                ..
            } => {
                pending_blocks.extend([*then_block, *else_block]);
            }
            MirErasedTerminator::Return(_) | MirErasedTerminator::Unreachable => {}
        }
    }
    if body.blocks.iter().enumerate().any(|(index, block)| {
        !reachable.contains(&MirErasedBlock(index as u32))
            && (!block.statements.is_empty()
                || block.terminator != MirErasedTerminator::Unreachable)
    }) {
        return Err(MirErasedError::InvalidLocal);
    }
    #[derive(Clone, PartialEq, Eq)]
    struct State {
        initialized: Vec<InitState>,
        possibly_live: Vec<bool>,
        origins: Vec<OriginSet>,
        borrows: LoanState,
        opened: Vec<bool>,
        payload_sources: Vec<Option<u32>>,
    }
    impl Lattice for State {
        fn join(&mut self, other: &Self) -> bool {
            let previous = self.clone();
            for (target, source) in self.initialized.iter_mut().zip(&other.initialized) {
                target.join(source);
            }
            for local in 0..self.possibly_live.len() {
                self.possibly_live[local] |= other.possibly_live[local];
                self.origins[local].join(&other.origins[local]);
                if self.payload_sources[local] != other.payload_sources[local] {
                    self.payload_sources[local] = None;
                }
            }
            self.borrows.join(&other.borrows);
            for (target, source) in self.opened.iter_mut().zip(&other.opened) {
                *target &= *source;
            }
            *self != previous
        }
    }
    let loan_table =
        LoanTable::root_envelopes(body.locals.len()).ok_or(MirErasedError::InvalidLocal)?;
    let mut entry = State {
        initialized: vec![InitState::Uninit; move_paths.places.len()],
        possibly_live: vec![false; body.locals.len()],
        origins: vec![OriginSet::new(); body.locals.len()],
        borrows: LoanState::new(loan_table.len()),
        opened: vec![false; body.scopes.len()],
        payload_sources: vec![None; body.locals.len()],
    };
    entry.initialized[..signature.params.len()].fill(InitState::Init);
    for index in 0..signature.params.len() {
        for path in move_paths.descendants(index) { entry.initialized[path] = InitState::Init; }
    }
    entry.possibly_live[..signature.params.len()].fill(true);
    for (index, origin) in entry
        .origins
        .iter_mut()
        .take(signature.params.len())
        .enumerate()
    {
        origin.insert(Origin::Param(Local(index)));
    }
    // Erased projections retain a conservative root envelope until their
    // descriptor supplies field-level evidence. Access compatibility is shared
    // with concrete MIR and applies to reads as well as receiver calls/borrows.
    fn access_conflicts(
        local: MirErasedLocal,
        access: LoanKind,
        borrows: &LoanState,
        live: &[bool],
        table: &LoanTable,
    ) -> bool {
        let mut active = borrows.clone();
        active.retain_owners(|owner| live.get(owner.0).copied().unwrap_or(false));
        LoanAnalysis::check_aliasing(
            &super::Place {
                local: Local(local.0 as usize),
                projection: vec![],
            },
            access,
            table,
            &active,
        )
        .is_err()
    }
    fn operand(
        op: &MirErasedOperand,
        function: &MirErasedFunction,
        contract: &super::MirBackendContract,
        tc: &crate::type_context::TypeContext,
        paths: &super::erased_moves::MirErasedMovePaths,
        initialized: &mut [InitState],
        possibly_live: &mut [bool],
        borrows: &LoanState,
        table: &LoanTable,
    ) -> Result<MirErasedType, MirErasedError> {
        let (place, moving) = match op {
            MirErasedOperand::Move(place) => (place, true),
            MirErasedOperand::Copy(place) => (place, false),
        };
        if moving && place.local.0 == 0 && contract.erased.nominal_drop_entries.values()
            .any(|selection| selection.entry == function.key)
        {
            return Err(MirErasedError::ProjectedMoveRequiresDropProof);
        }
        if !initialized
            .get(paths.containing(place))
            .is_some_and(InitState::is_initialized)
        {
            return Err(MirErasedError::UninitializedOrMoved);
        }
        let ty = erased_place_type(function, place, contract, tc)?;
        if moving && paths.exact(place).is_none() {
            return Err(MirErasedError::ProjectedMoveRequiresDropProof);
        }
        if access_conflicts(
            place.local,
            if moving {
                LoanKind::Mut
            } else {
                LoanKind::Shared
            },
            borrows,
            possibly_live,
            table,
        ) {
            return Err(MirErasedError::BorrowConflict);
        }
        if moving {
            let index = paths.exact(place).ok_or(MirErasedError::ProjectedMoveRequiresDropProof)?;
            for path in paths.affected(index, false) {
                initialized[path] = InitState::Moved(MoveInfo { span: None });
            }
            if place.projection.is_empty() { possibly_live[place.local.0 as usize] = false; }
        } else if !copy_proven(&ty, tc) {
            return Err(MirErasedError::CopyWithoutProof);
        }
        Ok(ty)
    }
    let mut worklist = super::dataflow::engine::ForwardWorklist::new(body.blocks.len(), entry);
    while let Some((block_index, mut state)) = worklist.pop() {
        let State {
            initialized,
            possibly_live,
            origins,
            borrows: local_borrows,
            opened,
            payload_sources,
        } = &mut state;
        let block = &body.blocks[block_index];
        for (statement_index, statement) in block.statements.iter().enumerate() {
            *failure = Some((MirErasedBlock(block_index as u32), statement_index));
            let dest = statement.destination.0 as usize;
            let expected = body.locals.get(dest).ok_or(MirErasedError::InvalidLocal)?;
            let mut scoped = BTreeSet::new();
            expected.scoped_binders(&mut scoped);
            if !matches!(
                statement.value,
                MirErasedRvalue::Open { .. } | MirErasedRvalue::PackageOpen { .. } | MirErasedRvalue::OwnerOpen { .. }
            ) && scoped.iter().any(|index| !opened[*index as usize])
            {
                return Err(MirErasedError::UnboundParameter);
            }
            if possibly_live[dest] {
                return Err(MirErasedError::InvalidLocal);
            }
            let mut result_origins = OriginSet::new();
            let mut result_borrows = BTreeSet::new();
            let mut payload_source = None;
            let receiver_initialized = match &statement.value {
                MirErasedRvalue::DictionaryCall {
                    receiver: Some(receiver),
                    ..
                }
                | MirErasedRvalue::OpenedCall { receiver, .. }
                | MirErasedRvalue::OpenedErasedCall { receiver, .. } => initialized
                    .get(move_paths.containing(receiver))
                    .is_some_and(InitState::is_initialized),
                _ => true,
            };
            // Snapshot before evaluating/moving arguments; a later move cannot
            // retrospectively authorize an earlier receiver access.
            let receiver_access_conflicts = match &statement.value {
                MirErasedRvalue::DictionaryCall {
                    receiver: Some(receiver),
                    ..
                }
                | MirErasedRvalue::OpenedCall { receiver, .. }
                | MirErasedRvalue::OpenedErasedCall { receiver, .. } => {
                    [LoanKind::Shared, LoanKind::Mut].map(|access| {
                        access_conflicts(
                            receiver.local,
                            access,
                            local_borrows,
                            possibly_live,
                            &loan_table,
                        )
                    })
                }
                _ => [false; 2],
            };
            let receiver_conflicts = |mode: ReceiverMode| {
                receiver_access_conflicts[usize::from(LoanKind::from(mode) == LoanKind::Mut)]
            };
            let mut use_operand = |op: &MirErasedOperand| {
                let place = match op {
                    MirErasedOperand::Move(place) | MirErasedOperand::Copy(place) => place,
                };
                let ty = operand(
                    op,
                    function,
                    contract,
                    tc,
                    &move_paths,
                    initialized,
                    possibly_live,
                    local_borrows,
                    &loan_table,
                )?;
                for origin in &origins[place.local.0 as usize] {
                    if place.projection.contains(&MirErasedProjection::Deref) {
                        match origin {
                            Origin::Local(local) | Origin::Temporary(local) => {
                                result_origins.extend(origins[local.0].iter().copied());
                            }
                            origin => {
                                result_origins.insert(origin.loaded_contents());
                            }
                        }
                    } else {
                        result_origins.insert(*origin);
                    }
                }
                result_borrows.extend(
                    local_borrows
                        .roots_carried_by(Local(place.local.0 as usize), &loan_table)
                        .map(|root| root.0 as u32),
                );
                Ok::<_, MirErasedError>(ty)
            };
            let actual = match &statement.value {
                MirErasedRvalue::OwnerOpen { scope, source, plan } => {
                    let evidence = body.scopes.get(*scope as usize).ok_or(MirErasedError::UnboundParameter)?;
                    if opened[*scope as usize]
                        || evidence.source != (MirErasedScopeSource::Owner { opening: plan.clone() })
                        || !matches!(source, MirErasedOperand::Move(_))
                        || !erased_types_agree(&use_operand(source)?, &MirErasedType::Concrete(plan.closed), tc)
                    {
                        return Err(MirErasedError::InvalidType);
                    }
                    // Metadata becomes authoritative only through this selected
                    // split. Validate the rebuild in that newly bound environment.
                    opened[*scope as usize] = true;
                    validate_owner_environment(&plan.rebuild, signature, &body_dictionaries, opened, &factory_layouts, contract, tc)?;
                    plan.opened.clone()
                }
                MirErasedRvalue::OwnerCall { scope, owner, split, release, slot, args } => {
                    let evidence = body.scopes.get(*scope as usize).ok_or(MirErasedError::UnboundParameter)?;
                    let MirErasedScopeSource::Owner { opening } = &evidence.source else {
                        return Err(MirErasedError::InvalidType);
                    };
                    let protocol = contract.owner_protocol.as_ref().ok_or(MirErasedError::MissingCallable)?;
                    let (unique, release_member) = protocol.unique.ok_or(MirErasedError::MissingCallable)?;
                    if !opened[*scope as usize] || !matches!(owner, MirErasedOperand::Move(_))
                        || !matches!(split.target, MirErasedOwnerTarget::Erased { .. })
                        || !matches!(release.target, MirErasedOwnerTarget::Erased { .. })
                        || split.constructor != opening.split.constructor
                        || split.implementation != opening.split.implementation
                        || split.trait_id != protocol.transport || split.member != protocol.split
                        || release.constructor != split.constructor
                        || release.trait_id != unique || release.member != release_member
                        || !erased_types_agree(&use_operand(owner)?, &opening.opened, tc)
                    {
                        return Err(MirErasedError::InvalidType);
                    }
                    validate_owner_environment(split, signature, &body_dictionaries, opened, &factory_layouts, contract, tc)?;
                    validate_owner_environment(release, signature, &body_dictionaries, opened, &factory_layouts, contract, tc)?;
                    let (params, parts) = super::owner_open::operation_types(split, contract)?;
                    let (release_params, release_ret) = super::owner_open::operation_types(release, contract)?;
                    let unit = MirErasedType::Concrete(tc.id_for_type(&Type::Unit).ok_or(MirErasedError::MissingLayout)?);
                    let byte_pointer = MirErasedType::Pointer(Box::new(MirErasedType::Concrete(tc.id_for_type(&Type::U8).ok_or(MirErasedError::MissingLayout)?)));
                    if params.len() != 1 || !erased_types_agree(&params[0], &opening.opened, tc)
                        || !erased_types_agree(&parts, &MirErasedType::Tuple(vec![MirErasedType::Pointer(Box::new(MirErasedType::Scoped(*scope))), opening.state.clone()]), tc)
                        || release_params.len() != 2 || !erased_types_agree(&release_params[0], &byte_pointer, tc)
                        || !erased_types_agree(&release_params[1], &opening.state, tc)
                        || !erased_types_agree(&release_ret, &unit, tc)
                    {
                        return Err(MirErasedError::InvalidType);
                    }
                    let schema = &contract.object_schemas[&evidence.object];
                    match slot {
                        MirErasedOwnedSlot::Concrete(index) => {
                            let member = schema.slots.get(*index as usize).ok_or(MirErasedError::MissingCallable)?;
                            if member.receiver != ReceiverMode::Move || member.result != super::MirObjectResult::Direct
                                || args.len() + 1 != member.signature.params.len()
                            { return Err(MirErasedError::InvalidType); }
                            for (arg, expected) in args.iter().zip(member.signature.params.iter().skip(1)) {
                                if !erased_types_agree(&use_operand(arg)?, &MirErasedType::Concrete(expected.semantic_ty), tc) {
                                    return Err(MirErasedError::InvalidType);
                                }
                            }
                            MirErasedType::Concrete(member.signature.ret.semantic_ty)
                        }
                        MirErasedOwnedSlot::Erased { slot, type_args, dictionaries } => {
                            let member = schema.erased_slots.get(*slot as usize).ok_or(MirErasedError::MissingCallable)?;
                            if member.receiver != ReceiverMode::Move || member.result != super::MirObjectResult::Direct
                                || type_args.first() != Some(&MirErasedType::Scoped(*scope))
                                || member.signature.params.first() != Some(&MirErasedType::Parameter(0))
                                || args.len() + 1 != member.signature.params.len()
                            { return Err(MirErasedError::InvalidType); }
                            validate_body_erased_environment(&member.signature, type_args, dictionaries, signature, &body_dictionaries, opened, &factory_layouts, contract, tc)?;
                            for (arg, expected) in args.iter().zip(member.signature.params.iter().skip(1)) {
                                if !erased_types_agree(&use_operand(arg)?, &expected.substitute(type_args)?, tc) {
                                    return Err(MirErasedError::InvalidType);
                                }
                            }
                            member.signature.ret.substitute(type_args)?
                        }
                    }
                }
                MirErasedRvalue::Forget(value) => {
                    use_operand(value)?;
                    result_origins.clear(); result_borrows.clear();
                    MirErasedType::Concrete(tc.id_for_type(&Type::Unit).ok_or(MirErasedError::MissingLayout)?)
                }
                MirErasedRvalue::DropInPlace { pointer, pointee } => {
                    if use_operand(pointer)? != MirErasedType::Pointer(Box::new(pointee.clone())) {
                        return Err(MirErasedError::InvalidType);
                    }
                    if !signature.layouts.contains(pointee) && !factory_layouts.contains(pointee)
                        && !matches!(pointee, MirErasedType::Concrete(id) if contract.erased.descriptors.contains_key(id))
                    {
                        return Err(MirErasedError::MissingLayout);
                    }
                    result_origins.clear(); result_borrows.clear();
                    MirErasedType::Concrete(tc.id_for_type(&Type::Unit).ok_or(MirErasedError::MissingLayout)?)
                }
                MirErasedRvalue::PackageOpen { scope, source } => {
                    let evidence = body
                        .scopes
                        .get(*scope as usize)
                        .ok_or(MirErasedError::UnboundParameter)?;
                    let MirErasedScopeSource::Package { package } = evidence.source else {
                        return Err(MirErasedError::InvalidType);
                    };
                    if opened[*scope as usize]
                        || !matches!(source, MirErasedOperand::Move(place) if place.projection.is_empty())
                    {
                        return Err(MirErasedError::InvalidType);
                    }
                    if use_operand(source)? != MirErasedType::Concrete(package) {
                        return Err(MirErasedError::InvalidType);
                    }
                    opened[*scope as usize] = true;
                    payload_source = Some(*scope);
                    super::scoped_package_payload(
                        package,
                        *scope,
                        evidence.witness,
                        &mut tc.clone(),
                    )?
                }
                MirErasedRvalue::PackagePack { payload, plan } => {
                    if !super::package_plan_is_valid(contract, tc, plan, &body.scopes, opened) {
                        return Err(MirErasedError::InvalidType);
                    }
                    let actual = use_operand(payload)?;
                    let scope = match plan.witness {
                        super::MirPackageWitnessEvidence::Scoped { scope, .. } => Some(scope),
                        _ => None,
                    };
                    let witness = scope
                        .and_then(|index| body.scopes.get(index as usize))
                        .map(|scope| Type::Witness(scope.witness));
                    let witness = match (&plan.witness, witness) {
                        (super::MirPackageWitnessEvidence::Concrete { witness, .. }, _) => {
                            tc.type_for(*witness)
                        }
                        (_, Some(witness)) => witness,
                        _ => return Err(MirErasedError::UnboundParameter),
                    };
                    let expected_payload = super::package_payload_type(plan.package, &witness, tc)?;
                    let expected_payload = MirErasedType::from_type_scoped(
                        &expected_payload,
                        &BTreeMap::new(),
                        None,
                        &body
                            .scopes
                            .iter()
                            .enumerate()
                            .map(|(index, scope)| (scope.witness, index as u32))
                            .collect(),
                        &mut tc.clone(),
                        &Default::default(),
                    )?;
                    if !erased_types_agree(&actual, &expected_payload, tc)
                        || !super::package_plan_is_valid(contract, tc, plan, &body.scopes, opened)
                    {
                        return Err(MirErasedError::InvalidType);
                    }
                    if let super::MirPackagePayloadDescriptor::FromPackage(scope) =
                        plan.payload_descriptor
                    {
                        let place = match payload {
                            MirErasedOperand::Move(place) | MirErasedOperand::Copy(place) => place,
                        };
                        if !place.projection.is_empty()
                            || payload_sources[place.local.0 as usize] != Some(scope)
                        {
                            return Err(MirErasedError::InvalidType);
                        }
                    }
                    MirErasedType::Concrete(plan.package)
                }
                MirErasedRvalue::Open { scope, source } => {
                    let schema = body
                        .scopes
                        .get(*scope as usize)
                        .ok_or(MirErasedError::UnboundParameter)?;
                    if opened[*scope as usize] {
                        return Err(MirErasedError::InvalidType);
                    }
                    let MirErasedScopeSource::Object { mutable } = schema.source else {
                        return Err(MirErasedError::InvalidType);
                    };
                    let source_ty = use_operand(source)?.instantiate(&[], tc)?;
                    if !matches!(source_ty, Type::Reference { inner, mutable }
                    if tc.id_for_type(&inner) == Some(schema.object) && mutable == schema.allows_mutable_place())
                    {
                        return Err(MirErasedError::InvalidType);
                    }
                    opened[*scope as usize] = true;
                    MirErasedType::Reference {
                        inner: Box::new(MirErasedType::Scoped(*scope)),
                        mutable,
                    }
                }
                MirErasedRvalue::Reclose {
                    scope,
                    value,
                    object,
                } => {
                    let evidence = body
                        .scopes
                        .get(*scope as usize)
                        .ok_or(MirErasedError::UnboundParameter)?;
                    if !opened[*scope as usize] {
                        return Err(MirErasedError::UnboundParameter);
                    }
                    let source = use_operand(value)?;
                    let MirErasedType::Reference { inner, mutable } = source else {
                        return Err(MirErasedError::InvalidType);
                    };
                    if *inner != MirErasedType::Scoped(*scope)
                        || (mutable && !evidence.allows_mutable_place())
                        || !super::object::object_view_is_admitted(tc, evidence.object, *object)
                    {
                        return Err(MirErasedError::InvalidType);
                    }
                    MirErasedType::Concrete(
                        tc.id_for_type(&Type::Reference {
                            inner: Box::new(tc.type_for(*object)),
                            mutable,
                        })
                        .ok_or(MirErasedError::MissingLayout)?,
                    )
                }
                MirErasedRvalue::OpenedCall {
                    scope,
                    slot,
                    receiver,
                    args,
                } => {
                    let evidence = body
                        .scopes
                        .get(*scope as usize)
                        .ok_or(MirErasedError::UnboundParameter)?;
                    if !opened[*scope as usize] || !receiver_initialized {
                        return Err(MirErasedError::UninitializedOrMoved);
                    }
                    let slot = contract.object_schemas[&evidence.object]
                        .slots
                        .get(*slot as usize)
                        .ok_or(MirErasedError::MissingCallable)?;
                    if erased_place_type(function, receiver, contract, tc)?
                        != MirErasedType::Scoped(*scope)
                        || slot.receiver == ReceiverMode::Move
                        || (slot.receiver == ReceiverMode::Mut && !evidence.allows_mutable_place())
                        || args.len() + 1 != slot.signature.params.len()
                    {
                        return Err(MirErasedError::InvalidType);
                    }
                    if receiver_conflicts(slot.receiver) {
                        return Err(MirErasedError::BorrowConflict);
                    }
                    if slot.receiver == ReceiverMode::Mut
                        && !erased_place_is_writable(function, receiver, contract, tc)?
                    {
                        return Err(MirErasedError::BorrowConflict);
                    }
                    for (arg, expected) in args.iter().zip(slot.signature.params.iter().skip(1)) {
                        if slot.receiver == ReceiverMode::Mut
                            && erased_type_may_borrow(
                                &MirErasedType::Concrete(expected.semantic_ty),
                                signature,
                                contract,
                                tc,
                            )
                        {
                            // Retention through a mutable hidden receiver needs a
                            // checked origin envelope, not just a matching native ABI.
                            return Err(MirErasedError::UnsupportedOperation);
                        }
                        if use_operand(arg)? != MirErasedType::Concrete(expected.semantic_ty) {
                            return Err(MirErasedError::InvalidType);
                        }
                    }
                    result_origins.extend(origins[receiver.local.0 as usize].iter().copied());
                    result_borrows.extend(
                        local_borrows
                            .roots_carried_by(Local(receiver.local.0 as usize), &loan_table)
                            .map(|root| root.0 as u32),
                    );
                    if erased_type_may_borrow(
                        &MirErasedType::Concrete(slot.signature.ret.semantic_ty),
                        signature,
                        contract,
                        tc,
                    ) {
                        result_borrows.insert(receiver.local.0);
                        if !erased_place_reborrows_reference(function, receiver, contract, tc) {
                            result_origins.insert(Origin::Local(Local(receiver.local.0 as usize)));
                        }
                    }
                    match slot.result {
                        super::MirObjectResult::BorrowedSelf { mutable } => {
                            MirErasedType::Reference {
                                inner: Box::new(MirErasedType::Scoped(*scope)),
                                mutable,
                            }
                        }
                        super::MirObjectResult::Direct => {
                            MirErasedType::Concrete(slot.signature.ret.semantic_ty)
                        }
                    }
                }
                MirErasedRvalue::OpenedErasedCall {
                    scope,
                    slot,
                    type_args,
                    dictionaries,
                    receiver,
                    args,
                } => {
                    let evidence = body
                        .scopes
                        .get(*scope as usize)
                        .ok_or(MirErasedError::UnboundParameter)?;
                    if !opened[*scope as usize] || !receiver_initialized {
                        return Err(MirErasedError::UninitializedOrMoved);
                    }
                    let slot = contract.object_schemas[&evidence.object]
                        .erased_slots
                        .get(*slot as usize)
                        .ok_or(MirErasedError::MissingCallable)?;
                    if type_args.first() != Some(&MirErasedType::Scoped(*scope))
                        || erased_place_type(function, receiver, contract, tc)?
                            != MirErasedType::Scoped(*scope)
                        || args.len() + 1 != slot.signature.params.len()
                    {
                        return Err(MirErasedError::InvalidType);
                    }
                    if slot.receiver == ReceiverMode::Move {
                        return Err(MirErasedError::UnsupportedOperation);
                    }
                    if slot.receiver == ReceiverMode::Mut
                        && (!evidence.allows_mutable_place()
                            || !erased_place_is_writable(function, receiver, contract, tc)?)
                    {
                        return Err(MirErasedError::BorrowConflict);
                    }
                    if receiver_conflicts(slot.receiver) {
                        return Err(MirErasedError::BorrowConflict);
                    }
                    validate_body_erased_environment(
                        &slot.signature,
                        type_args,
                        dictionaries,
                        signature,
                        &body_dictionaries,
                        opened,
                        &factory_layouts,
                        contract,
                        tc,
                    )?;
                    if slot
                        .signature
                        .params
                        .first()
                        .ok_or(MirErasedError::InvalidType)?
                        .substitute(type_args)?
                        != (MirErasedType::Reference {
                            inner: Box::new(MirErasedType::Scoped(*scope)),
                            mutable: slot.receiver == ReceiverMode::Mut,
                        })
                    {
                        return Err(MirErasedError::InvalidType);
                    }
                    for (arg, expected) in args.iter().zip(slot.signature.params.iter().skip(1)) {
                        let expected = expected.substitute(type_args)?;
                        if slot.receiver == ReceiverMode::Mut
                            && erased_type_may_borrow(&expected, signature, contract, tc)
                        {
                            return Err(MirErasedError::UnsupportedOperation);
                        }
                        if !erased_types_agree(&use_operand(arg)?, &expected, tc) {
                            return Err(MirErasedError::InvalidType);
                        }
                    }
                    let ret = slot.signature.ret.substitute(type_args)?;
                    result_origins.extend(origins[receiver.local.0 as usize].iter().copied());
                    result_borrows.extend(
                        local_borrows
                            .roots_carried_by(Local(receiver.local.0 as usize), &loan_table)
                            .map(|root| root.0 as u32),
                    );
                    if erased_type_may_borrow(&ret, signature, contract, tc) {
                        result_borrows.insert(receiver.local.0);
                        if !erased_place_reborrows_reference(function, receiver, contract, tc) {
                            result_origins.insert(Origin::Local(Local(receiver.local.0 as usize)));
                        }
                    }
                    ret
                }
                MirErasedRvalue::NativeCall { target, args } => {
                    let callee = contract
                        .callable(target)
                        .ok_or(MirErasedError::MissingCallable)?;
                    if matches!(callee.kind, super::MirCallableKind::ObjectProvided)
                        && !imported_native_link_is_valid(
                            function.key.definition,
                            target,
                            callee,
                            contract,
                        )
                    {
                        return Err(MirErasedError::MissingCallable);
                    }
                    if !matches!(
                        callee.kind,
                        super::MirCallableKind::LocalBody { .. }
                            | super::MirCallableKind::ObjectProvided
                    ) || args.len() != callee.signature.params.len()
                        || signature.params.len() != callee.signature.params.len()
                        || callee.source_def_id != Some(function.key.definition)
                        || signature.parameters.len() != 1
                        || !signature.dictionaries.is_empty()
                        || signature
                            .params
                            .iter()
                            .chain(std::iter::once(&signature.ret))
                            .any(|ty| {
                                ty.instantiate(&[], tc).map_or(true, |ty| {
                                    !crate::type_services::facts::TypeFacts::is_codegen_concrete(
                                        &ty,
                                    )
                                })
                            })
                    {
                        return Err(MirErasedError::InvalidType);
                    }
                    let source = *function
                        .key
                        .static_args
                        .first()
                        .ok_or(MirErasedError::InvalidType)?;
                    if !tc.type_id_tree_is_valid(source)
                        || !crate::type_services::facts::TypeFacts::is_codegen_concrete(
                            &tc.type_for(source),
                        )
                        || function.key.static_args.iter().any(|id| {
                            !tc.type_id_tree_is_valid(*id)
                            || !crate::type_services::facts::TypeFacts::is_concrete_type_argument(
                                &tc.type_for(*id),
                            )
                        })
                    {
                        return Err(MirErasedError::InvalidType);
                    }
                    let receiver = signature
                        .params
                        .first()
                        .ok_or(MirErasedError::InvalidType)?
                        .instantiate(&[], tc)?;
                    let receiver_source = match receiver {
                        Type::Reference { inner, .. } => *inner,
                        receiver => receiver,
                    };
                    if receiver_source != tc.type_for(source) {
                        return Err(MirErasedError::InvalidType);
                    }
                    if !tc.type_id_tree_is_valid(callee.signature.ret.semantic_ty)
                        || callee.signature.ret.abi_ty != callee.signature.ret.semantic_ty
                        || callee
                            .signature
                            .params
                            .iter()
                            .any(|parameter| !tc.type_id_tree_is_valid(parameter.semantic_ty))
                        || !erased_types_agree(
                            &signature.ret,
                            &MirErasedType::Concrete(callee.signature.ret.semantic_ty),
                            tc,
                        )
                        || signature.params.iter().zip(&callee.signature.params).any(
                            |(expected, parameter)| {
                                !erased_types_agree(
                                    expected,
                                    &MirErasedType::Concrete(parameter.semantic_ty),
                                    tc,
                                )
                            },
                        )
                    {
                        return Err(MirErasedError::InvalidType);
                    }
                    for (index, parameter) in callee.signature.params.iter().enumerate() {
                        if !matches!(
                            tc.type_for(parameter.semantic_ty),
                            Type::Reference { mutable: true, .. }
                        ) {
                            continue;
                        }
                        // A native ABI does not authorize storing another input's
                        // hidden region in mutable receiver storage.
                        if callee
                            .signature
                            .params
                            .iter()
                            .enumerate()
                            .any(|(other, parameter)| {
                                other != index
                                    && erased_type_may_borrow(
                                        &MirErasedType::Concrete(parameter.semantic_ty),
                                        signature,
                                        contract,
                                        tc,
                                    )
                            })
                        {
                            return Err(MirErasedError::UnsupportedOperation);
                        }
                    }
                    for (arg, parameter) in args.iter().zip(&callee.signature.params) {
                        if !erased_types_agree(
                            &use_operand(arg)?,
                            &MirErasedType::Concrete(parameter.semantic_ty),
                            tc,
                        ) {
                            return Err(MirErasedError::InvalidType);
                        }
                    }
                    MirErasedType::Concrete(callee.signature.ret.semantic_ty)
                }
                MirErasedRvalue::NativeApply { target, args } => {
                    let callee = contract
                        .callable(target)
                        .ok_or(MirErasedError::MissingCallable)?;
                    if !matches!(
                        callee.kind,
                        super::MirCallableKind::LocalBody { .. }
                            | super::MirCallableKind::ObjectProvided
                    ) || args.len() != callee.signature.params.len()
                        || callee.signature.ret.semantic_ty != callee.signature.ret.abi_ty
                    {
                        return Err(MirErasedError::InvalidType);
                    }
                    if matches!(callee.kind, super::MirCallableKind::ObjectProvided)
                        && !callee.source_def_id.is_some_and(|definition| {
                            imported_native_link_is_valid(definition, target, callee, contract)
                        })
                    {
                        return Err(MirErasedError::MissingCallable);
                    }
                    for (index, parameter) in callee.signature.params.iter().enumerate() {
                        if matches!(
                            tc.type_for(parameter.semantic_ty),
                            Type::Reference { mutable: true, .. }
                        ) && callee.signature.params.iter().enumerate().any(
                            |(other, parameter)| {
                                other != index
                                    && erased_type_may_borrow(
                                        &MirErasedType::Concrete(parameter.semantic_ty),
                                        signature,
                                        contract,
                                        tc,
                                    )
                            },
                        ) {
                            return Err(MirErasedError::UnsupportedOperation);
                        }
                    }
                    for (arg, parameter) in args.iter().zip(&callee.signature.params) {
                        if !erased_types_agree(
                            &use_operand(arg)?,
                            &MirErasedType::Concrete(parameter.semantic_ty),
                            tc,
                        ) {
                            return Err(MirErasedError::InvalidType);
                        }
                    }
                    MirErasedType::Concrete(callee.signature.ret.semantic_ty)
                }
                MirErasedRvalue::NominalAggregate { ty, fields } => {
                    let requirement = super::nominal_requirement(contract, tc, ty)?;
                    if fields.len() != requirement.fields.len() {
                        return Err(MirErasedError::InvalidType);
                    }
                    for ((id, operand), (expected_id, expected_ty)) in
                        fields.iter().zip(&requirement.fields)
                    {
                        if id != expected_id
                            || !erased_types_agree(&use_operand(operand)?, expected_ty, tc)
                        {
                            return Err(MirErasedError::InvalidType);
                        }
                    }
                    ty.clone()
                }
                MirErasedRvalue::NativeAggregate { ty, fields } => {
                    let descriptor = contract
                        .erased
                        .descriptors
                        .get(ty)
                        .ok_or(MirErasedError::MissingLayout)?;
                    if !matches!(tc.type_for(*ty), Type::Struct { .. })
                        || fields.len() != descriptor.fields.len()
                    {
                        return Err(MirErasedError::InvalidType);
                    }
                    for (field, expected) in fields.iter().zip(&descriptor.fields) {
                        if !erased_types_agree(
                            &use_operand(field)?,
                            &MirErasedType::Concrete(*expected),
                            tc,
                        ) {
                            return Err(MirErasedError::InvalidType);
                        }
                    }
                    MirErasedType::Concrete(*ty)
                }
                MirErasedRvalue::Use(op) => {
                    let place = match op {
                        MirErasedOperand::Move(place) | MirErasedOperand::Copy(place) => place,
                    };
                    if place.projection.is_empty() {
                        payload_source = payload_sources[place.local.0 as usize];
                    }
                    use_operand(op)?
                }
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
                        .get(move_paths.containing(place))
                        .is_some_and(InitState::is_initialized)
                    {
                        return Err(MirErasedError::UninitializedOrMoved);
                    }
                    if *mutable && !erased_place_is_writable(function, place, contract, tc)? {
                        return Err(MirErasedError::BorrowConflict);
                    }
                    let access = if *mutable {
                        LoanKind::Mut
                    } else {
                        LoanKind::Shared
                    };
                    if access_conflicts(
                        place.local,
                        access,
                        local_borrows,
                        possibly_live,
                        &loan_table,
                    ) {
                        return Err(MirErasedError::BorrowConflict);
                    }
                    result_origins.extend(
                        origins[place.local.0 as usize]
                            .iter()
                            .map(|origin| origin.borrowed_contents(false, false)),
                    );
                    result_borrows.extend(
                        local_borrows
                            .roots_carried_by(Local(place.local.0 as usize), &loan_table)
                            .map(|root| root.0 as u32),
                    );
                    if !erased_place_reborrows_reference(function, place, contract, tc) {
                        result_origins.insert(Origin::Local(Local(place.local.0 as usize)));
                    }
                    result_borrows.insert(place.local.0);
                    MirErasedType::Reference {
                        inner: Box::new(erased_place_type(function, place, contract, tc)?),
                        mutable: *mutable,
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
                    if native_bridge_source(callee).is_some_and(|source| {
                        type_args.first() != Some(&MirErasedType::Concrete(source))
                    }) {
                        return Err(MirErasedError::InvalidType);
                    }
                    if args.len() != callee.signature.params.len() {
                        return Err(MirErasedError::InvalidType);
                    }
                    validate_body_erased_environment(
                        &callee.signature,
                        type_args,
                        dictionaries,
                        signature,
                        &body_dictionaries,
                        opened,
                        &factory_layouts,
                        contract,
                        tc,
                    )?;
                    for (arg, expected) in args.iter().zip(&callee.signature.params) {
                        if !erased_types_agree(
                            &use_operand(arg)?,
                            &expected.substitute(type_args)?,
                            tc,
                        ) {
                            return Err(MirErasedError::InvalidType);
                        }
                    }
                    callee.signature.ret.substitute(type_args)?
                }
                MirErasedRvalue::DictionaryCall {
                    dictionary,
                    member,
                    type_args,
                    dictionaries,
                    receiver,
                    args,
                } => {
                    let dictionary = body_dictionaries
                        .get(*dictionary as usize)
                        .ok_or(MirErasedError::InvalidDictionary)?;
                    let member = dictionary
                        .members
                        .get(*member as usize)
                        .ok_or(MirErasedError::InvalidDictionary)?;
                    if !dictionaries.is_empty() || type_args.len() != member.binder.parameters.len()
                    {
                        return Err(MirErasedError::InvalidDictionary);
                    }
                    for argument in type_args {
                        leaves(argument, contract, tc)?;
                        if argument.kind(signature, contract, tc)? != Kind::Type {
                            return Err(MirErasedError::KindMismatch);
                        }
                        if !signature.layouts.contains(argument)
                            && !factory_layouts.contains(argument)
                            && !matches!(argument, MirErasedType::Concrete(id) if contract.erased.descriptors.contains_key(id))
                            && !erased_scoped_layout_is_available(argument, opened)
                        {
                            return Err(MirErasedError::MissingLayout);
                        }
                    }
                    let (params, ret) = member.opened_types(type_args)?;
                    if receiver.is_some() != member.receiver.is_some() {
                        return Err(MirErasedError::InvalidDictionary);
                    }
                    if let Some(receiver) = receiver {
                        if !receiver_initialized {
                            return Err(MirErasedError::UninitializedOrMoved);
                        }
                        if erased_place_type(function, receiver, contract, tc)?
                            != dictionary.subject
                        {
                            return Err(MirErasedError::InvalidDictionary);
                        }
                        if member.receiver != Some(ReceiverMode::Move) && args.iter().any(|arg| matches!(arg, MirErasedOperand::Move(place) if place.local == receiver.local)) { return Err(MirErasedError::BorrowConflict); }
                        if member.receiver.is_some_and(receiver_conflicts) {
                            return Err(MirErasedError::BorrowConflict);
                        }
                        if member.receiver == Some(ReceiverMode::Mut)
                            && !erased_place_is_writable(function, receiver, contract, tc)?
                        {
                            return Err(MirErasedError::BorrowConflict);
                        }
                        if member.receiver == Some(ReceiverMode::Move) {
                            use_operand(&MirErasedOperand::Move(receiver.clone()))?;
                        }
                    }
                    let receiver_count = usize::from(receiver.is_some());
                    if args.len() + receiver_count != params.len() {
                        return Err(MirErasedError::InvalidDictionary);
                    }
                    for (arg, expected) in args.iter().zip(params.iter().skip(receiver_count)) {
                        if use_operand(arg)? != *expected {
                            return Err(MirErasedError::InvalidDictionary);
                        }
                    }
                    // The contract covers writes even when the result is unit.
                    // Opaque storage is not an outlives proof: only origins already
                    // carried by that destination may be retained there.
                    let inputs = receiver
                        .iter()
                        .chain(args.iter().map(|arg| match arg {
                            MirErasedOperand::Move(place) | MirErasedOperand::Copy(place) => place,
                        }))
                        .collect::<Vec<_>>();
                    for effect in &member.borrowing.retentions {
                        // Descriptor-bound destinations retain this precondition
                        // in the dictionary interface. The concrete entry validator
                        // must discharge it before runtime execution.
                        let Ok(destination_type) =
                            params[effect.destination.argument as usize].instantiate(&[], tc)
                        else {
                            continue;
                        };
                        let destination_type =
                            borrow_path_type(&destination_type, &effect.destination.path)
                                .ok_or(MirErasedError::InvalidDictionary)?;
                        let can_retain = if effect.destination.argument == 0
                            && member
                                .receiver
                                .is_some_and(|mode| mode != ReceiverMode::Shared)
                        {
                            reference_slots(destination_type, contract, tc)
                        } else {
                            writable_reference_slots(destination_type, contract, tc)
                        };
                        if !can_retain {
                            continue;
                        }
                        let destination = inputs[effect.destination.argument as usize];
                        for source in &effect.sources {
                            let index = source.argument as usize;
                            if erased_type_may_borrow(&params[index], signature, contract, tc)
                                && !origins[inputs[index].local.0 as usize]
                                    .is_subset(&origins[destination.local.0 as usize])
                            {
                                return Err(MirErasedError::BorrowEscape);
                            }
                        }
                    }
                    if let Some(receiver) = receiver {
                        let receiver_origin = member
                            .borrowing
                            .returns
                            .iter()
                            .flat_map(|effect| &effect.sources)
                            .any(|source| source.argument == 0);
                        if member.receiver != Some(ReceiverMode::Move) && receiver_origin {
                            result_origins
                                .extend(origins[receiver.local.0 as usize].iter().copied());
                            result_borrows.extend(
                                local_borrows
                                    .roots_carried_by(Local(receiver.local.0 as usize), &loan_table)
                                    .map(|root| root.0 as u32),
                            );
                        }
                        if member.receiver != Some(ReceiverMode::Move)
                            && receiver_origin
                            && erased_type_may_borrow(&ret, signature, contract, tc)
                        {
                            result_borrows.insert(receiver.local.0);
                            if !erased_place_reborrows_reference(function, receiver, contract, tc) {
                                result_origins
                                    .insert(Origin::Local(Local(receiver.local.0 as usize)));
                            }
                        }
                    }
                    ret
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
            initialized[dest] = InitState::Init;
            for path in move_paths.descendants(dest) { initialized[path] = InitState::Init; }
            possibly_live[dest] = true;
            if !erased_type_may_borrow(&actual, signature, contract, tc) {
                result_borrows.clear();
                result_origins.retain(|origin| origin.local_storage().is_none());
            }
            origins[dest] = result_origins;
            local_borrows.release_owner(Local(dest));
            for root in result_borrows {
                let loan = loan_table
                    .root_envelope_id(Local(root as usize), loan_kind(&actual, tc))
                    .ok_or(MirErasedError::InvalidLocal)?;
                local_borrows.add_owner(loan, Local(dest));
            }
            payload_sources[dest] = payload_source;
        }
        *failure = Some((MirErasedBlock(block_index as u32), block.statements.len()));
        let successors = match &block.terminator {
            MirErasedTerminator::Goto(target) => vec![*target],
            MirErasedTerminator::Cleanup { locals, target } => {
                let mut seen = BTreeSet::new();
                for local in super::cleanup::cleanup_order(locals) {
                    let index = local.0 as usize;
                    if index >= body.locals.len() || !seen.insert(local) {
                        return Err(MirErasedError::InvalidLocal);
                    }
                    // Reborrowing a reference's contents does not keep its
                    // pointer slot alive; borrowing its storage does.
                    if possibly_live[index]
                        && possibly_live.iter().zip(origins.iter()).enumerate().any(
                            |(owner, (live, origin))| {
                                *live
                                    && local_borrows
                                        .roots_carried_by(Local(owner), &loan_table)
                                        .any(|root| root.0 == local.0 as usize)
                                    && origin.contains(&Origin::Local(Local(local.0 as usize)))
                            },
                        )
                    {
                        return Err(MirErasedError::BorrowConflict);
                    }
                    initialized[index] = InitState::Uninit;
                    for path in move_paths.descendants(index) { initialized[path] = InitState::Uninit; }
                    possibly_live[index] = false;
                    local_borrows.release_owner(Local(index));
                    origins[index].clear();
                    payload_sources[index] = None;
                }
                vec![*target]
            }
            MirErasedTerminator::Branch {
                condition,
                then_block,
                else_block,
            } => {
                let condition_ty = operand(
                    condition,
                    function,
                    contract,
                    tc,
                    &move_paths,
                    initialized,
                    possibly_live,
                    local_borrows,
                    &loan_table,
                )?;
                if condition_ty.instantiate(&[], tc)? != Type::Bool {
                    return Err(MirErasedError::InvalidType);
                }
                vec![*then_block, *else_block]
            }
            MirErasedTerminator::Unreachable => Vec::new(),
            MirErasedTerminator::Return(result) => {
                let returned = match result {
                    MirErasedOperand::Move(place) | MirErasedOperand::Copy(place) => place,
                };
                if origins
                    .get(returned.local.0 as usize)
                    .ok_or(MirErasedError::InvalidLocal)?
                    .iter()
                    .any(|origin| origin.local_storage().is_some())
                {
                    return Err(MirErasedError::BorrowEscape);
                }
                let borrowing =
                    if let Some(member) = contract.erased.member_entries.get(&function.key) {
                        member.member.borrowing.clone()
                    } else {
                        erased_borrow_contract(&signature.params, &signature.ret, None, tc)?
                    };
                let mut authorized = borrowing
                    .returns
                    .iter()
                    .flat_map(|result| &result.sources)
                    .map(|source| source.argument as usize)
                    .collect::<BTreeSet<_>>();
                if contract.erased.owner_rebuild_entries.contains_key(&function.key) {
                    super::owner_open::validate_rebuild_entry(&function.key, contract, tc)?;
                    authorized.insert(0);
                }
                if erased_type_may_borrow(&signature.ret, signature, contract, tc)
                    && origins[returned.local.0 as usize]
                        .iter()
                        .any(|origin| match origin {
                            Origin::Param(local)
                            | Origin::ParamContents(local)
                            | Origin::ParamBorrowedContents(local) => {
                                !authorized.contains(&local.0)
                            }
                            Origin::Local(_) | Origin::Temporary(_) | Origin::UnknownExternal => {
                                true
                            }
                            Origin::Static => false,
                        })
                {
                    return Err(MirErasedError::BorrowEscape);
                }
                if !erased_types_agree(
                    &operand(
                        result,
                        function,
                        contract,
                        tc,
                        &move_paths,
                        initialized,
                        possibly_live,
                        local_borrows,
                        &loan_table,
                    )?,
                    &signature.ret,
                    tc,
                ) {
                    return Err(MirErasedError::InvalidType);
                }
                Vec::new()
            }
        };
        for successor in successors {
            worklist
                .propagate(successor.0 as usize, &state)
                .ok_or(MirErasedError::InvalidLocal)?;
        }
    }
    Ok(())
}

pub(crate) fn erased_types_agree(
    actual: &MirErasedType,
    expected: &MirErasedType,
    tc: &crate::type_context::TypeContext,
) -> bool {
    actual == expected
        || actual
            .instantiate(&[], tc)
            .ok()
            .zip(expected.instantiate(&[], tc).ok())
            .is_some_and(|(actual, expected)| actual == expected)
}

fn imported_native_link_is_valid(
    definition: DefId,
    target: &super::MirCallableKey,
    declaration: &super::MirCallableDecl,
    contract: &super::MirBackendContract,
) -> bool {
    let mut links = contract.artifact_exports.iter().filter(|link| {
        link.origin_def_id == Some(definition) && link.backend_symbol == declaration.llvm_symbol
    });
    matches!(target, super::MirCallableKey::Instance(_))
        && declaration.source_def_id == Some(definition)
        && declaration.linkage == super::MirLinkage::External
        && !declaration.llvm_symbol.is_empty()
        && !contract.function_bodies.values().any(|body| body == target)
        && links.next().is_some_and(|link| {
            link.provided_by_object
                && link.substitution_empty
                && !link.has_body
                && !link.is_specialization
                && !link.is_drop_glue
        })
        && links.next().is_none()
}

fn native_bridge_source(function: &MirErasedFunction) -> Option<TypeId> {
    function
        .body
        .as_ref()
        .filter(|body| {
            body.blocks
                .iter()
                .flat_map(|block| &block.statements)
                .any(|statement| matches!(&statement.value, MirErasedRvalue::NativeCall { .. }))
        })
        .and_then(|_| function.key.static_args.first().copied())
}

fn erased_type_has_mutable_reference(
    ty: &MirErasedType,
    tc: &crate::type_context::TypeContext,
) -> bool {
    match ty {
        MirErasedType::Concrete(id) => {
            crate::type_services::visit::type_any(&tc.type_for(*id), |ty| {
                matches!(ty, crate::types::Type::Reference { mutable: true, .. })
            })
        }
        MirErasedType::Reference { mutable, .. } => *mutable,
        MirErasedType::Tuple(fields) => fields
            .iter()
            .any(|ty| erased_type_has_mutable_reference(ty, tc)),
        MirErasedType::Array { element, .. } => erased_type_has_mutable_reference(element, tc),
        MirErasedType::Pointer(_) => false,
        _ => true,
    }
}

fn validate_body_erased_environment(
    callee: &MirErasedSignature,
    type_args: &[MirErasedType],
    dictionaries: &[u32],
    caller: &MirErasedSignature,
    caller_dictionaries: &[MirErasedDictionarySchema],
    opened: &[bool],
    factory_layouts: &BTreeSet<MirErasedType>,
    contract: &super::MirBackendContract,
    tc: &crate::type_context::TypeContext,
) -> Result<(), MirErasedError> {
    if type_args.len() != callee.parameters.len() || dictionaries.len() != callee.dictionaries.len()
    {
        return Err(MirErasedError::InvalidType);
    }
    for (ty, parameter) in type_args.iter().zip(&callee.parameters) {
        let mut scoped = BTreeSet::new();
        ty.scoped_binders(&mut scoped);
        if scoped
            .iter()
            .any(|index| opened.get(*index as usize) != Some(&true))
        {
            return Err(MirErasedError::UnboundParameter);
        }
        if ty.kind(caller, contract, tc)? != parameter.kind {
            return Err(MirErasedError::KindMismatch);
        }
    }
    for requirement in &callee.require_borrow_free {
        if erased_type_may_borrow(&requirement.substitute(type_args)?, caller, contract, tc) {
            return Err(MirErasedError::UnsupportedOperation);
        }
    }
    for (index, required) in dictionaries.iter().zip(&callee.dictionaries) {
        if caller_dictionaries.get(*index as usize) != Some(&required.substitute(type_args, tc)?) {
            return Err(MirErasedError::InvalidDictionary);
        }
    }
    for layout in &callee.layouts {
        let layout = layout.substitute(type_args)?;
        if !caller.layouts.contains(&layout)
            && !factory_layouts.contains(&layout)
            && !layout
                .instantiate(&[], tc)
                .ok()
                .and_then(|ty| tc.id_for_type(&ty))
                .is_some_and(|id| contract.erased.descriptors.contains_key(&id))
            && !erased_scoped_layout_is_available(&layout, opened)
        {
            return Err(MirErasedError::MissingLayout);
        }
    }
    Ok(())
}

fn validate_owner_environment(
    operation: &MirErasedOwnerOperation,
    signature: &MirErasedSignature,
    dictionaries: &[MirErasedDictionarySchema],
    opened: &[bool],
    factories: &BTreeSet<MirErasedType>,
    contract: &super::MirBackendContract,
    tc: &crate::type_context::TypeContext,
) -> Result<(), MirErasedError> {
    super::owner_open::operation_types(operation, contract)?;
    super::owner_open::validate_constructor(operation, contract, tc)?;
    if let MirErasedOwnerTarget::Erased { key, type_args } = &operation.target {
        validate_body_erased_environment(&contract.erased.functions[key].signature, type_args, &[], signature, dictionaries, opened, factories, contract, tc)?;
    }
    Ok(())
}

fn erased_place_reborrows_reference(
    function: &MirErasedFunction,
    place: &MirErasedPlace,
    contract: &super::MirBackendContract,
    tc: &crate::type_context::TypeContext,
) -> bool {
    let mut prefix = MirErasedPlace::local(place.local);
    for projection in &place.projection {
        if *projection == MirErasedProjection::Deref {
            // A contained reference carries its pointee's region, just like a
            // bare reference; borrowing the field's address still uses storage.
            return match erased_place_type(function, &prefix, contract, tc) {
                Ok(MirErasedType::Reference { .. }) => true,
                Ok(MirErasedType::Concrete(id)) => {
                    matches!(tc.type_for(id), crate::types::Type::Reference { .. })
                }
                _ => false,
            };
        }
        prefix.projection.push(projection.clone());
    }
    false
}

fn erased_place_is_writable(
    function: &MirErasedFunction,
    place: &MirErasedPlace,
    contract: &super::MirBackendContract,
    tc: &crate::type_context::TypeContext,
) -> Result<bool, MirErasedError> {
    let mut writable = function
        .body
        .as_ref()
        .is_some_and(|body| body.mutable_locals.contains(&place.local));
    let mut prefix = MirErasedPlace::local(place.local);
    for projection in &place.projection {
        if *projection == MirErasedProjection::Deref {
            writable = match erased_place_type(function, &prefix, contract, tc)? {
                MirErasedType::Reference { mutable, .. } => mutable,
                MirErasedType::Concrete(id) => matches!(
                    tc.type_for(id),
                    crate::types::Type::Reference { mutable: true, .. }
                ),
                _ => false,
            };
        }
        prefix.projection.push(projection.clone());
    }
    Ok(writable)
}

fn erased_scoped_layout_is_available(ty: &MirErasedType, opened: &[bool]) -> bool {
    let scope = match ty {
        MirErasedType::Scoped(scope) => scope,
        MirErasedType::Reference { inner, .. } | MirErasedType::Pointer(inner) => {
            let MirErasedType::Scoped(scope) = inner.as_ref() else {
                return false;
            };
            scope
        }
        _ => return false,
    };
    opened.get(*scope as usize) == Some(&true)
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

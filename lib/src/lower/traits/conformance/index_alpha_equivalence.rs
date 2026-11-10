//! Alpha-equivalence checks for index impls.

use crate::hir::*;
use crate::ids::{AssocTypeId, DefId};
use crate::types::{GenericParamId, TraitBound, Type};

pub(super) fn associated_type_for_index(
    imp: &HirImpl,
    id: AssocTypeId,
) -> Option<&HirAssociatedTypeDef> {
    imp.associated_types
        .iter()
        .find(|associated| associated.id == id)
}

pub(super) fn index_impls_are_alpha_equivalent(left: &HirImpl, right: &HirImpl) -> bool {
    alpha_equivalent_receiver_pattern(
        &left.receiver_pattern,
        left.id,
        &right.receiver_pattern,
        right.id,
    ) && left.type_generics.len() == right.type_generics.len()
        && alpha_equivalent_type_lists(
            &left.trait_arg_types,
            left.id,
            &right.trait_arg_types,
            right.id,
        )
        && alpha_equivalent_bounds(&left.bounds, left.id, &right.bounds, right.id)
}

fn alpha_equivalent_receiver_pattern(
    left: &HirImplReceiverPattern,
    left_impl: DefId,
    right: &HirImplReceiverPattern,
    right_impl: DefId,
) -> bool {
    match (left, right) {
        (
            HirImplReceiverPattern::Exact(Type::Reference {
                mutable: true,
                inner: left_inner,
            }),
            HirImplReceiverPattern::Exact(Type::Reference {
                mutable: false,
                inner: right_inner,
            }),
        ) => alpha_equivalent_type(left_inner, left_impl, right_inner, right_impl),
        (HirImplReceiverPattern::Exact(left), HirImplReceiverPattern::Exact(right))
            if !matches!(left, Type::Reference { .. })
                && !matches!(right, Type::Reference { .. }) =>
        {
            alpha_equivalent_type(left, left_impl, right, right_impl)
        }
        (
            HirImplReceiverPattern::SliceFamily { element: left },
            HirImplReceiverPattern::SliceFamily { element: right },
        ) => alpha_equivalent_type(left, left_impl, right, right_impl),
        _ => false,
    }
}

fn alpha_equivalent_type_lists(
    left: &[Type],
    left_impl: DefId,
    right: &[Type],
    right_impl: DefId,
) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| alpha_equivalent_type(left, left_impl, right, right_impl))
}

pub(super) fn alpha_equivalent_type(
    left: &Type,
    left_impl: DefId,
    right: &Type,
    right_impl: DefId,
) -> bool {
    match (left, right) {
        (Type::Generic(left), Type::Generic(right)) => {
            if left.owner == left_impl || right.owner == right_impl {
                left.owner == left_impl && right.owner == right_impl && left.index == right.index
            } else {
                left == right
            }
        }
        (Type::Slice(left), Type::Slice(right)) | (Type::Pointer(left), Type::Pointer(right)) => {
            alpha_equivalent_type(left, left_impl, right, right_impl)
        }
        (Type::Array(left, left_len), Type::Array(right, right_len)) => {
            alpha_equivalent_type(
                &left_len.to_const_type(),
                left_impl,
                &right_len.to_const_type(),
                right_impl,
            ) && alpha_equivalent_type(left, left_impl, right, right_impl)
        }
        (Type::Tuple(left), Type::Tuple(right)) => {
            alpha_equivalent_type_lists(left, left_impl, right, right_impl)
        }
        (
            Type::Function {
                params: left_params,
                ret: left_ret,
                safety: left_safety,
                callable_kind: left_kind,
                captures: left_captures,
            },
            Type::Function {
                params: right_params,
                ret: right_ret,
                safety: right_safety,
                callable_kind: right_kind,
                captures: right_captures,
            },
        ) => {
            left_safety == right_safety
                && left_kind == right_kind
                && left_captures.len() == right_captures.len()
                && left_captures
                    .iter()
                    .zip(right_captures)
                    .all(|(left, right)| {
                        left.kind == right.kind
                            && alpha_equivalent_type(&left.ty, left_impl, &right.ty, right_impl)
                    })
                && alpha_equivalent_type_lists(left_params, left_impl, right_params, right_impl)
                && alpha_equivalent_type(left_ret, left_impl, right_ret, right_impl)
        }
        (
            Type::Struct {
                id: left_id,
                args: left_args,
            },
            Type::Struct {
                id: right_id,
                args: right_args,
            },
        ) => {
            left_id == right_id
                && alpha_equivalent_type_lists(left_args, left_impl, right_args, right_impl)
        }
        (
            Type::Enum {
                id: left_id,
                args: left_args,
            },
            Type::Enum {
                id: right_id,
                args: right_args,
            },
        ) => {
            left_id == right_id
                && alpha_equivalent_type_lists(left_args, left_impl, right_args, right_impl)
        }
        (
            Type::Reference {
                mutable: left_mutable,
                inner: left_inner,
            },
            Type::Reference {
                mutable: right_mutable,
                inner: right_inner,
            },
        ) => {
            left_mutable == right_mutable
                && alpha_equivalent_type(left_inner, left_impl, right_inner, right_impl)
        }
        (
            Type::Projection {
                ty: left_ty,
                trait_id: left_trait,
                assoc_type: left_assoc,
                trait_args: left_args,
            },
            Type::Projection {
                ty: right_ty,
                trait_id: right_trait,
                assoc_type: right_assoc,
                trait_args: right_args,
            },
        ) => {
            left_trait == right_trait
                && left_assoc == right_assoc
                && alpha_equivalent_type(left_ty, left_impl, right_ty, right_impl)
                && alpha_equivalent_type_lists(left_args, left_impl, right_args, right_impl)
        }
        _ => left == right,
    }
}

fn alpha_equivalent_generic_param(
    left: GenericParamId,
    left_impl: DefId,
    right: GenericParamId,
    right_impl: DefId,
) -> bool {
    if left.owner == left_impl || right.owner == right_impl {
        left.owner == left_impl && right.owner == right_impl && left.index == right.index
    } else {
        left == right
    }
}

fn alpha_equivalent_bounds(
    left: &crate::hir::HirGenericBounds,
    left_impl: DefId,
    right: &crate::hir::HirGenericBounds,
    right_impl: DefId,
) -> bool {
    if left.len() != right.len() {
        return false;
    }

    let mut left_entries = left.iter().collect::<Vec<_>>();
    left_entries.sort_by_key(|(param, _)| (param.owner, param.index));
    let mut right_entries = right.iter().collect::<Vec<_>>();
    right_entries.sort_by_key(|(param, _)| (param.owner, param.index));
    left_entries.into_iter().all(|(left_param, left_bounds)| {
        let Some((_, right_bounds)) = right_entries.iter().find(|(right_param, _)| {
            alpha_equivalent_generic_param(*left_param, left_impl, **right_param, right_impl)
        }) else {
            return false;
        };
        alpha_equivalent_bound_multiset(left_bounds, left_impl, right_bounds, right_impl)
    })
}

fn alpha_equivalent_bound_multiset(
    left: &[TraitBound],
    left_impl: DefId,
    right: &[TraitBound],
    right_impl: DefId,
) -> bool {
    if left.len() != right.len() {
        return false;
    }

    let mut matched = vec![false; right.len()];
    left.iter().all(|left| {
        let Some(index) = right.iter().enumerate().position(|(index, right)| {
            !matched[index]
                && left.trait_id == right.trait_id
                && alpha_equivalent_type_lists(
                    &left.type_args,
                    left_impl,
                    &right.type_args,
                    right_impl,
                )
        }) else {
            return false;
        };
        matched[index] = true;
        true
    })
}

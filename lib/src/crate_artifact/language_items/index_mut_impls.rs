use super::product_def_id;
use crate::hir::{HirGenericBounds, HirImplReceiverPattern};
use crate::ids::{AssocTypeId, DefId};
use crate::products::{CompilerProducts, ProductDefId, ProductImplInterface, ProductLanguageItems};
use crate::types::{GenericParamId, TraitBound, Type};

pub(super) fn validate_product_index_mut_impls(
    products: &CompilerProducts,
    language_items: &ProductLanguageItems,
) -> Result<(), String> {
    let Some(index) = language_items.index.as_ref() else {
        return Ok(());
    };
    let Some(index_mut) = language_items.index_mut.as_ref() else {
        return Ok(());
    };
    let index_trait = product_def_id(index.trait_id);
    let index_mut_trait = product_def_id(index_mut.trait_id);

    let mut index_mut_impls = products
        .interface
        .impls
        .values()
        .filter(|imp| imp.trait_id == Some(index_mut_trait))
        .collect::<Vec<_>>();
    index_mut_impls.sort_by_key(|imp| imp.id);

    let mut index_impls = products
        .interface
        .impls
        .values()
        .filter(|imp| imp.trait_id == Some(index_trait))
        .collect::<Vec<_>>();
    index_impls.sort_by_key(|imp| imp.id);

    for index_mut_impl in index_mut_impls {
        let mut matches = index_impls
            .iter()
            .filter(|index_impl| {
                product_index_impls_are_alpha_equivalent(index_mut_impl, index_impl)
            })
            .copied()
            .collect::<Vec<_>>();
        matches.sort_by_key(|imp| imp.id);

        let key = index_mut_impl
            .trait_arg_types
            .first()
            .map(ToString::to_string)
            .unwrap_or_else(|| "<missing>".to_string());
        let context = format!(
            "Product artifact IndexMut implementation for {} with key {}",
            index_mut_impl.type_name, key
        );

        let Some(index_impl) = (match matches.as_slice() {
            [] => {
                return Err(format!(
                    "{context} requires a matching Index implementation"
                ));
            }
            [index_impl] => Some(*index_impl),
            matches => {
                let ids = matches
                    .iter()
                    .map(|imp| format!("{:?}", ProductDefId::from(imp.id)))
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "{context} has multiple matching Index implementations: {ids}"
                ));
            }
        }) else {
            continue;
        };

        let index_output_context = format!(
            "Product artifact Index implementation for {} with key {}",
            index_impl.type_name, key
        );
        let index_mut_output_context = format!(
            "Product artifact IndexMut implementation for {} with key {}",
            index_mut_impl.type_name, key
        );
        let index_output =
            product_associated_type(index_impl, index.output_id, &index_output_context)?;
        let index_mut_output = product_associated_type(
            index_mut_impl,
            index_mut.output_id,
            &index_mut_output_context,
        )?;
        if !alpha_equivalent_type(
            index_output,
            index_impl.id,
            index_mut_output,
            index_mut_impl.id,
        ) {
            return Err(format!(
                "Product artifact IndexMut implementation output {} does not match Index output {}",
                index_mut_output, index_output
            ));
        }
    }

    Ok(())
}

fn product_associated_type<'a>(
    imp: &'a ProductImplInterface,
    id: AssocTypeId,
    context: &str,
) -> Result<&'a Type, String> {
    let matches = imp
        .associated_types
        .iter()
        .filter(|associated| associated.id == id)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Err(format!(
            "{context} is missing marked associated output {id:?}"
        )),
        [associated] => Ok(&associated.ty),
        _ => Err(format!(
            "{context} has duplicate marked associated output {id:?}"
        )),
    }
}

fn product_index_impls_are_alpha_equivalent(
    left: &ProductImplInterface,
    right: &ProductImplInterface,
) -> bool {
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

fn alpha_equivalent_type(left: &Type, left_impl: DefId, right: &Type, right_impl: DefId) -> bool {
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
                ..
            },
            Type::Function {
                params: right_params,
                ret: right_ret,
                safety: right_safety,
                callable_kind: right_kind,
                captures: right_captures,
                ..
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
    left: &HirGenericBounds,
    left_impl: DefId,
    right: &HirGenericBounds,
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

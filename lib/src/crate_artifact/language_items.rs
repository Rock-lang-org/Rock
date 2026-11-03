use std::collections::{BTreeSet, HashMap};

use crate::hir::{
    HirGenericBounds, HirImplReceiverPattern, HirLanguageItems, HirNameTables, HirProgram,
};
use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId};
use crate::language_items::{
    DropLanguageItems, FnLanguageItems, FnMutLanguageItems, FnOnceLanguageItems, FoldLanguageItems,
    IndexLanguageItems, IndexMutLanguageItems, RangeLanguageItems, SendLanguageItems,
    SizedLanguageItems, SyncLanguageItems, TryLanguageItems,
};
use crate::products::{
    CompilerProducts, ProductCrateId, ProductDefId, ProductImplInterface, ProductLanguageItems,
};
use crate::types::{GenericParamId, TraitBound, Type};

use super::load::ProductIdentityRemap;

pub(super) fn language_items_from_products(
    products: &CompilerProducts,
    remap: &ProductIdentityRemap,
) -> Result<HirLanguageItems, String> {
    validate_product_language_items(products)?;

    let language_items = &products.interface.language_items;
    Ok(HirLanguageItems {
        fold: language_items
            .fold
            .as_ref()
            .map(|items| {
                Ok::<_, String>(FoldLanguageItems {
                    trait_id: remap.def_id(items.trait_id)?,
                    method_id: remap.def_id(items.method_id)?,
                })
            })
            .transpose()?,
        sized: language_items
            .sized
            .as_ref()
            .map(|items| {
                Ok::<_, String>(SizedLanguageItems {
                    trait_id: remap.def_id(items.trait_id)?,
                })
            })
            .transpose()?,
        drop: language_items
            .drop
            .as_ref()
            .map(|items| {
                Ok::<_, String>(DropLanguageItems {
                    trait_id: remap.def_id(items.trait_id)?,
                    method_id: remap.def_id(items.method_id)?,
                })
            })
            .transpose()?,
        index: language_items
            .index
            .as_ref()
            .map(|items| {
                Ok::<_, String>(IndexLanguageItems {
                    trait_id: remap.def_id(items.trait_id)?,
                    output_id: items.output_id,
                    method_id: remap.def_id(items.method_id)?,
                })
            })
            .transpose()?,
        index_mut: language_items
            .index_mut
            .as_ref()
            .map(|items| {
                Ok::<_, String>(IndexMutLanguageItems {
                    trait_id: remap.def_id(items.trait_id)?,
                    output_id: items.output_id,
                    method_id: remap.def_id(items.method_id)?,
                })
            })
            .transpose()?,
        fn_once: language_items
            .fn_once
            .as_ref()
            .map(|items| {
                Ok::<_, String>(FnOnceLanguageItems {
                    trait_id: remap.def_id(items.trait_id)?,
                    output_id: items.output_id,
                    method_id: remap.def_id(items.method_id)?,
                })
            })
            .transpose()?,
        fn_mut: language_items
            .fn_mut
            .as_ref()
            .map(|items| {
                Ok::<_, String>(FnMutLanguageItems {
                    trait_id: remap.def_id(items.trait_id)?,
                    output_id: items.output_id,
                    method_id: remap.def_id(items.method_id)?,
                })
            })
            .transpose()?,
        fn_trait: language_items
            .fn_trait
            .as_ref()
            .map(|items| {
                Ok::<_, String>(FnLanguageItems {
                    trait_id: remap.def_id(items.trait_id)?,
                    output_id: items.output_id,
                    method_id: remap.def_id(items.method_id)?,
                })
            })
            .transpose()?,
        send: language_items
            .send
            .as_ref()
            .map(|items| {
                Ok::<_, String>(SendLanguageItems {
                    trait_id: remap.def_id(items.trait_id)?,
                })
            })
            .transpose()?,
        sync: language_items
            .sync
            .as_ref()
            .map(|items| {
                Ok::<_, String>(SyncLanguageItems {
                    trait_id: remap.def_id(items.trait_id)?,
                })
            })
            .transpose()?,
        try_protocol: language_items
            .try_protocol
            .as_ref()
            .map(|items| {
                Ok::<_, String>(TryLanguageItems {
                    try_trait_id: remap.def_id(items.try_trait_id)?,
                    output_id: items.output_id,
                    residual_id: items.residual_id,
                    branch_method_id: remap.def_id(items.branch_method_id)?,
                    from_residual_trait_id: remap.def_id(items.from_residual_trait_id)?,
                    from_residual_method_id: remap.def_id(items.from_residual_method_id)?,
                    control_flow_enum_id: remap.def_id(items.control_flow_enum_id)?,
                    break_variant_id: items.break_variant_id,
                    continue_variant_id: items.continue_variant_id,
                })
            })
            .transpose()?,
        range: language_items
            .range
            .as_ref()
            .map(|items| {
                Ok::<_, String>(RangeLanguageItems {
                    enum_id: remap.def_id(items.enum_id)?,
                    full_variant_id: items.full_variant_id,
                    from_variant_id: items.from_variant_id,
                    to_variant_id: items.to_variant_id,
                    to_inclusive_variant_id: items.to_inclusive_variant_id,
                    exclusive_variant_id: items.exclusive_variant_id,
                    inclusive_variant_id: items.inclusive_variant_id,
                })
            })
            .transpose()?,
    })
}

pub(super) fn validate_product_language_items(products: &CompilerProducts) -> Result<(), String> {
    let language_items = &products.interface.language_items;
    if language_items.sized.is_none()
        && language_items.drop.is_none()
        && language_items.fold.is_none()
        && language_items.index.is_none()
        && language_items.index_mut.is_none()
        && language_items.fn_once.is_none()
        && language_items.fn_mut.is_none()
        && language_items.fn_trait.is_none()
        && language_items.send.is_none()
        && language_items.sync.is_none()
        && language_items.try_protocol.is_none()
        && language_items.range.is_none()
    {
        return Ok(());
    }

    let local_crate = products.identity_table.local_crate.ok_or_else(|| {
        format!(
            "Product artifact for crate '{}' has no local product crate ID",
            products.crate_identity.name
        )
    })?;
    validate_product_local_ids(language_items, local_crate)?;
    validate_product_index_mut_impls(products, language_items)?;

    let program = product_language_item_program(products)?;
    validate_bundle(
        &program,
        language_items.fold.as_ref().map(|items| {
            (
                "fold.trait",
                items.trait_id,
                HirLanguageItems {
                    fold: Some(FoldLanguageItems {
                        trait_id: product_def_id(items.trait_id),
                        method_id: product_def_id(items.method_id),
                    }),
                    ..HirLanguageItems::default()
                },
            )
        }),
    )?;
    validate_bundle(
        &program,
        language_items.sized.as_ref().map(|items| {
            (
                "sized.trait",
                items.trait_id,
                HirLanguageItems {
                    sized: Some(SizedLanguageItems {
                        trait_id: product_def_id(items.trait_id),
                    }),
                    ..HirLanguageItems::default()
                },
            )
        }),
    )?;
    validate_bundle(
        &program,
        language_items.drop.as_ref().map(|items| {
            (
                "drop.trait",
                items.trait_id,
                HirLanguageItems {
                    drop: Some(DropLanguageItems {
                        trait_id: product_def_id(items.trait_id),
                        method_id: product_def_id(items.method_id),
                    }),
                    ..HirLanguageItems::default()
                },
            )
        }),
    )?;
    validate_bundle(
        &program,
        language_items.index.as_ref().map(|items| {
            (
                "index.trait",
                items.trait_id,
                HirLanguageItems {
                    index: Some(IndexLanguageItems {
                        trait_id: product_def_id(items.trait_id),
                        output_id: items.output_id,
                        method_id: product_def_id(items.method_id),
                    }),
                    ..HirLanguageItems::default()
                },
            )
        }),
    )?;
    validate_bundle(
        &program,
        language_items.index_mut.as_ref().map(|items| {
            (
                "index_mut.trait",
                items.trait_id,
                HirLanguageItems {
                    index: language_items
                        .index
                        .as_ref()
                        .map(|index| IndexLanguageItems {
                            trait_id: product_def_id(index.trait_id),
                            output_id: index.output_id,
                            method_id: product_def_id(index.method_id),
                        }),
                    index_mut: Some(IndexMutLanguageItems {
                        trait_id: product_def_id(items.trait_id),
                        output_id: items.output_id,
                        method_id: product_def_id(items.method_id),
                    }),
                    ..HirLanguageItems::default()
                },
            )
        }),
    )?;
    validate_bundle(
        &program,
        language_items.fn_once.as_ref().map(|items| {
            (
                "fn_once.trait",
                items.trait_id,
                HirLanguageItems {
                    fn_once: Some(FnOnceLanguageItems {
                        trait_id: product_def_id(items.trait_id),
                        output_id: items.output_id,
                        method_id: product_def_id(items.method_id),
                    }),
                    ..HirLanguageItems::default()
                },
            )
        }),
    )?;
    validate_bundle(
        &program,
        language_items.fn_mut.as_ref().map(|items| {
            (
                "fn_mut.trait",
                items.trait_id,
                HirLanguageItems {
                    fn_mut: Some(FnMutLanguageItems {
                        trait_id: product_def_id(items.trait_id),
                        output_id: items.output_id,
                        method_id: product_def_id(items.method_id),
                    }),
                    ..HirLanguageItems::default()
                },
            )
        }),
    )?;
    validate_bundle(
        &program,
        language_items.fn_trait.as_ref().map(|items| {
            (
                "fn.trait",
                items.trait_id,
                HirLanguageItems {
                    fn_trait: Some(FnLanguageItems {
                        trait_id: product_def_id(items.trait_id),
                        output_id: items.output_id,
                        method_id: product_def_id(items.method_id),
                    }),
                    ..HirLanguageItems::default()
                },
            )
        }),
    )?;
    validate_bundle(
        &program,
        language_items.send.as_ref().map(|items| {
            (
                "send.trait",
                items.trait_id,
                HirLanguageItems {
                    send: Some(SendLanguageItems {
                        trait_id: product_def_id(items.trait_id),
                    }),
                    ..HirLanguageItems::default()
                },
            )
        }),
    )?;
    validate_bundle(
        &program,
        language_items.sync.as_ref().map(|items| {
            (
                "sync.trait",
                items.trait_id,
                HirLanguageItems {
                    sync: Some(SyncLanguageItems {
                        trait_id: product_def_id(items.trait_id),
                    }),
                    ..HirLanguageItems::default()
                },
            )
        }),
    )?;
    validate_bundle(
        &program,
        language_items.try_protocol.as_ref().map(|items| {
            (
                "try.trait",
                items.try_trait_id,
                HirLanguageItems {
                    try_protocol: Some(TryLanguageItems {
                        try_trait_id: product_def_id(items.try_trait_id),
                        output_id: items.output_id,
                        residual_id: items.residual_id,
                        branch_method_id: product_def_id(items.branch_method_id),
                        from_residual_trait_id: product_def_id(items.from_residual_trait_id),
                        from_residual_method_id: product_def_id(items.from_residual_method_id),
                        control_flow_enum_id: product_def_id(items.control_flow_enum_id),
                        break_variant_id: items.break_variant_id,
                        continue_variant_id: items.continue_variant_id,
                    }),
                    ..HirLanguageItems::default()
                },
            )
        }),
    )?;
    if let Some(items) = &language_items.range {
        let enum_id = product_def_id(items.enum_id);
        let range_enum = program.enums.get(&enum_id).ok_or_else(|| {
            format!("Product artifact language item range.enum references missing enum {enum_id:?}")
        })?;
        for (role, variant_id, arity) in [
            ("range.range_full", items.full_variant_id, 0usize),
            ("range.range_from", items.from_variant_id, 1),
            ("range.range_to", items.to_variant_id, 1),
            ("range.range_to_inclusive", items.to_inclusive_variant_id, 1),
            ("range.range_exclusive", items.exclusive_variant_id, 2),
            ("range.range_inclusive", items.inclusive_variant_id, 2),
        ] {
            let variant = range_enum
                .variants
                .iter()
                .find(|variant| variant.id == variant_id)
                .ok_or_else(|| {
                    format!(
                        "Product artifact language item {role} references missing variant {variant_id:?}"
                    )
                })?;
            let fields = match &variant.fields {
                crate::hir::HirVariantFields::Unit => Vec::new(),
                crate::hir::HirVariantFields::Positional(fields) => fields.clone(),
                crate::hir::HirVariantFields::Named(fields) => {
                    fields.iter().map(|field| field.ty.clone()).collect()
                }
            };
            if fields.len() != arity || fields.iter().any(|field| field != &Type::I64) {
                return Err(format!(
                    "Product artifact language item {role} must have {arity} I64 fields"
                ));
            }
        }
    }

    Ok(())
}

fn validate_product_index_mut_impls(
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
            left_len == right_len && alpha_equivalent_type(left, left_impl, right, right_impl)
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

fn validate_product_local_ids(
    language_items: &ProductLanguageItems,
    local_crate: ProductCrateId,
) -> Result<(), String> {
    let mut ids = Vec::new();
    if let Some(items) = &language_items.fold {
        ids.extend([
            ("fold.trait", items.trait_id),
            ("fold.method", items.method_id),
        ]);
    }
    if let Some(items) = &language_items.sized {
        ids.push(("sized.trait", items.trait_id));
    }
    if let Some(items) = &language_items.drop {
        ids.extend([
            ("drop.trait", items.trait_id),
            ("drop.method", items.method_id),
        ]);
    }
    if let Some(items) = &language_items.index {
        ids.extend([
            ("index.trait", items.trait_id),
            ("index.method", items.method_id),
        ]);
    }
    if let Some(items) = &language_items.index_mut {
        ids.extend([
            ("index_mut.trait", items.trait_id),
            ("index_mut.method", items.method_id),
        ]);
    }
    if let Some(items) = &language_items.fn_once {
        ids.extend([
            ("fn_once.trait", items.trait_id),
            ("fn_once.method", items.method_id),
        ]);
    }
    if let Some(items) = &language_items.fn_mut {
        ids.extend([
            ("fn_mut.trait", items.trait_id),
            ("fn_mut.method", items.method_id),
        ]);
    }
    if let Some(items) = &language_items.fn_trait {
        ids.extend([("fn.trait", items.trait_id), ("fn.method", items.method_id)]);
    }
    if let Some(items) = &language_items.send {
        ids.push(("send.trait", items.trait_id));
    }
    if let Some(items) = &language_items.sync {
        ids.push(("sync.trait", items.trait_id));
    }
    if let Some(items) = &language_items.try_protocol {
        ids.extend([
            ("try.trait", items.try_trait_id),
            ("try.branch", items.branch_method_id),
            ("from_residual.trait", items.from_residual_trait_id),
            ("from_residual.method", items.from_residual_method_id),
            ("control_flow.enum", items.control_flow_enum_id),
        ]);
    }
    if let Some(items) = &language_items.range {
        ids.push(("range.enum", items.enum_id));
    }

    for (role, id) in ids {
        if id.crate_id != local_crate {
            return Err(format!(
                "Product artifact language item {role} uses non-local {id:?}; expected product crate {}",
                local_crate.0
            ));
        }
    }
    Ok(())
}

fn product_language_item_program(products: &CompilerProducts) -> Result<HirProgram, String> {
    let ids = product_language_item_ids(products);
    let functions = products
        .interface
        .functions
        .iter()
        .filter(|(id, _)| ids.contains(id))
        .map(|(id, function)| {
            validate_embedded_id("function", *id, function.id)?;
            Ok((
                product_def_id(*id),
                crate::crate_artifact::types::hir_function_from_interface(function),
            ))
        })
        .collect::<Result<HashMap<_, _>, String>>()?;
    let structs = products
        .interface
        .structs
        .iter()
        .filter(|(id, _)| ids.contains(id))
        .map(|(id, strukt)| {
            validate_embedded_id("struct", *id, strukt.id)?;
            Ok((
                product_def_id(*id),
                crate::crate_artifact::types::hir_struct_from_interface(strukt),
            ))
        })
        .collect::<Result<HashMap<_, _>, String>>()?;
    let enums = products
        .interface
        .enums
        .iter()
        .filter(|(id, _)| ids.contains(id))
        .map(|(id, enm)| {
            validate_embedded_id("enum", *id, enm.id)?;
            Ok((
                product_def_id(*id),
                crate::crate_artifact::types::hir_enum_from_interface(enm),
            ))
        })
        .collect::<Result<HashMap<_, _>, String>>()?;
    let traits = products
        .interface
        .traits
        .iter()
        .filter(|(id, _)| ids.contains(id))
        .map(|(id, trt)| {
            validate_embedded_id("trait", *id, trt.id)?;
            Ok((
                product_def_id(*id),
                crate::crate_artifact::types::hir_trait_from_interface(trt),
            ))
        })
        .collect::<Result<HashMap<_, _>, String>>()?;
    let impls = products
        .interface
        .impls
        .iter()
        .filter(|(id, _)| ids.contains(id))
        .map(|(id, imp)| {
            validate_embedded_id("impl", *id, imp.id)?;
            Ok((
                product_def_id(*id),
                crate::crate_artifact::types::hir_impl_from_interface(imp),
            ))
        })
        .collect::<Result<HashMap<_, _>, String>>()?;
    let externs = products
        .interface
        .externs
        .iter()
        .filter(|(id, _)| ids.contains(id))
        .map(|(id, ext)| {
            validate_embedded_id("extern", *id, ext.id)?;
            Ok((
                product_def_id(*id),
                crate::crate_artifact::types::hir_extern_from_interface(ext),
            ))
        })
        .collect::<Result<HashMap<_, _>, String>>()?;

    Ok(HirProgram::from_id_parts_with_names_and_canonical_names(
        functions,
        structs,
        enums,
        traits,
        impls,
        externs,
        HirNameTables::default(),
        HirLanguageItems::default(),
        &HashMap::new(),
    ))
}

fn product_language_item_ids(products: &CompilerProducts) -> BTreeSet<ProductDefId> {
    let language_items = &products.interface.language_items;
    let mut ids = BTreeSet::new();
    let mut member_ids = BTreeSet::new();
    let mut assoc_ids = BTreeSet::new();
    let mut variant_ids = BTreeSet::new();
    if let Some(items) = &language_items.fold {
        ids.insert(items.trait_id);
        ids.insert(items.method_id);
        member_ids.insert(items.method_id);
    }
    if let Some(items) = &language_items.sized {
        ids.insert(items.trait_id);
    }
    if let Some(items) = &language_items.drop {
        ids.insert(items.trait_id);
        ids.insert(items.method_id);
        member_ids.insert(items.method_id);
    }
    if let Some(items) = &language_items.index {
        ids.insert(items.trait_id);
        ids.insert(items.method_id);
        member_ids.insert(items.method_id);
        assoc_ids.insert(items.output_id);
    }
    if let Some(items) = &language_items.index_mut {
        ids.insert(items.trait_id);
        ids.insert(items.method_id);
        member_ids.insert(items.method_id);
        assoc_ids.insert(items.output_id);
    }
    if let Some(items) = &language_items.fn_once {
        ids.insert(items.trait_id);
        ids.insert(items.method_id);
        member_ids.insert(items.method_id);
        assoc_ids.insert(items.output_id);
    }
    if let Some(items) = &language_items.fn_mut {
        ids.insert(items.trait_id);
        ids.insert(items.method_id);
        member_ids.insert(items.method_id);
        assoc_ids.insert(items.output_id);
    }
    if let Some(items) = &language_items.fn_trait {
        ids.insert(items.trait_id);
        ids.insert(items.method_id);
        member_ids.insert(items.method_id);
        assoc_ids.insert(items.output_id);
    }
    if let Some(items) = &language_items.send {
        ids.insert(items.trait_id);
    }
    if let Some(items) = &language_items.sync {
        ids.insert(items.trait_id);
    }
    if let Some(items) = &language_items.try_protocol {
        ids.insert(items.try_trait_id);
        ids.insert(items.branch_method_id);
        ids.insert(items.from_residual_trait_id);
        ids.insert(items.from_residual_method_id);
        ids.insert(items.control_flow_enum_id);
        member_ids.insert(items.branch_method_id);
        member_ids.insert(items.from_residual_method_id);
        assoc_ids.insert(items.output_id);
        assoc_ids.insert(items.residual_id);
        variant_ids.insert(items.break_variant_id);
        variant_ids.insert(items.continue_variant_id);
    }
    if let Some(items) = &language_items.range {
        ids.insert(items.enum_id);
        variant_ids.extend([
            items.full_variant_id,
            items.from_variant_id,
            items.to_variant_id,
            items.to_inclusive_variant_id,
            items.exclusive_variant_id,
            items.inclusive_variant_id,
        ]);
    }

    for (trait_id, trait_def) in &products.interface.traits {
        let owns_member = trait_def
            .methods
            .values()
            .any(|method| member_ids.contains(&ProductDefId::from(method.id)))
            || trait_def
                .signatures
                .values()
                .any(|signature| member_ids.contains(&ProductDefId::from(signature.id)));
        let owns_associated_type = trait_def
            .associated_types
            .iter()
            .any(|associated_type| assoc_ids.contains(&associated_type.id));
        if owns_member || owns_associated_type {
            ids.insert(*trait_id);
        }
    }
    for (enum_id, enum_def) in &products.interface.enums {
        if enum_def
            .variants
            .iter()
            .any(|variant| variant_ids.contains(&variant.id))
        {
            ids.insert(*enum_id);
        }
    }

    ids
}

fn validate_embedded_id(
    row_kind: &str,
    row_id: ProductDefId,
    embedded_id: DefId,
) -> Result<(), String> {
    let embedded_id = ProductDefId::from(embedded_id);
    if embedded_id != row_id {
        return Err(format!(
            "Product artifact language-item interface {row_kind} row {row_id:?} has embedded {embedded_id:?}"
        ));
    }
    Ok(())
}

fn validate_bundle(
    program: &HirProgram,
    bundle: Option<(&str, ProductDefId, HirLanguageItems)>,
) -> Result<(), String> {
    let Some((role, id, language_items)) = bundle else {
        return Ok(());
    };
    let mut program = program.clone();
    program.language_items = language_items;
    let errors = crate::hir::language_items::validate_language_items(&program);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Product artifact language item {role} {id:?}: {}",
            errors.join("; ")
        ))
    }
}

fn product_def_id(id: ProductDefId) -> DefId {
    DefId::new(CrateId(id.crate_id.0), LocalDefId(id.local_id.0))
}

#[cfg(test)]
mod tests;

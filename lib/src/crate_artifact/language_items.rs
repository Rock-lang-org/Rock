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
        object_owner: language_items.object_owner.as_ref().map(|items| Ok::<_, String>(crate::language_items::ObjectOwnerLanguageItems { trait_id: remap.def_id(items.trait_id)?, into_parts_id: remap.def_id(items.into_parts_id)?, from_parts_id: remap.def_id(items.from_parts_id)? })).transpose()?,
        object_owner_allocate: language_items.object_owner_allocate.as_ref().map(|items| Ok::<_, String>(crate::language_items::OwnerOperationLanguageItems { trait_id: remap.def_id(items.trait_id)?, method_id: remap.def_id(items.method_id)? })).transpose()?,
        object_owner_unique: language_items.object_owner_unique.as_ref().map(|items| Ok::<_, String>(crate::language_items::OwnerOperationLanguageItems { trait_id: remap.def_id(items.trait_id)?, method_id: remap.def_id(items.method_id)? })).transpose()?,
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
        && language_items.object_owner.is_none()
        && language_items.object_owner_allocate.is_none()
        && language_items.object_owner_unique.is_none()
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
    let owner_items = HirLanguageItems {
        object_owner: language_items.object_owner.as_ref().map(|items| crate::language_items::ObjectOwnerLanguageItems { trait_id: product_def_id(items.trait_id), into_parts_id: product_def_id(items.into_parts_id), from_parts_id: product_def_id(items.from_parts_id) }),
        object_owner_allocate: language_items.object_owner_allocate.as_ref().map(|items| crate::language_items::OwnerOperationLanguageItems { trait_id: product_def_id(items.trait_id), method_id: product_def_id(items.method_id) }),
        object_owner_unique: language_items.object_owner_unique.as_ref().map(|items| crate::language_items::OwnerOperationLanguageItems { trait_id: product_def_id(items.trait_id), method_id: product_def_id(items.method_id) }),
        ..HirLanguageItems::default()
    };
    if let Some(id) = language_items.object_owner.as_ref().map(|items| items.trait_id).or_else(|| language_items.object_owner_allocate.as_ref().map(|items| items.trait_id)).or_else(|| language_items.object_owner_unique.as_ref().map(|items| items.trait_id)) {
        validate_bundle(&program, Some(("object_owner", id, owner_items)))?;
    }
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
    if let Some(items) = &language_items.object_owner { ids.extend([("object_owner.trait", items.trait_id), ("object_owner.into_parts", items.into_parts_id), ("object_owner.from_parts", items.from_parts_id)]); }
    if let Some(items) = &language_items.object_owner_allocate { ids.extend([("object_owner_allocate.trait", items.trait_id), ("object_owner_allocate.method", items.method_id)]); }
    if let Some(items) = &language_items.object_owner_unique { ids.extend([("object_owner_unique.trait", items.trait_id), ("object_owner_unique.method", items.method_id)]); }
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
    if let Some(items) = &language_items.object_owner {
        ids.extend([items.trait_id, items.into_parts_id, items.from_parts_id]);
        member_ids.extend([items.into_parts_id, items.from_parts_id]);
    }
    for items in [&language_items.object_owner_allocate, &language_items.object_owner_unique].into_iter().flatten() {
        ids.extend([items.trait_id, items.method_id]);
        member_ids.insert(items.method_id);
    }
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
mod tests {
    use std::collections::{BTreeMap, HashMap};
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::hir::{
        HirAssociatedTypeDecl, HirEnum, HirFunctionSig, HirImplOwner, HirImplReceiverPattern,
        HirTrait, HirVariant, HirVariantFields,
    };
    use crate::ids::{AssocTypeId, CrateId, VariantId};
    use crate::language_items::{
        DropLanguageItems, IndexLanguageItems, IndexMutLanguageItems, LanguageItems,
        SizedLanguageItems, TryLanguageItems,
    };
    use crate::products::{
        CompilerProducts, ProductAssociatedTypeInterface, ProductBodies, ProductCrateId,
        ProductCrateIdentity, ProductFunctionInterface, ProductIdentityTable, ProductImplInterface,
        ProductInterface, ProductLinkData, ProductLinkRecord, ProductLocalDefId,
        ProductSourceFingerprint, ProductStructInterface,
    };
    use crate::types::ReceiverMode;
    use crate::types::{AssociatedTypeKey, GenericParamDecl, GenericParamId, TraitBound, Type};

    use super::{
        product_def_id, validate_product_index_mut_impls, validate_product_language_items,
    };

    static ARTIFACT_CASE_ID: AtomicU64 = AtomicU64::new(0);

    fn product_id(local_id: u32) -> crate::products::ProductDefId {
        crate::products::ProductDefId {
            crate_id: ProductCrateId(0),
            local_id: ProductLocalDefId(local_id),
        }
    }

    fn signature(
        id: crate::products::ProductDefId,
        receiver: Option<ReceiverMode>,
        params: Vec<Type>,
        ret: Type,
    ) -> HirFunctionSig {
        HirFunctionSig {
            id: product_def_id(id),
            name: format!("m{}", id.local_id.0),
            generic_params: Vec::new(),
            params,
            ret,
            generic_bounds: HashMap::new().into(),
            self_receiver: receiver,
            is_unsafe: false,
        }
    }

    fn self_ty(id: crate::products::ProductDefId, index: u32) -> Type {
        Type::Generic(GenericParamId {
            owner: product_def_id(id),
            index,
        })
    }

    fn trait_def(
        id: crate::products::ProductDefId,
        generic_params: Vec<&str>,
        associated_types: Vec<AssocTypeId>,
        signatures: Vec<HirFunctionSig>,
    ) -> HirTrait {
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: product_def_id(id),
            name: format!("T{}", id.local_id.0),
            generic_params: GenericParamDecl::type_params(
                product_def_id(id),
                generic_params.into_iter(),
            ),
            associated_types: associated_types
                .into_iter()
                .map(|id| HirAssociatedTypeDecl {
                    id,
                    name: format!("A{}", id.0),
                    kind: crate::type_services::kind::Kind::Type,
                })
                .collect(),
            methods: HashMap::new(),
            signatures: signatures
                .into_iter()
                .map(|signature| (signature.name.clone(), signature))
                .collect(),
        }
    }

    fn valid_products() -> CompilerProducts {
        let sized = product_id(1);
        let drop_trait = product_id(2);
        let drop_method = product_id(3);
        let index_trait = product_id(4);
        let index_method = product_id(5);
        let try_trait = product_id(6);
        let branch_method = product_id(7);
        let residual_trait = product_id(8);
        let residual_method = product_id(9);
        let control_flow = product_id(10);
        let index_mut_trait = product_id(11);
        let index_mut_method = product_id(12);

        let index_key = Type::Generic(GenericParamId {
            owner: product_def_id(index_trait),
            index: 0,
        });
        let index_self = self_ty(index_trait, 1);
        let index_output = Type::Reference {
            mutable: false,
            inner: Box::new(Type::Projection {
                ty: Box::new(index_self.clone()),
                trait_id: product_def_id(index_trait),
                assoc_type: AssociatedTypeKey {
                    owner: product_def_id(index_trait),
                    assoc_type_id: AssocTypeId(0),
                },
                trait_args: vec![index_key.clone()],
            }),
        };
        let index_mut_key = Type::Generic(GenericParamId {
            owner: product_def_id(index_mut_trait),
            index: 0,
        });
        let index_mut_self = self_ty(index_mut_trait, 1);
        let index_mut_output = Type::Reference {
            mutable: true,
            inner: Box::new(Type::Projection {
                ty: Box::new(index_mut_self.clone()),
                trait_id: product_def_id(index_mut_trait),
                assoc_type: AssociatedTypeKey {
                    owner: product_def_id(index_mut_trait),
                    assoc_type_id: AssocTypeId(0),
                },
                trait_args: vec![index_mut_key.clone()],
            }),
        };
        let try_projection = |assoc_type_id| Type::Projection {
            ty: Box::new(self_ty(try_trait, 0)),
            trait_id: product_def_id(try_trait),
            assoc_type: AssociatedTypeKey {
                owner: product_def_id(try_trait),
                assoc_type_id,
            },
            trait_args: Vec::new(),
        };

        let mut interface = ProductInterface::default();
        interface.insert_trait(
            sized,
            crate::products::ProductTraitInterface::from(&trait_def(
                sized,
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )),
        );
        interface.insert_trait(
            drop_trait,
            crate::products::ProductTraitInterface::from(&trait_def(
                drop_trait,
                Vec::new(),
                Vec::new(),
                vec![signature(
                    drop_method,
                    Some(ReceiverMode::Move),
                    vec![self_ty(drop_trait, 0)],
                    Type::Unit,
                )],
            )),
        );
        interface.insert_trait(
            index_trait,
            crate::products::ProductTraitInterface::from(&trait_def(
                index_trait,
                vec!["Key"],
                vec![AssocTypeId(0)],
                vec![signature(
                    index_method,
                    Some(ReceiverMode::Shared),
                    vec![
                        Type::Reference {
                            mutable: false,
                            inner: Box::new(index_self),
                        },
                        index_key,
                    ],
                    index_output,
                )],
            )),
        );
        interface.insert_trait(
            index_mut_trait,
            crate::products::ProductTraitInterface::from(&trait_def(
                index_mut_trait,
                vec!["Key"],
                vec![AssocTypeId(0)],
                vec![signature(
                    index_mut_method,
                    Some(ReceiverMode::Mut),
                    vec![
                        Type::Reference {
                            mutable: true,
                            inner: Box::new(index_mut_self),
                        },
                        index_mut_key,
                    ],
                    index_mut_output,
                )],
            )),
        );
        interface.insert_trait(
            try_trait,
            crate::products::ProductTraitInterface::from(&trait_def(
                try_trait,
                Vec::new(),
                vec![AssocTypeId(0), AssocTypeId(1)],
                vec![signature(
                    branch_method,
                    Some(ReceiverMode::Move),
                    vec![self_ty(try_trait, 0)],
                    Type::Enum {
                        id: product_def_id(control_flow),
                        args: vec![
                            try_projection(AssocTypeId(1)),
                            try_projection(AssocTypeId(0)),
                        ],
                    },
                )],
            )),
        );
        interface.insert_trait(
            residual_trait,
            crate::products::ProductTraitInterface::from(&trait_def(
                residual_trait,
                vec!["R"],
                Vec::new(),
                vec![signature(
                    residual_method,
                    None,
                    vec![self_ty(residual_trait, 0)],
                    self_ty(residual_trait, 1),
                )],
            )),
        );
        interface.enums.insert(
            control_flow,
            crate::products::ProductEnumInterface::from(&HirEnum {
                id: product_def_id(control_flow),
                name: "ControlFlow".to_string(),
                generic_params: GenericParamDecl::type_params(
                    product_def_id(control_flow),
                    ["B", "C"].into_iter(),
                ),
                variants: vec![
                    HirVariant {
                        id: VariantId(0),
                        name: "Break".to_string(),
                        fields: HirVariantFields::Positional(vec![self_ty(control_flow, 0)]),
                    },
                    HirVariant {
                        id: VariantId(1),
                        name: "Continue".to_string(),
                        fields: HirVariantFields::Positional(vec![self_ty(control_flow, 1)]),
                    },
                ],
            }),
        );
        interface.language_items = LanguageItems {
            object_owner: None,
            object_owner_allocate: None,
            object_owner_unique: None,
            fold: None,
            sized: Some(SizedLanguageItems { trait_id: sized }),
            drop: Some(DropLanguageItems {
                trait_id: drop_trait,
                method_id: drop_method,
            }),
            index: Some(IndexLanguageItems {
                trait_id: index_trait,
                output_id: AssocTypeId(0),
                method_id: index_method,
            }),
            index_mut: Some(IndexMutLanguageItems {
                trait_id: index_mut_trait,
                output_id: AssocTypeId(0),
                method_id: index_mut_method,
            }),
            fn_once: None,
            fn_mut: None,
            fn_trait: None,
            send: None,
            sync: None,
            try_protocol: Some(TryLanguageItems {
                try_trait_id: try_trait,
                output_id: AssocTypeId(0),
                residual_id: AssocTypeId(1),
                branch_method_id: branch_method,
                from_residual_trait_id: residual_trait,
                from_residual_method_id: residual_method,
                control_flow_enum_id: control_flow,
                break_variant_id: VariantId(0),
                continue_variant_id: VariantId(1),
            }),
            range: None,
        };

        CompilerProducts {
            crate_identity: ProductCrateIdentity::local("protocol".to_string()),
            identity_table: ProductIdentityTable {
                local_crate: Some(ProductCrateId(0)),
                ..ProductIdentityTable::default()
            },
            interface,
            bodies: ProductBodies::default(),
            link: ProductLinkData::default(),
            dependencies: Vec::new(),
            source_fingerprint: ProductSourceFingerprint::default(),
            infix_precedence: Default::default(),
            proc_macros: Vec::new(),
        }
    }

    fn product_index_impl_with_outputs(
        id: u32,
        trait_id: crate::products::ProductDefId,
        associated_types: Vec<ProductAssociatedTypeInterface>,
    ) -> ProductImplInterface {
        let impl_id = product_id(id);
        ProductImplInterface {
            id: product_def_id(impl_id),
            owner: HirImplOwner::Named("Cell".to_string()),
            type_name: "Cell".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: product_def_id(product_id(20)),
                args: Vec::new(),
            }),
            trait_name: Some("RenamedIndex".to_string()),
            trait_id: Some(product_def_id(trait_id)),
            trait_generics: Vec::new(),
            trait_arg_types: vec![Type::I64],
            associated_types,
            bounds: HashMap::new().into(),
            methods: BTreeMap::new(),
        }
    }

    fn product_index_impl_with_bounds(
        id: u32,
        trait_id: crate::products::ProductDefId,
        bounds: Vec<TraitBound>,
    ) -> ProductImplInterface {
        let impl_id = product_id(id);
        let impl_def_id = product_def_id(impl_id);
        let generic = GenericParamId {
            owner: impl_def_id,
            index: 0,
        };
        ProductImplInterface {
            id: impl_def_id,
            owner: HirImplOwner::Named("Cell".to_string()),
            type_name: "Cell".to_string(),
            type_generics: vec![GenericParamDecl::type_param(generic, "T")],
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: product_def_id(product_id(20)),
                args: vec![Type::Generic(generic)],
            }),
            trait_name: Some("RenamedIndex".to_string()),
            trait_id: Some(product_def_id(trait_id)),
            trait_generics: Vec::new(),
            trait_arg_types: vec![Type::I64],
            associated_types: vec![ProductAssociatedTypeInterface {
                id: AssocTypeId(0),
                name: "Value".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::Generic(generic),
            }],
            bounds: HashMap::from([(generic, bounds)]).into(),
            methods: BTreeMap::new(),
        }
    }

    fn add_product_index_impl(
        products: &mut CompilerProducts,
        id: u32,
        trait_id: crate::products::ProductDefId,
        associated_types: Vec<ProductAssociatedTypeInterface>,
    ) {
        let is_mut = trait_id == product_id(11);
        let trait_method_id = if is_mut {
            product_id(12)
        } else {
            product_id(5)
        };
        let impl_method_id = product_id(id + 100);
        let impl_id = product_id(id);
        let mut imp = product_index_impl_with_outputs(id, trait_id, associated_types);
        imp.methods.insert(
            if is_mut {
                "write_at".to_string()
            } else {
                "read_at".to_string()
            },
            ProductFunctionInterface {
                id: product_def_id(impl_method_id),
                name: if is_mut {
                    "write_at".to_string()
                } else {
                    "read_at".to_string()
                },
                generic_params: Vec::new(),
                generic_bounds: HashMap::new().into(),
                params: vec![
                    Type::Reference {
                        mutable: is_mut,
                        inner: Box::new(Type::Struct {
                            id: product_def_id(product_id(20)),
                            args: Vec::new(),
                        }),
                    },
                    Type::I64,
                ],
                ret_type: Type::Reference {
                    mutable: is_mut,
                    inner: Box::new(Type::I64),
                },
                is_curried: false,
                is_method: true,
                self_receiver: Some(if is_mut {
                    ReceiverMode::Mut
                } else {
                    ReceiverMode::Shared
                }),
                is_unsafe: false,
            },
        );
        products.interface.impls.insert(impl_id, imp);
        products
            .interface
            .effective_trait_methods
            .insert((impl_id, trait_method_id), impl_method_id);
        products.link.records.insert(
            impl_method_id,
            ProductLinkRecord {
                backend_symbol: format!("__rock_fn_index_fixture_{id}"),
            },
        );
    }

    fn assert_loader_error_in_both_modes(products: &CompilerProducts, expected: &str) {
        let case_id = ARTIFACT_CASE_ID.fetch_add(1, Ordering::SeqCst);
        let base = std::env::temp_dir().join(format!(
            "rock_language_item_loader_{}_{}",
            std::process::id(),
            case_id
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("temporary artifact directory");
        let artifact_path = base.join("protocol.rkca");
        products
            .write_artifact_to_path(&artifact_path)
            .expect("serialized malformed product artifact");

        for alias_mode in [false, true] {
            let mut crate_context = crate::crate_system::CrateContext::new();
            let mut type_context = crate::type_context::TypeContext::new();
            let error = if alias_mode {
                crate_context.load_product_artifact_from_path_as_with_type_context(
                    "protocol",
                    artifact_path.clone(),
                    &mut type_context,
                )
            } else {
                crate_context.load_product_artifact_from_path_with_type_context(
                    artifact_path.clone(),
                    &mut type_context,
                )
            }
            .expect_err("malformed artifact must fail to load");

            assert_eq!(error, expected);
            assert!(crate_context.product_crate_ids.is_empty());
            assert!(crate_context.extern_crate("protocol").is_none());
            assert_eq!(type_context.len(), 0);
        }

        let _ = std::fs::remove_dir_all(base);
    }

    fn products_with_reference_index_pair(
        index_receiver_is_mutable: bool,
        index_mut_receiver_is_mutable: bool,
    ) -> CompilerProducts {
        let mut products = valid_products();
        let index_trait_id = products
            .interface
            .language_items
            .index
            .as_ref()
            .expect("Index language item")
            .trait_id;
        let index_mut_trait_id = products
            .interface
            .language_items
            .index_mut
            .as_ref()
            .expect("IndexMut language item")
            .trait_id;
        let cell_id = product_id(20);
        products.interface.structs.insert(
            cell_id,
            ProductStructInterface {
                id: product_def_id(cell_id),
                name: "Cell".to_string(),
                generic_params: Vec::new(),
                fields: Vec::new(),
            },
        );
        add_product_index_impl(
            &mut products,
            30,
            index_trait_id,
            vec![ProductAssociatedTypeInterface {
                id: AssocTypeId(0),
                name: "Value".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::I64,
            }],
        );
        add_product_index_impl(
            &mut products,
            31,
            index_mut_trait_id,
            vec![ProductAssociatedTypeInterface {
                id: AssocTypeId(0),
                name: "Value".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::I64,
            }],
        );
        let cell_type = Type::Struct {
            id: product_def_id(cell_id),
            args: Vec::new(),
        };
        products
            .interface
            .impls
            .get_mut(&product_id(30))
            .expect("Index fixture")
            .receiver_pattern = HirImplReceiverPattern::Exact(Type::Reference {
            mutable: index_receiver_is_mutable,
            inner: Box::new(cell_type.clone()),
        });
        products
            .interface
            .impls
            .get_mut(&product_id(31))
            .expect("IndexMut fixture")
            .receiver_pattern = HirImplReceiverPattern::Exact(Type::Reference {
            mutable: index_mut_receiver_is_mutable,
            inner: Box::new(cell_type),
        });
        products
    }

    #[test]
    fn product_language_item_validation_accepts_all_complete_protocols() {
        validate_product_language_items(&valid_products()).expect("complete registry is valid");
    }

    #[test]
    fn product_artifact_rejects_unpaired_index_mut_impl() {
        let mut products = valid_products();
        let index_mut_trait = products
            .interface
            .language_items
            .index_mut
            .as_ref()
            .expect("IndexMut language item")
            .trait_id;
        let index_mut_method = products
            .interface
            .language_items
            .index_mut
            .as_ref()
            .expect("IndexMut language item")
            .method_id;
        let index_mut_impl_method = product_id(22);
        let cell_id = product_id(20);
        products.interface.structs.insert(
            cell_id,
            ProductStructInterface {
                id: product_def_id(cell_id),
                name: "Cell".to_string(),
                generic_params: Vec::new(),
                fields: Vec::new(),
            },
        );
        products.interface.impls.insert(
            product_id(21),
            ProductImplInterface {
                id: product_def_id(product_id(21)),
                owner: HirImplOwner::Named("Cell".to_string()),
                type_name: "Cell".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                    id: product_def_id(cell_id),
                    args: Vec::new(),
                }),
                trait_name: Some("WriteAt".to_string()),
                trait_id: Some(product_def_id(index_mut_trait)),
                trait_generics: Vec::new(),
                trait_arg_types: vec![Type::I64],
                associated_types: vec![ProductAssociatedTypeInterface {
                    id: AssocTypeId(0),
                    name: "WriteValue".to_string(),
                    kind: crate::type_services::kind::Kind::Type,
                    ty: Type::I64,
                }],
                bounds: HashMap::new().into(),
                methods: BTreeMap::from([(
                    "write_at".to_string(),
                    ProductFunctionInterface {
                        id: product_def_id(index_mut_impl_method),
                        name: "write_at".to_string(),
                        generic_params: Vec::new(),
                        generic_bounds: HashMap::new().into(),
                        params: vec![
                            Type::Reference {
                                mutable: true,
                                inner: Box::new(Type::Struct {
                                    id: product_def_id(cell_id),
                                    args: Vec::new(),
                                }),
                            },
                            Type::I64,
                        ],
                        ret_type: Type::Reference {
                            mutable: true,
                            inner: Box::new(Type::I64),
                        },
                        is_curried: false,
                        is_method: true,
                        self_receiver: Some(ReceiverMode::Mut),
                        is_unsafe: false,
                    },
                )]),
            },
        );
        products
            .interface
            .effective_trait_methods
            .insert((product_id(21), index_mut_method), index_mut_impl_method);
        products.link.records.insert(
            index_mut_impl_method,
            ProductLinkRecord {
                backend_symbol: "__rock_fn_index_mut_fixture".to_string(),
            },
        );

        assert_loader_error_in_both_modes(
            &products,
            "Product artifact IndexMut implementation for Cell with key I64 requires a matching Index implementation",
        );
    }

    #[test]
    fn product_artifact_accepts_shared_mutable_reference_index_pair() {
        let products = products_with_reference_index_pair(false, true);
        validate_product_language_items(&products).expect("shared/mutable pair is valid");
    }

    #[test]
    fn product_artifact_rejects_reverse_reference_index_pair() {
        let products = products_with_reference_index_pair(true, false);
        assert_loader_error_in_both_modes(
            &products,
            "Product artifact IndexMut implementation for Cell with key I64 requires a matching Index implementation",
        );
    }

    #[test]
    fn product_artifact_rejects_same_shared_reference_index_pair() {
        let products = products_with_reference_index_pair(false, false);
        assert_loader_error_in_both_modes(
            &products,
            "Product artifact IndexMut implementation for Cell with key I64 requires a matching Index implementation",
        );
    }

    #[test]
    fn product_artifact_rejects_missing_marked_index_mut_output() {
        let mut products = valid_products();
        let (index_trait_id, index_output_id) = {
            let index = products.interface.language_items.index.as_ref().unwrap();
            (index.trait_id, index.output_id)
        };
        let index_mut_trait_id = products
            .interface
            .language_items
            .index_mut
            .as_ref()
            .unwrap()
            .trait_id;
        products.interface.structs.insert(
            product_id(20),
            ProductStructInterface {
                id: product_def_id(product_id(20)),
                name: "Cell".to_string(),
                generic_params: Vec::new(),
                fields: Vec::new(),
            },
        );
        add_product_index_impl(
            &mut products,
            30,
            index_trait_id,
            vec![ProductAssociatedTypeInterface {
                id: index_output_id,
                name: "ReadValue".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::I64,
            }],
        );
        add_product_index_impl(&mut products, 31, index_mut_trait_id, Vec::new());

        assert_loader_error_in_both_modes(
            &products,
            "Product artifact IndexMut implementation for Cell with key I64 is missing marked associated output AssocTypeId(0)",
        );
    }

    #[test]
    fn product_artifact_rejects_duplicate_marked_index_mut_output() {
        let mut products = valid_products();
        let (index_trait_id, index_output_id) = {
            let index = products.interface.language_items.index.as_ref().unwrap();
            (index.trait_id, index.output_id)
        };
        let (index_mut_trait_id, index_mut_output_id) = products
            .interface
            .language_items
            .index_mut
            .as_ref()
            .map(|index_mut| (index_mut.trait_id, index_mut.output_id))
            .unwrap();
        products.interface.structs.insert(
            product_id(20),
            ProductStructInterface {
                id: product_def_id(product_id(20)),
                name: "Cell".to_string(),
                generic_params: Vec::new(),
                fields: Vec::new(),
            },
        );
        add_product_index_impl(
            &mut products,
            32,
            index_trait_id,
            vec![ProductAssociatedTypeInterface {
                id: index_output_id,
                name: "ReadValue".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::I64,
            }],
        );
        add_product_index_impl(
            &mut products,
            33,
            index_mut_trait_id,
            vec![
                ProductAssociatedTypeInterface {
                    id: index_mut_output_id,
                    name: "WriteValue".to_string(),
                    kind: crate::type_services::kind::Kind::Type,
                    ty: Type::I64,
                },
                ProductAssociatedTypeInterface {
                    id: index_mut_output_id,
                    name: "WriteValueAgain".to_string(),
                    kind: crate::type_services::kind::Kind::Type,
                    ty: Type::I64,
                },
            ],
        );

        assert_loader_error_in_both_modes(
            &products,
            "Product artifact IndexMut implementation for Cell with key I64 has duplicate marked associated output AssocTypeId(0)",
        );
    }

    #[test]
    fn product_index_pair_accepts_reordered_bounds() {
        let mut products = valid_products();
        let index = products.interface.language_items.index.as_ref().unwrap();
        let index_mut = products
            .interface
            .language_items
            .index_mut
            .as_ref()
            .unwrap();
        let first = TraitBound {
            trait_id: product_def_id(product_id(1)),
            type_args: Vec::new(),
        };
        let second = TraitBound {
            trait_id: product_def_id(product_id(2)),
            type_args: Vec::new(),
        };
        products.interface.impls.insert(
            product_id(34),
            product_index_impl_with_bounds(34, index.trait_id, vec![first.clone(), second.clone()]),
        );
        products.interface.impls.insert(
            product_id(35),
            product_index_impl_with_bounds(35, index_mut.trait_id, vec![second, first]),
        );

        validate_product_index_mut_impls(&products, &products.interface.language_items)
            .expect("reordered conjunctive bounds should pair");
    }

    #[test]
    fn product_language_item_preflight_ignores_unrelated_interface_rows() {
        let mut products = valid_products();
        let row_id = product_id(99);
        products.interface.functions.insert(
            row_id,
            crate::products::ProductFunctionInterface {
                id: product_def_id(product_id(98)),
                name: "unrelated".to_string(),
                generic_params: Vec::new(),
                generic_bounds: HashMap::new().into(),
                params: Vec::new(),
                ret_type: Type::Unit,
                is_curried: false,
                is_method: false,
                self_receiver: None,
                is_unsafe: false,
            },
        );

        assert_loader_error_in_both_modes(
            &products,
            "Product artifact interface function row 99 has embedded ID 98",
        );
    }

    #[test]
    fn product_language_item_loader_rejects_malformed_registry_matrix() {
        macro_rules! loader_case {
            ($mutate:expr, $expected:expr) => {{
                let mut products = valid_products();
                ($mutate)(&mut products);
                assert_loader_error_in_both_modes(&products, $expected);
            }};
        }

        loader_case!(
            |products: &mut CompilerProducts| {
                products.interface.language_items.index.as_mut().unwrap().trait_id = product_id(99);
            },
            "Product artifact language item index.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(99) }: index.trait must identify a trait; DefId { crate_id: CrateId(0), local: LocalDefId(99) } is no declaration"
        );
        loader_case!(
            |products: &mut CompilerProducts| {
                products.interface.language_items.index.as_mut().unwrap().method_id = product_id(99);
            },
            "Product artifact language item index.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(4) }: index.method DefId { crate_id: CrateId(0), local: LocalDefId(99) } is not declared by any trait"
        );
        loader_case!(
            |products: &mut CompilerProducts| {
                products
                    .interface
                    .language_items
                    .index_mut
                    .as_mut()
                    .unwrap()
                    .method_id = product_id(99);
            },
            "Product artifact language item index_mut.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(11) }: index_mut.method DefId { crate_id: CrateId(0), local: LocalDefId(99) } is not declared by any trait"
        );
        loader_case!(
            |products: &mut CompilerProducts| {
                products
                    .interface
                    .language_items
                    .index_mut
                    .as_mut()
                    .unwrap()
                    .output_id = AssocTypeId(1);
            },
            "Product artifact language item index_mut.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(11) }: index_mut.output AssocTypeId(1) is declared by trait DefId { crate_id: CrateId(0), local: LocalDefId(6) }, not DefId { crate_id: CrateId(0), local: LocalDefId(11) }"
        );
        loader_case!(
            |products: &mut CompilerProducts| {
                let method = products
                    .interface
                    .traits
                    .get_mut(&product_id(11))
                    .unwrap()
                    .signatures
                    .values_mut()
                    .next()
                    .unwrap();
                method.self_receiver = Some(ReceiverMode::Shared);
            },
            "Product artifact language item index_mut.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(11) }: index_mut.method must use a mutable receiver"
        );
        loader_case!(
            |products: &mut CompilerProducts| {
                let method = products
                    .interface
                    .traits
                    .get_mut(&product_id(11))
                    .unwrap()
                    .signatures
                    .values_mut()
                    .next()
                    .unwrap();
                let Type::Reference { mutable, .. } = &mut method.ret else {
                    panic!("valid IndexMut return must be a reference");
                };
                *mutable = false;
            },
            "Product artifact language item index_mut.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(11) }: index_mut.method must return a mutable reference to index_mut.output on Self"
        );
        loader_case!(
            |products: &mut CompilerProducts| {
                products.interface.language_items.index.as_mut().unwrap().output_id = AssocTypeId(9);
            },
            "Product artifact language item index.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(4) }: index.output AssocTypeId(9) is not declared by trait DefId { crate_id: CrateId(0), local: LocalDefId(4) }"
        );
        loader_case!(
            |products: &mut CompilerProducts| {
                products
                    .interface
                    .traits
                    .get_mut(&product_id(4))
                    .unwrap()
                    .signatures
                    .values_mut()
                    .next()
                    .unwrap()
                    .ret = Type::Unit;
            },
            "Product artifact language item index.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(4) }: index.method must return a shared reference to index.output on Self"
        );
        loader_case!(
            |products: &mut CompilerProducts| {
                products.interface.language_items.try_protocol.as_mut().unwrap().try_trait_id =
                    product_id(10);
            },
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(10) }: try.trait must identify a trait; DefId { crate_id: CrateId(0), local: LocalDefId(10) } is an enum"
        );
        loader_case!(
            |products: &mut CompilerProducts| {
                products
                    .interface
                    .language_items
                    .try_protocol
                    .as_mut()
                    .unwrap()
                    .branch_method_id = product_id(99);
            },
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: try.branch DefId { crate_id: CrateId(0), local: LocalDefId(99) } is not declared by any trait"
        );
        loader_case!(
            |products: &mut CompilerProducts| {
                products
                    .interface
                    .traits
                    .get_mut(&product_id(6))
                    .unwrap()
                    .signatures
                    .values_mut()
                    .next()
                    .unwrap()
                    .ret = Type::Unit;
            },
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: try.branch must return control_flow<Residual, Output>"
        );
        loader_case!(
            |products: &mut CompilerProducts| {
                products.interface.language_items.try_protocol.as_mut().unwrap().residual_id =
                    AssocTypeId(9);
            },
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: try.residual AssocTypeId(9) is not declared by trait DefId { crate_id: CrateId(0), local: LocalDefId(6) }"
        );
        loader_case!(
            |products: &mut CompilerProducts| {
                products
                    .interface
                    .language_items
                    .try_protocol
                    .as_mut()
                    .unwrap()
                    .from_residual_method_id = product_id(99);
            },
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: from_residual.method DefId { crate_id: CrateId(0), local: LocalDefId(99) } is not declared by any trait"
        );
        loader_case!(
            |products: &mut CompilerProducts| {
                products
                    .interface
                    .traits
                    .get_mut(&product_id(8))
                    .unwrap()
                    .signatures
                    .values_mut()
                    .next()
                    .unwrap()
                    .ret = Type::Unit;
            },
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: from_residual.method must return Self"
        );
        loader_case!(
            |products: &mut CompilerProducts| {
                products
                    .interface
                    .language_items
                    .try_protocol
                    .as_mut()
                    .unwrap()
                    .break_variant_id = VariantId(9);
            },
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: control_flow.break VariantId(9) is not declared by enum DefId { crate_id: CrateId(0), local: LocalDefId(10) }"
        );
        loader_case!(
            |products: &mut CompilerProducts| {
                products
                    .interface
                    .enums
                    .get_mut(&product_id(10))
                    .unwrap()
                    .variants[0]
                    .fields = HirVariantFields::Unit;
            },
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: control_flow.break must not be unit"
        );
        loader_case!(
            |products: &mut CompilerProducts| {
                products
                    .interface
                    .enums
                    .get_mut(&product_id(10))
                    .unwrap()
                    .generic_params
                    .push(GenericParamDecl::type_param(
                        GenericParamId {
                            owner: product_def_id(product_id(10)),
                            index: 2,
                        },
                        "Extra",
                    ));
            },
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: control_flow.enum must declare exactly 2 generic parameters; found 3"
        );
        loader_case!(
            |products: &mut CompilerProducts| {
                products
                    .interface
                    .enums
                    .get_mut(&product_id(10))
                    .unwrap()
                    .variants
                    .push(crate::products::ProductEnumVariantInterface {
                        id: VariantId(2),
                        name: "Extra".to_string(),
                        fields: HirVariantFields::Unit,
                    });
            },
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: control_flow.enum must declare exactly 2 variants; found 3"
        );
        loader_case!(
            |products: &mut CompilerProducts| {
                products.interface.language_items.sized.as_mut().unwrap().trait_id =
                    crate::products::ProductDefId {
                        crate_id: ProductCrateId(1),
                        local_id: ProductLocalDefId(1),
                    };
            },
            "Product artifact language item sized.trait uses non-local ProductDefId { crate_id: ProductCrateId(1), local_id: ProductLocalDefId(1) }; expected product crate 0"
        );
        loader_case!(
            |products: &mut CompilerProducts| {
                products.interface.language_items.drop.as_mut().unwrap().method_id =
                    crate::products::ProductDefId {
                        crate_id: ProductCrateId(1),
                        local_id: ProductLocalDefId(3),
                    };
            },
            "Product artifact language item drop.method uses non-local ProductDefId { crate_id: ProductCrateId(1), local_id: ProductLocalDefId(3) }; expected product crate 0"
        );
        loader_case!(
            |products: &mut CompilerProducts| {
                products.interface.language_items.index.as_mut().unwrap().method_id =
                    crate::products::ProductDefId {
                        crate_id: ProductCrateId(1),
                        local_id: ProductLocalDefId(5),
                    };
            },
            "Product artifact language item index.method uses non-local ProductDefId { crate_id: ProductCrateId(1), local_id: ProductLocalDefId(5) }; expected product crate 0"
        );
        loader_case!(
            |products: &mut CompilerProducts| {
                products
                    .interface
                    .language_items
                    .try_protocol
                    .as_mut()
                    .unwrap()
                    .branch_method_id = crate::products::ProductDefId {
                    crate_id: ProductCrateId(1),
                    local_id: ProductLocalDefId(7),
                };
            },
            "Product artifact language item try.branch uses non-local ProductDefId { crate_id: ProductCrateId(1), local_id: ProductLocalDefId(7) }; expected product crate 0"
        );
    }

    #[test]
    fn product_language_items_remap_every_def_id_at_nonzero_runtime_crate_id() {
        let mut first = valid_products();
        first.crate_identity = ProductCrateIdentity::local("first".to_string());
        let mut second = valid_products();
        second.crate_identity = ProductCrateIdentity::local("second".to_string());
        let base = std::env::temp_dir().join(format!(
            "rock_language_item_remap_{}_{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("temporary artifact directory");
        let first_path = base.join("first.rkca");
        let second_path = base.join("second.rkca");
        first
            .write_artifact_to_path(&first_path)
            .expect("first product artifact");
        second
            .write_artifact_to_path(&second_path)
            .expect("second product artifact");
        let mut context = crate::crate_system::CrateContext::new();
        context
            .load_product_artifact_from_path(first_path)
            .expect("first product artifact should load");
        context
            .load_product_artifact_from_path(second_path)
            .expect("second product artifact should load");
        let items = context
            .extern_crate("second")
            .expect("second product extern record")
            .metadata()
            .language_items()
            .clone();
        let runtime_crate = items.sized.as_ref().unwrap().trait_id.crate_id;
        assert_ne!(runtime_crate, CrateId(0));
        let drop = items.drop.as_ref().unwrap();
        assert_eq!(drop.trait_id.crate_id, runtime_crate);
        assert_eq!(drop.method_id.crate_id, runtime_crate);
        let index = items.index.as_ref().unwrap();
        assert_eq!(index.trait_id.crate_id, runtime_crate);
        assert_eq!(index.method_id.crate_id, runtime_crate);
        assert_eq!(index.output_id, AssocTypeId(0));
        let index_mut = items.index_mut.as_ref().unwrap();
        assert_eq!(index_mut.trait_id.crate_id, runtime_crate);
        assert_eq!(index_mut.method_id.crate_id, runtime_crate);
        assert_eq!(index_mut.output_id, AssocTypeId(0));
        let try_protocol = items.try_protocol.as_ref().unwrap();
        assert_eq!(try_protocol.try_trait_id.crate_id, runtime_crate);
        assert_eq!(try_protocol.branch_method_id.crate_id, runtime_crate);
        assert_eq!(try_protocol.from_residual_trait_id.crate_id, runtime_crate);
        assert_eq!(try_protocol.from_residual_method_id.crate_id, runtime_crate);
        assert_eq!(try_protocol.control_flow_enum_id.crate_id, runtime_crate);
        assert_eq!(try_protocol.output_id, AssocTypeId(0));
        assert_eq!(try_protocol.residual_id, AssocTypeId(1));
        assert_eq!(try_protocol.break_variant_id, VariantId(0));
        assert_eq!(try_protocol.continue_variant_id, VariantId(1));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn product_language_item_validation_rejects_nonlocal_ids_in_every_multi_id_bundle() {
        let mut products = valid_products();
        products
            .interface
            .language_items
            .drop
            .as_mut()
            .unwrap()
            .method_id = crate::products::ProductDefId {
            crate_id: ProductCrateId(1),
            local_id: ProductLocalDefId(3),
        };
        assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item drop.method uses non-local ProductDefId { crate_id: ProductCrateId(1), local_id: ProductLocalDefId(3) }; expected product crate 0"
        );

        let mut products = valid_products();
        products
            .interface
            .language_items
            .index
            .as_mut()
            .unwrap()
            .method_id = crate::products::ProductDefId {
            crate_id: ProductCrateId(1),
            local_id: ProductLocalDefId(5),
        };
        assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item index.method uses non-local ProductDefId { crate_id: ProductCrateId(1), local_id: ProductLocalDefId(5) }; expected product crate 0"
        );

        let mut products = valid_products();
        products
            .interface
            .language_items
            .index_mut
            .as_mut()
            .unwrap()
            .method_id = crate::products::ProductDefId {
            crate_id: ProductCrateId(1),
            local_id: ProductLocalDefId(12),
        };
        assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item index_mut.method uses non-local ProductDefId { crate_id: ProductCrateId(1), local_id: ProductLocalDefId(12) }; expected product crate 0"
        );

        let mut products = valid_products();
        products
            .interface
            .language_items
            .try_protocol
            .as_mut()
            .unwrap()
            .branch_method_id = crate::products::ProductDefId {
            crate_id: ProductCrateId(1),
            local_id: ProductLocalDefId(7),
        };
        assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item try.branch uses non-local ProductDefId { crate_id: ProductCrateId(1), local_id: ProductLocalDefId(7) }; expected product crate 0"
        );
    }

    #[test]
    fn product_language_item_validation_rejects_missing_and_wrong_kind_roots() {
        let mut products = valid_products();
        products
            .interface
            .language_items
            .sized
            .as_mut()
            .unwrap()
            .trait_id = product_id(99);
        assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item sized.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(99) }: sized.trait must identify a trait; DefId { crate_id: CrateId(0), local: LocalDefId(99) } is no declaration"
        );

        let mut products = valid_products();
        let function_id = product_id(99);
        products.interface.functions.insert(
            function_id,
            crate::products::ProductFunctionInterface {
                id: product_def_id(function_id),
                name: "wrong".to_string(),
                generic_params: Vec::new(),
                generic_bounds: HashMap::new().into(),
                params: Vec::new(),
                ret_type: Type::Unit,
                is_curried: false,
                is_method: false,
                self_receiver: None,
                is_unsafe: false,
            },
        );
        products
            .interface
            .language_items
            .sized
            .as_mut()
            .unwrap()
            .trait_id = function_id;
        assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item sized.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(99) }: sized.trait must identify a trait; DefId { crate_id: CrateId(0), local: LocalDefId(99) } is a function"
        );

        let mut products = valid_products();
        products
            .interface
            .language_items
            .sized
            .as_mut()
            .unwrap()
            .trait_id = product_id(10);
        assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item sized.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(10) }: sized.trait must identify a trait; DefId { crate_id: CrateId(0), local: LocalDefId(10) } is an enum"
        );

        let mut products = valid_products();
        products
            .interface
            .language_items
            .try_protocol
            .as_mut()
            .unwrap()
            .control_flow_enum_id = product_id(6);
        assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: try.branch must return the marked control_flow enum; control_flow.enum must identify an enum; DefId { crate_id: CrateId(0), local: LocalDefId(6) } is a trait"
        );
    }

    #[test]
    fn product_language_item_validation_rejects_wrong_member_assoc_and_variant_owners() {
        let mut products = valid_products();
        let other_trait = product_id(13);
        let other_method = product_id(14);
        products.interface.insert_trait(
            other_trait,
            crate::products::ProductTraitInterface::from(&trait_def(
                other_trait,
                Vec::new(),
                Vec::new(),
                vec![signature(other_method, None, Vec::new(), Type::Unit)],
            )),
        );
        products
            .interface
            .language_items
            .drop
            .as_mut()
            .unwrap()
            .method_id = other_method;
        assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item drop.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(2) }: drop.method DefId { crate_id: CrateId(0), local: LocalDefId(14) } is owned by trait DefId { crate_id: CrateId(0), local: LocalDefId(13) }, not DefId { crate_id: CrateId(0), local: LocalDefId(2) }"
        );

        let mut products = valid_products();
        products
            .interface
            .language_items
            .index
            .as_mut()
            .unwrap()
            .output_id = AssocTypeId(9);
        assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item index.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(4) }: index.output AssocTypeId(9) is not declared by trait DefId { crate_id: CrateId(0), local: LocalDefId(4) }"
        );

        let mut products = valid_products();
        products
            .interface
            .language_items
            .try_protocol
            .as_mut()
            .unwrap()
            .break_variant_id = VariantId(9);
        assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: control_flow.break VariantId(9) is not declared by enum DefId { crate_id: CrateId(0), local: LocalDefId(10) }"
        );
    }

    #[test]
    fn product_language_item_validation_reuses_shared_shape_rules() {
        let mut products = valid_products();
        products
            .interface
            .traits
            .get_mut(&product_id(2))
            .unwrap()
            .signatures
            .values_mut()
            .next()
            .unwrap()
            .ret = Type::I64;
        assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item drop.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(2) }: drop.method must return ()"
        );

        let mut products = valid_products();
        products
            .interface
            .traits
            .get_mut(&product_id(4))
            .unwrap()
            .signatures
            .values_mut()
            .next()
            .unwrap()
            .ret = Type::Unit;
        assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item index.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(4) }: index.method must return a shared reference to index.output on Self"
        );

        let mut products = valid_products();
        products
            .interface
            .traits
            .get_mut(&product_id(6))
            .unwrap()
            .signatures
            .values_mut()
            .next()
            .unwrap()
            .ret = Type::Unit;
        assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: try.branch must return control_flow<Residual, Output>"
        );

        let mut products = valid_products();
        products
            .interface
            .traits
            .get_mut(&product_id(8))
            .unwrap()
            .signatures
            .values_mut()
            .next()
            .unwrap()
            .ret = Type::Unit;
        assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: from_residual.method must return Self"
        );

        let mut products = valid_products();
        products
            .interface
            .enums
            .get_mut(&product_id(10))
            .unwrap()
            .variants[0]
            .fields = HirVariantFields::Unit;
        assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: control_flow.break must not be unit"
        );
    }

    #[test]
    fn product_language_item_validation_rejects_index_and_control_flow_shape_matrix() {
        let mut products = valid_products();
        products
            .interface
            .traits
            .get_mut(&product_id(4))
            .unwrap()
            .generic_params
            .push(GenericParamDecl::type_param(
                GenericParamId {
                    owner: product_def_id(product_id(4)),
                    index: 1,
                },
                "Extra",
            ));
        assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item index.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(4) }: index.trait must declare exactly 1 generic parameters; found 2"
        );

        let mut products = valid_products();
        let method = products
            .interface
            .traits
            .get_mut(&product_id(4))
            .unwrap()
            .signatures
            .values_mut()
            .next()
            .unwrap();
        method.self_receiver = Some(ReceiverMode::Move);
        method.params[1] = Type::Unit;
        assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item index.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(4) }: index.method must use a shared receiver; index.method explicit parameter 0 has the wrong type"
        );

        let mut products = valid_products();
        let method = products
            .interface
            .traits
            .get_mut(&product_id(4))
            .unwrap()
            .signatures
            .values_mut()
            .next()
            .unwrap();
        let Type::Reference { inner, .. } = &mut method.ret else {
            panic!("valid Index return must be a reference");
        };
        let Type::Projection { trait_id, .. } = inner.as_mut() else {
            panic!("valid Index return must be a projection");
        };
        *trait_id = product_def_id(product_id(2));
        assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item index.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(4) }: index.method return projection must use index.trait"
        );

        let mut products = valid_products();
        products
            .interface
            .enums
            .get_mut(&product_id(10))
            .unwrap()
            .generic_params
            .push(GenericParamDecl::type_param(
                GenericParamId {
                    owner: product_def_id(product_id(10)),
                    index: 2,
                },
                "Extra",
            ));
        assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: control_flow.enum must declare exactly 2 generic parameters; found 3"
        );

        let mut products = valid_products();
        products
            .interface
            .enums
            .get_mut(&product_id(10))
            .unwrap()
            .variants
            .push(crate::products::ProductEnumVariantInterface {
                id: VariantId(2),
                name: "Extra".to_string(),
                fields: HirVariantFields::Unit,
            });
        assert_eq!(
            validate_product_language_items(&products).unwrap_err(),
            "Product artifact language item try.trait ProductDefId { crate_id: ProductCrateId(0), local_id: ProductLocalDefId(6) }: control_flow.enum must declare exactly 2 variants; found 3"
        );
    }
}

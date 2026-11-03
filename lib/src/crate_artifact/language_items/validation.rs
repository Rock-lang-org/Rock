use super::index_mut_impls::validate_product_index_mut_impls;
use super::product_def_id;
use super::program_rows::product_language_item_program;
use crate::hir::{HirLanguageItems, HirProgram};
use crate::ids::DefId;
use crate::language_items::{
    DropLanguageItems, FnLanguageItems, FnMutLanguageItems, FnOnceLanguageItems, FoldLanguageItems,
    IndexLanguageItems, IndexMutLanguageItems, SendLanguageItems, SizedLanguageItems,
    SyncLanguageItems, TryLanguageItems,
};
use crate::products::{CompilerProducts, ProductCrateId, ProductDefId, ProductLanguageItems};
use crate::types::Type;

pub(in crate::crate_artifact) fn validate_product_language_items(
    products: &CompilerProducts,
) -> Result<(), String> {
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

pub(super) fn validate_embedded_id(
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

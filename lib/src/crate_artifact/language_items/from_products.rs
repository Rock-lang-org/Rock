use super::validation::validate_product_language_items;
use crate::crate_artifact::load::ProductIdentityRemap;
use crate::hir::HirLanguageItems;
use crate::language_items::{
    DropLanguageItems, FnLanguageItems, FnMutLanguageItems, FnOnceLanguageItems, FoldLanguageItems,
    IndexLanguageItems, IndexMutLanguageItems, RangeLanguageItems, SendLanguageItems,
    SizedLanguageItems, SyncLanguageItems, TryLanguageItems,
};
use crate::products::CompilerProducts;

pub(in crate::crate_artifact) fn language_items_from_products(
    products: &CompilerProducts,
    remap: &ProductIdentityRemap,
) -> Result<HirLanguageItems, String> {
    // The single language-item check per artifact load. The producer already
    // validated these items, but the artifact is read from disk: a malformed
    // one must fail here, not as an internal error during lowering.
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

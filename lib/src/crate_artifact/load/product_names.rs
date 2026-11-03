use super::ProductIdentityRemap;
use crate::ids::DefId;
use crate::products::{CompilerProducts, ProductDefId};

pub(super) fn product_display_name(
    products: &CompilerProducts,
    id: ProductDefId,
) -> Option<String> {
    products.identity_table.display_names.get(&id).cloned()
}

pub(super) fn qualify_product_name(crate_name: &str, name: &str) -> String {
    if name
        .strip_prefix(crate_name)
        .is_some_and(|rest| rest.starts_with("::"))
    {
        name.to_string()
    } else {
        format!("{}::{}", crate_name, name)
    }
}

pub(super) fn product_def_id_to_existing_def_id(
    products: &CompilerProducts,
    remap: &ProductIdentityRemap,
    id: ProductDefId,
) -> Result<Option<DefId>, String> {
    if product_id_has_canonical_binding(products, id) {
        Ok(Some(remap.def_id(id)?))
    } else {
        Ok(None)
    }
}

pub(super) fn product_def_id_to_prelude_export_def_id(
    products: &CompilerProducts,
    remap: &ProductIdentityRemap,
    id: ProductDefId,
) -> Result<Option<DefId>, String> {
    if products.interface.functions.contains_key(&id)
        || products.interface.structs.contains_key(&id)
        || products.interface.enums.contains_key(&id)
        || products.interface.type_aliases.contains_key(&id)
        || products.interface.traits.contains_key(&id)
        || products.interface.externs.contains_key(&id)
    {
        Ok(Some(remap.def_id(id)?))
    } else {
        Ok(None)
    }
}

pub(super) fn product_id_has_canonical_binding(
    products: &CompilerProducts,
    id: ProductDefId,
) -> bool {
    products.interface.functions.contains_key(&id)
        || products.interface.structs.contains_key(&id)
        || products.interface.enums.contains_key(&id)
        || products.interface.type_aliases.contains_key(&id)
        || products.interface.traits.contains_key(&id)
        || products.interface.externs.contains_key(&id)
        || products.interface.traits.values().any(|trait_def| {
            trait_def
                .methods
                .values()
                .any(|method| ProductDefId::from(method.id) == id)
        })
        || products.interface.impls.values().any(|impl_def| {
            impl_def
                .methods
                .values()
                .any(|method| ProductDefId::from(method.id) == id)
        })
}

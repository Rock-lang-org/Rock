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
    bindings: &CanonicalBindings,
    remap: &ProductIdentityRemap,
    id: ProductDefId,
) -> Result<Option<DefId>, String> {
    if bindings.contains(id) {
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

/// Product IDs with a canonical binding: items, plus trait and impl methods.
/// Built once per load; a per-name scan of every method was quadratic.
pub(super) struct CanonicalBindings(std::collections::HashSet<ProductDefId>);

impl CanonicalBindings {
    pub(super) fn new(products: &CompilerProducts) -> Self {
        let interface = &products.interface;
        let mut ids = std::collections::HashSet::new();
        ids.extend(interface.functions.keys().copied());
        ids.extend(interface.structs.keys().copied());
        ids.extend(interface.enums.keys().copied());
        ids.extend(interface.type_aliases.keys().copied());
        ids.extend(interface.traits.keys().copied());
        ids.extend(interface.externs.keys().copied());
        for trait_def in interface.traits.values() {
            ids.extend(trait_def.methods.values().map(|m| ProductDefId::from(m.id)));
        }
        for impl_def in interface.impls.values() {
            ids.extend(impl_def.methods.values().map(|m| ProductDefId::from(m.id)));
        }
        Self(ids)
    }

    pub(super) fn contains(&self, id: ProductDefId) -> bool {
        self.0.contains(&id)
    }
}

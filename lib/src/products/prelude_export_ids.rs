use super::interface_types::CompilerProducts;
use super::ProductDefId;

impl CompilerProducts {
    pub fn record_prelude_export_ids(
        &mut self,
        exports: impl IntoIterator<Item = (String, crate::crate_artifact::ArtifactExport)>,
    ) {
        for (alias, export) in exports {
            if let Some(id) = self.product_def_id_for_export(&export) {
                self.identity_table.prelude_export_names.insert(alias, id);
            } else if let Some(variant) = self.prelude_variant_export(&export) {
                self.identity_table
                    .prelude_variant_export_names
                    .insert(alias, variant);
            }
        }
    }

    /// An export whose id is an enum and whose source continues to one of
    /// its variants (`...::Option::Some`).
    fn prelude_variant_export(
        &self,
        export: &crate::crate_artifact::ArtifactExport,
    ) -> Option<(ProductDefId, String)> {
        let id = ProductDefId::from(export.id);
        let enumeration = self.interface.enums.get(&id)?;
        let display_name = self.identity_table.display_names.get(&id)?;
        let (owner, variant) = export.source.rsplit_once("::")?;
        (self.prelude_export_source_is_compatible(owner, display_name)
            && enumeration
                .variants
                .iter()
                .any(|candidate| candidate.name == variant))
        .then(|| (id, variant.to_string()))
    }

    fn product_def_id_for_export(
        &self,
        export: &crate::crate_artifact::ArtifactExport,
    ) -> Option<ProductDefId> {
        let explicit_id = ProductDefId::from(export.id);
        if let Some(display_name) = self.identity_table.display_names.get(&explicit_id) {
            if self.product_id_is_exportable_prelude_item(explicit_id)
                && self.prelude_export_source_is_compatible(&export.source, display_name)
            {
                return Some(explicit_id);
            }
        }

        self.product_def_id_for_source(&export.source)
    }

    fn prelude_export_source_is_compatible(&self, source: &str, display_name: &str) -> bool {
        source == display_name
            || source == format!("{}::{}", self.crate_identity.name, display_name)
    }

    fn product_def_id_for_source(&self, source: &str) -> Option<ProductDefId> {
        self.identity_table
            .display_names
            .iter()
            .find_map(|(id, name)| {
                let crate_qualified = format!("{}::{}", self.crate_identity.name, name);
                (self.product_id_is_exportable_prelude_item(*id)
                    && (name == source || crate_qualified == source))
                    .then_some(*id)
            })
    }

    fn product_id_is_exportable_prelude_item(&self, id: ProductDefId) -> bool {
        self.interface.functions.contains_key(&id)
            || self.interface.structs.contains_key(&id)
            || self.interface.enums.contains_key(&id)
            || self.interface.traits.contains_key(&id)
            || self.interface.externs.contains_key(&id)
    }
}

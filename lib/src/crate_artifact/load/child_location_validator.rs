use std::collections::{BTreeMap, HashMap};

use super::ProductIdentityRemap;
use crate::crate_system::CrateContext;
use crate::ids::{CrateId, DefId, Idx};
use crate::products::{CompilerProducts, ProductCrateId, ProductDefId, ProductLocalDefId};

pub(super) struct ProductChildLocationValidator {
    pub(super) fields_by_id: HashMap<(ProductDefId, crate::ids::FieldId), String>,
    pub(super) variants_by_id: HashMap<(ProductDefId, crate::ids::VariantId), String>,
}

pub(super) const MAX_ARTIFACT_HIR_REMAP_DEPTH: usize = 4096;

impl ProductChildLocationValidator {
    pub(super) fn from_products(products: &CompilerProducts) -> Self {
        let mut fields_by_id = HashMap::new();
        let mut variants_by_id = HashMap::new();

        for (id, struct_def) in &products.interface.structs {
            for field in &struct_def.fields {
                fields_by_id.insert((*id, field.id), field.name.clone());
            }
        }

        for (id, enum_def) in &products.interface.enums {
            for variant in &enum_def.variants {
                variants_by_id.insert((*id, variant.id), variant.name.clone());
                if let crate::hir::HirVariantFields::Named(fields) = &variant.fields {
                    for field in fields {
                        fields_by_id.insert((*id, field.id), field.name.clone());
                    }
                }
            }
        }

        Self {
            fields_by_id,
            variants_by_id,
        }
    }

    pub(super) fn from_products_with_context(
        products: &CompilerProducts,
        remap: &ProductIdentityRemap,
        ctx: &CrateContext,
    ) -> Self {
        let mut validator = Self::from_products(products);
        let consumer_to_product_crate: BTreeMap<CrateId, ProductCrateId> = remap
            .crate_ids
            .iter()
            .filter_map(|(product_crate, consumer_crate)| {
                (*product_crate != remap.local_crate).then_some((*consumer_crate, *product_crate))
            })
            .collect();

        let product_id_for_def = |id: DefId| {
            consumer_to_product_crate
                .get(&id.crate_id)
                .copied()
                .map(|crate_id| ProductDefId {
                    crate_id,
                    local_id: ProductLocalDefId(id.local.0),
                })
        };

        for dep in ctx.extern_crates() {
            let interface = dep.metadata().interface();

            for struct_def in interface.structs.values() {
                let Some(owner) = product_id_for_def(struct_def.id) else {
                    continue;
                };
                for field in &struct_def.fields {
                    validator
                        .fields_by_id
                        .insert((owner, field.id), field.name.clone());
                }
            }

            for enum_def in interface.enums.values() {
                let Some(owner) = product_id_for_def(enum_def.id) else {
                    continue;
                };
                for variant in &enum_def.variants {
                    validator
                        .variants_by_id
                        .insert((owner, variant.id), variant.name.clone());
                    if let crate::hir::HirVariantFields::Named(fields) = &variant.fields {
                        for field in fields {
                            validator
                                .fields_by_id
                                .insert((owner, field.id), field.name.clone());
                        }
                    }
                }
            }
        }

        validator
    }

    pub(super) fn validate_field(
        &self,
        owner: ProductDefId,
        field_id: crate::ids::FieldId,
        location_name: &str,
        expr_name: &str,
    ) -> Result<(), String> {
        let Some(metadata_name) = self.fields_by_id.get(&(owner, field_id)) else {
            return Err(format!(
                "Product artifact references unknown field ID {} on definition {}",
                field_id.raw(),
                owner.local_id.0
            ));
        };

        if metadata_name != location_name || metadata_name != expr_name {
            return Err(format!(
                "Product artifact field sidecar mismatch for definition {} field {}: expression '{}', sidecar '{}', metadata '{}'",
                owner.local_id.0,
                field_id.raw(),
                expr_name,
                location_name,
                metadata_name
            ));
        }

        Ok(())
    }

    pub(super) fn validate_variant(
        &self,
        owner: ProductDefId,
        variant_id: crate::ids::VariantId,
        location_name: &str,
        expr_name: &str,
    ) -> Result<(), String> {
        let Some(metadata_name) = self.variants_by_id.get(&(owner, variant_id)) else {
            return Err(format!(
                "Product artifact references unknown variant ID {} on definition {}",
                variant_id.raw(),
                owner.local_id.0
            ));
        };

        if metadata_name != location_name || metadata_name != expr_name {
            return Err(format!(
                "Product artifact variant sidecar mismatch for definition {} variant {}: expression '{}', sidecar '{}', metadata '{}'",
                owner.local_id.0,
                variant_id.raw(),
                expr_name,
                location_name,
                metadata_name
            ));
        }

        Ok(())
    }
}

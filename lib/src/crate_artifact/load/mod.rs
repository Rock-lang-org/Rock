mod body_remap;
mod body_validation;
mod child_location_validator;
mod dependency_definitions;
mod extern_crate_record;
mod generic_remap;
mod id_remap;
mod method_authority;
mod nominal_checks;
mod nominal_validator;
mod object_link;
mod prelude_exports;
mod product_names;
mod product_validation;
mod products_to_interface;
mod target_remap;

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::crate_system::CrateContext;
use crate::ids::{CrateId, DefId, LocalDefId};
use crate::products::{
    CompilerProducts, PortableProductArtifact, ProductArtifactHeader, ProductCrateId, ProductDefId,
};
use extern_crate_record::{extern_crate_from_products, intern_loaded_extern_crate_types};

#[derive(Debug, Clone)]
pub(super) struct ProductIdentityRemap {
    local_crate: ProductCrateId,
    crate_ids: BTreeMap<ProductCrateId, CrateId>,
}

impl ProductIdentityRemap {
    pub(super) fn from_products(
        ctx: &mut CrateContext,
        products: &CompilerProducts,
    ) -> Result<Self, String> {
        let local_crate = products.identity_table.local_crate.ok_or_else(|| {
            format!(
                "Product artifact for crate '{}' has no local product crate ID",
                products.crate_identity.name
            )
        })?;

        let mut crate_ids = BTreeMap::new();
        crate_ids.insert(
            local_crate,
            ctx.consumer_crate_id_for_product_identity(&products.crate_identity),
        );

        for (product_crate_id, identity) in &products.identity_table.dependencies {
            if *product_crate_id == local_crate {
                return Err(format!(
                    "Product artifact for crate '{}' maps dependency '{}' to local product crate ID {}",
                    products.crate_identity.name,
                    identity.name,
                    product_crate_id.0
                ));
            }

            if let Some(loaded_identity) = ctx
                .product_crate_ids
                .keys()
                .find(|loaded_identity| loaded_identity.name == identity.name)
            {
                if loaded_identity != identity {
                    return Err(format!(
                        "Product artifact dependency identity mismatch for crate '{}': artifact expects {:?}, loaded {:?}",
                        identity.name, identity, loaded_identity
                    ));
                }
            }

            crate_ids.insert(
                *product_crate_id,
                ctx.consumer_crate_id_for_product_identity(identity),
            );
        }

        Ok(Self {
            local_crate,
            crate_ids,
        })
    }

    pub(super) fn def_id(&self, id: ProductDefId) -> Result<DefId, String> {
        let Some(crate_id) = self.crate_ids.get(&id.crate_id).copied() else {
            return Err(format!(
                "Product artifact references unmapped product DefId {}::{}",
                id.crate_id.0, id.local_id.0
            ));
        };

        Ok(DefId::new(crate_id, LocalDefId(id.local_id.0)))
    }
}

impl CrateContext {
    pub fn load_product_artifact_from_path(
        &mut self,
        artifact_path: PathBuf,
    ) -> Result<(), String> {
        let mut context = crate::type_context::TypeContext::new();
        self.load_product_artifact_from_path_with_type_context(artifact_path, &mut context)
    }

    pub(crate) fn load_product_artifact_from_path_with_type_context(
        &mut self,
        artifact_path: PathBuf,
        context: &mut crate::type_context::TypeContext,
    ) -> Result<(), String> {
        let header = ProductArtifactHeader::read_from_path_bounded(&artifact_path)?;
        let products =
            PortableProductArtifact::read_payload_from_path_bounded(&artifact_path, &header)?
                .into_products()?;
        let mut staged_crate_context = self.clone();
        let remap = ProductIdentityRemap::from_products(&mut staged_crate_context, &products)?;
        let extern_record =
            extern_crate_from_products(&products, &artifact_path, &remap, &staged_crate_context)?;
        let mut staged_context = context.clone();
        intern_loaded_extern_crate_types(&extern_record, &mut staged_context)?;
        staged_crate_context.add_extern_crate(extern_record)?;

        *self = staged_crate_context;
        *context = staged_context;

        Ok(())
    }

    pub(crate) fn load_product_artifact_from_header_as_with_type_context(
        &mut self,
        expected_name: &str,
        artifact_path: PathBuf,
        header: &ProductArtifactHeader,
        context: &mut crate::type_context::TypeContext,
    ) -> Result<(), String> {
        let products =
            PortableProductArtifact::read_payload_from_path_bounded(&artifact_path, header)?
                .into_products()?;
        let crate_name = products.crate_identity.name.clone();
        if crate_name != expected_name {
            return Err(format!(
                "Product artifact {} declares crate '{}', but --extern-artifact used name '{}'",
                artifact_path.display(),
                crate_name,
                expected_name
            ));
        }

        let mut staged_crate_context = self.clone();
        let remap = ProductIdentityRemap::from_products(&mut staged_crate_context, &products)?;
        let extern_record =
            extern_crate_from_products(&products, &artifact_path, &remap, &staged_crate_context)?;
        let mut staged_context = context.clone();
        intern_loaded_extern_crate_types(&extern_record, &mut staged_context)?;
        staged_crate_context.add_extern_crate(extern_record)?;

        *self = staged_crate_context;
        *context = staged_context;

        Ok(())
    }
}

#[cfg(test)]
mod tests;

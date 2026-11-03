use super::type_table;
use std::collections::BTreeSet;
use std::io::Write;

use super::artifact_codec::{encode_product_artifact, portable_product_artifact_from_bytes};
use super::artifact_format::{
    PortableProductArtifact, ProductArtifactHeader, PRODUCT_ARTIFACT_FORMAT_VERSION,
};
use super::artifact_metadata::{ProductDependencyFreshness, ProductFreshnessMetadata};
use super::interface_types::CompilerProducts;

impl CompilerProducts {
    pub fn to_artifact_bytes(&self) -> Result<Vec<u8>, String> {
        let artifact = type_table::products_to_artifact(self, PRODUCT_ARTIFACT_FORMAT_VERSION)?;
        let header = ProductArtifactHeader::from_products(self);
        encode_product_artifact(&header, &artifact)
    }

    pub fn from_artifact_bytes(bytes: &[u8]) -> Result<Self, String> {
        portable_product_artifact_from_bytes(bytes)?.into_products()
    }

    pub fn freshness_metadata(&self) -> ProductFreshnessMetadata {
        ProductFreshnessMetadata {
            compiler_version: env!("CARGO_PKG_VERSION").to_string(),
            artifact_format_version: PRODUCT_ARTIFACT_FORMAT_VERSION,
            target_triple: self.crate_identity.target_triple.clone(),
            config_hash: None,
            features: BTreeSet::new(),
            source_fingerprint: self.source_fingerprint.clone(),
            dependencies: self
                .dependencies
                .iter()
                .map(|dependency| ProductDependencyFreshness {
                    name: dependency.name.clone(),
                    artifact_path: dependency.artifact_path.clone(),
                    crate_identity: self
                        .identity_table
                        .dependencies
                        .values()
                        .find(|identity| identity.name == dependency.name)
                        .cloned(),
                    capabilities: dependency.capabilities.clone(),
                })
                .collect(),
        }
    }

    pub(crate) fn validate_freshness_metadata(
        &self,
        metadata: &ProductFreshnessMetadata,
    ) -> Result<(), String> {
        let expected = self.freshness_metadata();
        if metadata != &expected {
            return Err(format!(
                "Product artifact freshness metadata mismatch: expected {:?}, found {:?}",
                expected, metadata
            ));
        }

        Ok(())
    }

    pub fn write_artifact_to_path(&self, path: &std::path::Path) -> Result<(), String> {
        let bytes = self.to_artifact_bytes()?;
        let mut file = std::fs::File::create(path).map_err(|err| {
            format!(
                "Failed to write product artifact {}: {}",
                path.display(),
                err
            )
        })?;
        file.write_all(&bytes).map_err(|err| {
            format!(
                "Failed to write product artifact {}: {}",
                path.display(),
                err
            )
        })
    }

    pub fn read_artifact_from_path(path: &std::path::Path) -> Result<Self, String> {
        let header = ProductArtifactHeader::read_from_path_bounded(path)?;
        PortableProductArtifact::read_payload_from_path_bounded(path, &header)
            .and_then(PortableProductArtifact::into_products)
            .map_err(|err| {
                format!(
                    "Failed to deserialize product artifact {}: {}",
                    path.display(),
                    err
                )
            })
    }
}

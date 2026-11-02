use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{ProductCrateId, ProductCrateIdentity, ProductDependencyIdentity};
use crate::crate_system::CrateContext;
use crate::Config;

pub(crate) fn product_dependencies_from_config(config: &Config) -> Vec<ProductDependencyIdentity> {
    config
        .extern_artifacts
        .iter()
        .map(|(name, artifact_path)| {
            ProductDependencyIdentity::artifact_object(
                name.clone(),
                canonical_product_dependency_artifact_path(artifact_path),
            )
        })
        .collect()
}

fn canonical_product_dependency_artifact_path(artifact_path: &Path) -> PathBuf {
    artifact_path
        .canonicalize()
        .unwrap_or_else(|_| absolute_product_dependency_artifact_path(artifact_path))
}

fn absolute_product_dependency_artifact_path(artifact_path: &Path) -> PathBuf {
    if artifact_path.is_absolute() {
        return artifact_path.to_path_buf();
    }

    std::env::current_dir()
        .map(|cwd| cwd.join(artifact_path))
        .unwrap_or_else(|_| artifact_path.to_path_buf())
}

pub(crate) fn product_dependency_crate_identities_from_context(
    crate_ctx: &CrateContext,
) -> BTreeMap<ProductCrateId, ProductCrateIdentity> {
    crate_ctx
        .product_crate_ids
        .iter()
        .map(|(identity, crate_id)| (ProductCrateId::from(*crate_id), identity.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::ids::CrateId;
    use crate::products::ProductSourceFingerprint;

    #[test]
    fn product_dependency_crate_identities_from_context_preserves_real_artifact_identities() {
        let mut crate_ctx = CrateContext::new();
        let linux_identity = ProductCrateIdentity {
            name: "dep".to_string(),
            version: "1.0.0".to_string(),
            target_triple: Some("x86_64-unknown-linux-gnu".to_string()),
            format_version: 5,
            source_fingerprint: ProductSourceFingerprint::default(),
        };
        let wasm_identity = ProductCrateIdentity {
            name: "dep".to_string(),
            version: "2.0.0".to_string(),
            target_triple: Some("wasm32-unknown-unknown".to_string()),
            format_version: 6,
            source_fingerprint: ProductSourceFingerprint::default(),
        };
        crate_ctx
            .product_crate_ids
            .insert(linux_identity.clone(), CrateId(7));
        crate_ctx
            .product_crate_ids
            .insert(wasm_identity.clone(), CrateId(42));

        let identities = product_dependency_crate_identities_from_context(&crate_ctx);

        assert_eq!(
            identities,
            BTreeMap::from([
                (ProductCrateId(7), linux_identity),
                (ProductCrateId(42), wasm_identity),
            ])
        );
    }
}

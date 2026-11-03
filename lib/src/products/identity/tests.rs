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

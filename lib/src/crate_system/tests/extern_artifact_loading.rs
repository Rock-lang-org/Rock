use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use crate::crate_system::{load_extern_artifacts, validate_extern_artifact_names, CrateContext};
use crate::products::{
    CompilerProducts, ProductBodies, ProductCrateId, ProductCrateIdentity, ProductIdentityTable,
    ProductLinkData, ProductSourceFingerprint,
};
use crate::Config;

#[test]
fn compile_rejects_duplicate_extern_artifact_names() {
    let config = Config {
        entry_file: PathBuf::from("main.rk"),
        extern_artifacts: vec![
            ("dep".to_string(), "first.rkca".into()),
            ("dep".to_string(), "second.rkca".into()),
        ],
        ..Config::default()
    };

    let error = validate_extern_artifact_names(&config, "main").unwrap_err();
    assert!(error.0.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("Duplicate external artifact crate name 'dep'")
    }));
}

#[test]
fn validate_extern_artifact_names_accepts_nonmatching_entry_stem_alias() {
    let config = Config {
        entry_file: PathBuf::from("app.rk"),
        extern_artifacts: vec![("dep".to_string(), PathBuf::from("missing-dep.rkca"))],
        ..Config::default()
    };

    validate_extern_artifact_names(&config, "app").expect("nonmatching artifact alias is valid");
}

#[test]
fn load_extern_artifacts_rejects_missing_declared_dependency() {
    let base = std::env::temp_dir().join(format!(
        "rock_missing_extern_dependency_{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();

    let artifact_path = base.join("app.rkca");
    let object_path = base.join("app.o");
    fs::write(&object_path, b"").unwrap();

    let missing_identity = ProductCrateIdentity::local("missing".to_string());
    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("app".to_string()),
        identity_table: ProductIdentityTable {
            local_crate: Some(ProductCrateId(0)),
            dependencies: BTreeMap::from([(ProductCrateId(1), missing_identity)]),
            ..ProductIdentityTable::default()
        },
        interface: crate::products::ProductInterface::default(),
        bodies: ProductBodies::default(),
        link: ProductLinkData {
            object_path: Some(object_path),
            ..ProductLinkData::default()
        },
        dependencies: vec![],
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: BTreeMap::new(),
        proc_macros: Vec::new(),
    };
    products.write_artifact_to_path(&artifact_path).unwrap();

    let error = load_extern_artifacts(
        &mut CrateContext::new(),
        &[("app".to_string(), artifact_path)],
        base.clone(),
    )
    .unwrap_err();

    assert!(
        error.message.contains("missing"),
        "unexpected error: {error:?}"
    );

    let _ = fs::remove_dir_all(&base);
}

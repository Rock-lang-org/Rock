use super::*;

#[test]
fn test_run_config_validate_artifact_rejects_corrupt_artifact() {
    let base = std::env::temp_dir().join(format!(
        "rockc_validate_artifact_{}_{}",
        std::process::id(),
        "corrupt"
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let artifact_path = base.join("dep.rkca");
    std::fs::write(&artifact_path, b"not a product artifact").unwrap();

    let config = Config::try_parse_from([
        "rockc",
        "--validate-artifact",
        artifact_path.to_str().unwrap(),
    ])
    .unwrap();

    let diagnostics = run_config(config).expect_err("corrupt artifact should fail");
    assert_eq!(
        diagnostics.0[0].location,
        rock_lib::diagnostic::DiagnosticLocation::Artifact(artifact_path.clone())
    );
    assert_eq!(
        diagnostics.0[0].code,
        Some(rock_lib::diagnostic::DiagnosticCode::Artifact)
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_run_config_validate_artifact_reports_missing_object_as_file() {
    let base = std::env::temp_dir().join(format!(
        "rockc_validate_artifact_{}_{}",
        std::process::id(),
        "missing_object"
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("missing.o");
    let products = rock_lib::products::CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dep".to_string()),
        identity_table: Default::default(),
        interface: rock_lib::products::ProductInterface::default(),
        bodies: Default::default(),
        link: rock_lib::products::ProductLinkData {
            object_path: Some(PathBuf::from(object_path.file_name().unwrap())),
            records: Default::default(),
        },
        dependencies: Vec::new(),
        source_fingerprint: Default::default(),
        proc_macros: Vec::new(),
        infix_precedence: Default::default(),
    };
    products.write_artifact_to_path(&artifact_path).unwrap();

    let config = Config::try_parse_from([
        "rockc",
        "--validate-artifact",
        artifact_path.to_str().unwrap(),
    ])
    .unwrap();
    let diagnostics = run_config(config).expect_err("missing object should fail");

    assert_eq!(
        diagnostics.0[0].location,
        rock_lib::diagnostic::DiagnosticLocation::File(object_path.clone())
    );
    assert_eq!(
        diagnostics.0[0].code,
        Some(rock_lib::diagnostic::DiagnosticCode::File)
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_run_config_validate_artifact_accepts_sibling_root_relative_object() {
    let base = std::env::temp_dir().join(format!(
        "rockc_validate_artifact_{}_{}",
        std::process::id(),
        "sibling_root_object"
    ));
    let _ = std::fs::remove_dir_all(&base);
    let artifact_dir = base.join("build").join("artifacts");
    let object_dir = base.join("build").join("objects");
    std::fs::create_dir_all(&artifact_dir).unwrap();
    std::fs::create_dir_all(&object_dir).unwrap();
    let artifact_path = artifact_dir.join("dep.rkca");
    std::fs::write(object_dir.join("dep.o"), []).unwrap();

    let products = rock_lib::products::CompilerProducts {
        crate_identity: rock_lib::products::ProductCrateIdentity::local("dep".to_string()),
        identity_table: Default::default(),
        interface: rock_lib::products::ProductInterface::default(),
        bodies: Default::default(),
        link: rock_lib::products::ProductLinkData {
            object_path: Some(PathBuf::from("objects/dep.o")),
            records: Default::default(),
        },
        dependencies: Vec::new(),
        source_fingerprint: Default::default(),
        proc_macros: Vec::new(),
        infix_precedence: Default::default(),
    };
    products.write_artifact_to_path(&artifact_path).unwrap();

    let config = Config::try_parse_from([
        "rockc",
        "--validate-artifact",
        artifact_path.to_str().unwrap(),
    ])
    .unwrap();

    run_config(config).unwrap();

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_resolve_artifact_object_path_prefers_artifact_relative_over_cwd() {
    let test_name = format!(
        "rockc_validate_artifact_conflict_{}_{}",
        std::process::id(),
        "artifact_relative"
    );
    let base = std::env::temp_dir().join(&test_name);
    let _ = std::fs::remove_dir_all(&base);
    let artifact_dir = base.join("artifacts");
    std::fs::create_dir_all(&artifact_dir).unwrap();
    let artifact_path = artifact_dir.join("dep.rkca");

    let object_path = PathBuf::from("..")
        .join("target")
        .join(&test_name)
        .join("dep.o");
    let cwd_object_path = std::env::current_dir().unwrap().join(&object_path);
    let artifact_object_path = artifact_dir.join(&object_path);
    std::fs::create_dir_all(cwd_object_path.parent().unwrap()).unwrap();
    std::fs::create_dir_all(artifact_object_path.parent().unwrap()).unwrap();
    std::fs::write(&cwd_object_path, b"wrong object").unwrap();
    std::fs::write(&artifact_object_path, b"right object").unwrap();

    assert_eq!(
        resolve_artifact_object_path(&artifact_path, object_path),
        artifact_object_path
    );

    let _ = std::fs::remove_dir_all(&base);
    let _ = std::fs::remove_dir_all(
        std::env::current_dir()
            .unwrap()
            .join("..")
            .join("target")
            .join(test_name),
    );
}

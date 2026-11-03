use super::*;

#[test]
fn test_print_artifact_deps_reports_transitive_dependency_capabilities() {
    let base = std::env::temp_dir().join(format!(
        "rockc_artifact_deps_{}_{}",
        std::process::id(),
        "transitive_capabilities"
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let dep_a_artifact = base.join("dep_a.rkca");
    let dep_b_artifact = base.join("dep_b.rkca");

    product_artifact("dep_a", Vec::new(), ProductIdentityTable::default())
        .write_artifact_to_path(&dep_a_artifact)
        .unwrap();
    product_artifact(
        "dep_b",
        vec![ProductDependencyIdentity::artifact_object(
            "dep_a".to_string(),
            PathBuf::from("dep_a.rkca"),
        )],
        ProductIdentityTable {
            dependencies: BTreeMap::from([(
                ProductCrateId(1),
                ProductCrateIdentity::local("dep_a".to_string()),
            )]),
            ..ProductIdentityTable::default()
        },
    )
    .write_artifact_to_path(&dep_b_artifact)
    .unwrap();

    let extern_artifact = format!("dep_b={}", dep_b_artifact.display());
    let config = Config::try_parse_from([
        "rockc",
        "--print",
        "artifact-deps",
        "--extern-artifact",
        extern_artifact.as_str(),
    ])
    .unwrap();

    let output = print_request_output(&config).unwrap().unwrap();

    assert_eq!(
        output,
        format!(
            "dep_a={} metadata=true bodies=true link=object\n\
                 dep_b={} metadata=true bodies=true link=object",
            dep_a_artifact.display(),
            dep_b_artifact.display()
        )
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_print_artifact_deps_rejects_missing_dependency_artifact() {
    let base = artifact_deps_test_dir("missing_dependency_artifact");
    let dep_b_artifact = base.join("dep_b.rkca");
    product_artifact(
        "dep_b",
        vec![ProductDependencyIdentity::artifact_object(
            "missing".to_string(),
            PathBuf::from("missing.rkca"),
        )],
        identity_table_with_dependencies([("missing", ProductCrateId(1))]),
    )
    .write_artifact_to_path(&dep_b_artifact)
    .unwrap();

    let config = artifact_deps_config([("dep_b", &dep_b_artifact)]);
    let messages = diagnostic_messages(print_request_output(&config).unwrap_err());

    assert!(messages.contains("dependency artifact 'missing' referenced by 'dep_b'"));
    assert!(messages.contains("missing.rkca"));

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_print_artifact_deps_rejects_corrupt_dependency_artifact() {
    let base = artifact_deps_test_dir("corrupt_dependency_artifact");
    let corrupt_artifact = base.join("corrupt.rkca");
    let dep_b_artifact = base.join("dep_b.rkca");
    std::fs::write(&corrupt_artifact, b"not a product artifact").unwrap();
    product_artifact(
        "dep_b",
        vec![ProductDependencyIdentity::artifact_object(
            "corrupt".to_string(),
            PathBuf::from("corrupt.rkca"),
        )],
        identity_table_with_dependencies([("corrupt", ProductCrateId(1))]),
    )
    .write_artifact_to_path(&dep_b_artifact)
    .unwrap();

    let config = artifact_deps_config([("dep_b", &dep_b_artifact)]);
    let messages = diagnostic_messages(print_request_output(&config).unwrap_err());

    assert!(messages.contains("dependency artifact 'corrupt' referenced by 'dep_b'"));
    assert!(messages.contains("Product artifact is shorter than the 28-byte preamble"));

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_print_artifact_deps_rejects_crate_name_mismatch() {
    let base = artifact_deps_test_dir("crate_name_mismatch");
    let actual_artifact = base.join("actual.rkca");
    product_artifact("actual", Vec::new(), ProductIdentityTable::default())
        .write_artifact_to_path(&actual_artifact)
        .unwrap();

    let config = artifact_deps_config([("expected", &actual_artifact)]);
    let messages = diagnostic_messages(print_request_output(&config).unwrap_err());

    assert!(messages.contains("Artifact dependency 'expected'"));
    assert!(messages.contains("contains crate 'actual'"));

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_print_artifact_deps_rejects_cycles() {
    let base = artifact_deps_test_dir("cycle");
    let dep_a_artifact = base.join("dep_a.rkca");
    let dep_b_artifact = base.join("dep_b.rkca");
    product_artifact(
        "dep_a",
        vec![ProductDependencyIdentity::artifact_object(
            "dep_b".to_string(),
            PathBuf::from("dep_b.rkca"),
        )],
        identity_table_with_dependencies([("dep_b", ProductCrateId(1))]),
    )
    .write_artifact_to_path(&dep_a_artifact)
    .unwrap();
    product_artifact(
        "dep_b",
        vec![ProductDependencyIdentity::artifact_object(
            "dep_a".to_string(),
            PathBuf::from("dep_a.rkca"),
        )],
        identity_table_with_dependencies([("dep_a", ProductCrateId(1))]),
    )
    .write_artifact_to_path(&dep_b_artifact)
    .unwrap();

    let config = artifact_deps_config([("dep_a", &dep_a_artifact)]);
    let messages = diagnostic_messages(print_request_output(&config).unwrap_err());

    assert!(messages.contains("Cyclic artifact dependency involving 'dep_a'"));

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_print_artifact_deps_rejects_duplicate_root_names() {
    let config = Config::try_parse_from([
        "rockc",
        "--print",
        "artifact-deps",
        "--extern-artifact",
        "dep=/tmp/first.rkca",
        "--extern-artifact",
        "dep=/tmp/second.rkca",
    ])
    .unwrap();

    let messages = diagnostic_messages(print_request_output(&config).unwrap_err());

    assert!(messages.contains("Duplicate external artifact crate name 'dep'"));
}

#[test]
fn test_print_artifact_deps_rejects_conflicting_transitive_paths() {
    let base = artifact_deps_test_dir("conflicting_transitive_paths");
    let dep_a_artifact = base.join("dep_a.rkca");
    let dep_b_artifact = base.join("dep_b.rkca");
    let dep_c_artifact = base.join("dep_c.rkca");
    product_artifact("dep_a", Vec::new(), ProductIdentityTable::default())
        .write_artifact_to_path(&dep_a_artifact)
        .unwrap();
    product_artifact(
        "dep_b",
        vec![ProductDependencyIdentity::artifact_object(
            "dep_a".to_string(),
            PathBuf::from("dep_a.rkca"),
        )],
        identity_table_with_dependencies([("dep_a", ProductCrateId(1))]),
    )
    .write_artifact_to_path(&dep_b_artifact)
    .unwrap();
    product_artifact(
        "dep_c",
        vec![ProductDependencyIdentity::artifact_object(
            "dep_a".to_string(),
            PathBuf::from("other_dep_a.rkca"),
        )],
        identity_table_with_dependencies([("dep_a", ProductCrateId(1))]),
    )
    .write_artifact_to_path(&dep_c_artifact)
    .unwrap();

    let config = artifact_deps_config([("dep_b", &dep_b_artifact), ("dep_c", &dep_c_artifact)]);
    let messages = diagnostic_messages(print_request_output(&config).unwrap_err());

    assert!(messages.contains("Conflicting artifact dependency paths for crate 'dep_a'"));

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_print_artifact_deps_rejects_inconsistent_dependency_identity_metadata() {
    let base = artifact_deps_test_dir("inconsistent_dependency_identity_metadata");
    let dep_a_artifact = base.join("dep_a.rkca");
    let dep_b_artifact = base.join("dep_b.rkca");
    let mut mismatched_identity = ProductCrateIdentity::local("dep_a".to_string());
    mismatched_identity.version = "2.0.0".to_string();
    product_artifact("dep_a", Vec::new(), ProductIdentityTable::default())
        .write_artifact_to_path(&dep_a_artifact)
        .unwrap();
    product_artifact(
        "dep_b",
        vec![ProductDependencyIdentity::artifact_object(
            "dep_a".to_string(),
            PathBuf::from("dep_a.rkca"),
        )],
        ProductIdentityTable {
            dependencies: BTreeMap::from([(ProductCrateId(1), mismatched_identity)]),
            ..ProductIdentityTable::default()
        },
    )
    .write_artifact_to_path(&dep_b_artifact)
    .unwrap();

    let config = artifact_deps_config([("dep_b", &dep_b_artifact)]);
    let messages = diagnostic_messages(print_request_output(&config).unwrap_err());

    assert!(messages.contains("identity metadata that does not match"));

    let _ = std::fs::remove_dir_all(&base);
}

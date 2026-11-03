use super::*;

#[test]
fn load_product_artifacts_remap_same_local_crate_ids_to_distinct_consumer_crates() {
    let (base, _cleanup) = temp_test_dir("distinct_consumer_crates");

    let a_artifact = write_product_fixture(&base, "a");
    let b_artifact = write_product_fixture(&base, "b");

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(a_artifact).unwrap();
    ctx.load_product_artifact_from_path(b_artifact).unwrap();

    let a_id = ctx
        .extern_crate("a")
        .unwrap()
        .metadata()
        .resolver()
        .item_paths["a::answer"];
    let b_id = ctx
        .extern_crate("b")
        .unwrap()
        .metadata()
        .resolver()
        .item_paths["b::answer"];

    assert_ne!(a_id.crate_id, b_id.crate_id);
    assert_eq!(a_id.local, LocalDefId(0));
    assert_eq!(b_id.local, LocalDefId(0));
}

#[test]
fn product_crate_id_remap_reuses_shared_transitive_dependency_identity() {
    let shared = ProductCrateIdentity::local("shared".to_string());
    let mut first = product_with_function("first", ProductCrateId(0), 0, PathBuf::from("first.o"));
    first
        .identity_table
        .dependencies
        .insert(ProductCrateId(1), shared.clone());

    let mut second =
        product_with_function("second", ProductCrateId(0), 0, PathBuf::from("second.o"));
    second
        .identity_table
        .dependencies
        .insert(ProductCrateId(1), shared);

    let mut ctx = CrateContext::new();
    let first_remap = super::super::ProductIdentityRemap::from_products(&mut ctx, &first).unwrap();
    let second_remap =
        super::super::ProductIdentityRemap::from_products(&mut ctx, &second).unwrap();

    assert_ne!(
        first_remap.crate_ids[&ProductCrateId(0)],
        second_remap.crate_ids[&ProductCrateId(0)]
    );
    assert_eq!(
        first_remap.crate_ids[&ProductCrateId(1)],
        second_remap.crate_ids[&ProductCrateId(1)]
    );
}

#[test]
fn load_product_artifact_rejects_dependency_identity_mismatch() {
    let (base, _cleanup) = temp_test_dir("dependency_identity_mismatch");
    let dep_artifact = base.join("dep.rkca");
    let dep_object = base.join("dep.o");
    fs::write(&dep_object, []).unwrap();
    let app_artifact = base.join("app.rkca");
    let app_object = base.join("app.o");
    fs::write(&app_object, []).unwrap();

    let dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
    dep_products.write_artifact_to_path(&dep_artifact).unwrap();

    let mut app_products = product_with_function("app", ProductCrateId(0), 0, app_object);
    let mut mismatched_identity = ProductCrateIdentity::local("dep".to_string());
    mismatched_identity.format_version = mismatched_identity.format_version + 1;
    app_products
        .identity_table
        .dependencies
        .insert(ProductCrateId(1), mismatched_identity);
    app_products.write_artifact_to_path(&app_artifact).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(dep_artifact).unwrap();
    let err = ctx
        .load_product_artifact_from_path(app_artifact)
        .unwrap_err();

    assert!(
        err.contains("dependency identity mismatch"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_dependency_fingerprint_mismatch() {
    let (base, _cleanup) = temp_test_dir("dependency_fingerprint_mismatch");
    let dep_artifact = base.join("dep.rkca");
    let dep_object = base.join("dep.o");
    fs::write(&dep_object, []).unwrap();
    let app_artifact = base.join("app.rkca");
    let app_object = base.join("app.o");
    fs::write(&app_object, []).unwrap();

    let mut dep_products = product_with_function("dep", ProductCrateId(0), 0, dep_object);
    dep_products.crate_identity.source_fingerprint.source_hash = Some("dep-v1".to_string());
    dep_products.write_artifact_to_path(&dep_artifact).unwrap();

    let mut app_products = product_with_function("app", ProductCrateId(0), 0, app_object);
    let mut expected_identity = ProductCrateIdentity::local("dep".to_string());
    expected_identity.source_fingerprint.source_hash = Some("dep-v2".to_string());
    app_products
        .identity_table
        .dependencies
        .insert(ProductCrateId(1), expected_identity);
    app_products.write_artifact_to_path(&app_artifact).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(dep_artifact).unwrap();
    let err = ctx
        .load_product_artifact_from_path(app_artifact)
        .unwrap_err();

    assert!(
        err.contains("dependency identity mismatch"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_unmapped_product_crate_id() {
    let (base, _cleanup) = temp_test_dir("unmapped_product_crate");
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let function_id = product_def_id(0, 0);
    products
        .interface
        .functions
        .get_mut(&function_id)
        .unwrap()
        .generic_params
        .push(GenericParamDecl::type_param(
            crate::types::GenericParamId {
                owner: DefId::new(CrateId(9), LocalDefId(0)),
                index: 0,
            },
            "T",
        ));
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .unwrap_err();

    assert!(err.contains("unmapped product DefId 9::0"), "{err}");
}

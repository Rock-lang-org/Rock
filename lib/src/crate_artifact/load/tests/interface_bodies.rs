use super::*;

#[test]
fn load_product_artifact_rejects_body_only_function_without_interface_declaration() {
    let (base, _cleanup) = temp_test_dir("body_only_function_without_interface");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products.interface.functions.clear();
    products.identity_table.export_names.clear();
    products.link.records.clear();
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("body-only function rows should be rejected");

    assert!(
        err.contains("function body") && err.contains("interface declaration"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_body_only_trait_default_without_interface_declaration() {
    let (base, _cleanup) = temp_test_dir("body_only_trait_default_without_interface");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let method_id = product_def_id(0, 7);
    let method_def = product_hir_def_id(method_id);
    let mut products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dep".to_string()),
        identity_table: ProductIdentityTable {
            local_crate: Some(ProductCrateId(0)),
            display_names: BTreeMap::from([(method_id, "dep::Show::show".to_string())]),
            ..ProductIdentityTable::default()
        },
        interface: ProductInterface::default(),
        bodies: ProductBodies::default(),
        link: ProductLinkData {
            object_path: Some(object_path),
            records: BTreeMap::new(),
        },
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: BTreeMap::new(),
        proc_macros: Vec::new(),
    };
    products
        .bodies
        .trait_default_methods
        .insert(method_id, test_function(method_def, "show"));
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("body-only trait default rows should be rejected");

    assert!(
        err.contains("trait default body") && err.contains("interface declaration"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_trait_default_body_embedded_id_mismatch() {
    let (base, _cleanup) = temp_test_dir("trait_default_body_embedded_id_mismatch");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let trait_id = product_def_id(0, 7);
    let method_id = product_def_id(0, 8);
    let mismatched_method_id = product_def_id(0, 9);

    let mut interface_method = test_function(product_hir_def_id(method_id), "show");
    interface_method.is_method = true;
    let trait_def = HirTrait {
        target: None,
        predicates: Vec::new(),
        id: product_hir_def_id(trait_id),
        name: "Show".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::from([("show".to_string(), interface_method)]),
        signatures: HashMap::new(),
    };
    insert_interface_trait(&mut products, trait_id, trait_def);
    products
        .identity_table
        .display_names
        .insert(trait_id, "dep::Show".to_string());

    let mut body = test_function(product_hir_def_id(mismatched_method_id), "show");
    body.is_method = true;
    products
        .bodies
        .trait_default_methods
        .insert(method_id, body);
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("trait default body embedded ID mismatches should be rejected");

    assert!(
        err.contains("trait default body")
            && err.contains("embedded ID")
            && err.contains("interface declaration"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_interface_function_embedded_id_mismatch() {
    let (base, _cleanup) = temp_test_dir("interface_function_embedded_id_mismatch");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let function_id = product_def_id(0, 0);
    let mismatched_id = product_def_id(0, 9);
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products
        .interface
        .functions
        .get_mut(&function_id)
        .unwrap()
        .id = product_hir_def_id(mismatched_id);
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("interface function ID mismatches should be rejected");

    assert!(
        err.contains("interface function") && err.contains("embedded ID"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_interface_function_foreign_row_key() {
    let (base, _cleanup) = temp_test_dir("interface_function_foreign_row_key");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let foreign_function_id = product_def_id(1, 9);
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products.identity_table.dependencies.insert(
        ProductCrateId(1),
        ProductCrateIdentity::local("foreign".to_string()),
    );
    products
        .identity_table
        .display_names
        .insert(foreign_function_id, "foreign::answer".to_string());
    insert_interface_function(
        &mut products,
        foreign_function_id,
        &test_function(product_hir_def_id(foreign_function_id), "foreign::answer"),
    );
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("interface row keys must be owned by the artifact crate");

    assert!(
        err.contains("interface function") && err.contains("non-local"),
        "unexpected error: {err}"
    );
}

#[test]
fn product_interface_shared_default_member_id_requires_matching_signature() {
    let trait_id = product_def_id(0, 1);
    let member_id = product_hir_def_id(product_def_id(0, 2));
    let method = test_function(member_id, "value");
    let signature = crate::hir::HirFunctionSig {
        id: member_id,
        name: "value".to_string(),
        generic_params: method.generic_params.clone(),
        params: method.params.iter().map(|param| param.ty.clone()).collect(),
        ret: method.ret_type.clone(),
        generic_bounds: method.generic_bounds.clone(),
        self_receiver: method.self_receiver,
        is_unsafe: method.is_unsafe,
    };
    let mut products = product_with_function("dep", ProductCrateId(0), 0, PathBuf::from("dep.o"));
    insert_interface_trait(
        &mut products,
        trait_id,
        HirTrait {
            id: product_hir_def_id(trait_id),
            name: "Value".to_string(),
            generic_params: Vec::new(),
            target: None,
            predicates: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("value".to_string(), method)]),
            signatures: HashMap::from([("value".to_string(), signature)]),
        },
    );
    super::super::validate_product_interface_rows(&products).unwrap();
    for change in 0..3 {
        let mut malformed = products.clone();
        let signature = malformed
            .interface
            .traits
            .get_mut(&trait_id)
            .unwrap()
            .signatures
            .get_mut("value")
            .unwrap();
        match change {
            0 => signature.ret = Type::Bool,
            1 => signature.params.push(Type::I64),
            _ => signature.is_unsafe = true,
        }
        assert!(super::super::validate_product_interface_rows(&malformed)
            .unwrap_err()
            .contains("disagrees"));
    }
    let trait_def = products.interface.traits.get_mut(&trait_id).unwrap();
    let mut unrelated = trait_def.signatures["value"].clone();
    unrelated.name = "other".to_string();
    trait_def.signatures.insert("other".to_string(), unrelated);
    assert!(super::super::validate_product_interface_rows(&products)
        .unwrap_err()
        .contains("duplicate callable ID"));
}

#[test]
fn load_product_artifact_rejects_nested_method_reusing_top_level_function_id() {
    let (base, _cleanup) = temp_test_dir("nested_method_reuses_function_id");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let function_id = product_def_id(0, 0);
    let impl_id = product_def_id(0, 7);
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let mut method = test_function(product_hir_def_id(function_id), "value");
    method.is_method = true;
    insert_interface_impl(
        &mut products,
        impl_id,
        HirImpl {
            id: product_hir_def_id(impl_id),
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), method)]),
        },
    );
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("callable declaration IDs must be unique");

    assert!(
        err.contains("duplicate callable ID")
            && err.contains("function")
            && err.contains("method 'value'"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_interface_impl_method_name_mismatch() {
    let (base, _cleanup) = temp_test_dir("interface_impl_method_name_mismatch");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let impl_id = product_def_id(0, 7);
    let method_id = product_def_id(0, 8);
    let mut method = test_function(product_hir_def_id(method_id), "hidden");
    method.is_method = true;
    let interface_impl = test_generic_impl(
        product_hir_def_id(impl_id),
        HashMap::from([("value".to_string(), method)]),
    );
    insert_interface_impl(&mut products, impl_id, interface_impl);
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("interface impl method names must match method row keys");

    assert!(
        err.contains("interface impl")
            && err.contains("method 'value'")
            && err.contains("name 'hidden'"),
        "unexpected error: {err}"
    );
}

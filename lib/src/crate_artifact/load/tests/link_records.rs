use super::*;

#[test]
fn load_product_artifact_inserts_artifact_only_extern_record() {
    let (base, _cleanup) = temp_test_dir("artifact_only_extern_record");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let products = product_with_function("dep", ProductCrateId(0), 0, object_path.clone());
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();

    let dep = ctx
        .extern_crate("dep")
        .expect("extern record should be inserted");
    let answer_id = dep
        .metadata()
        .interface()
        .function_by_canonical_name("dep::answer")
        .expect("function interface should exist")
        .id;
    assert!(dep.body_providers().generic_function(answer_id).is_some());
    assert_eq!(
        dep.link().object_path(),
        Some(&object_path.canonicalize().unwrap())
    );
}

#[test]
fn load_product_artifact_rejects_link_record_for_undeclared_callable_id() {
    let (base, _cleanup) = temp_test_dir("link_record_undeclared_callable");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let undeclared_id = product_def_id(0, 9);
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products.link.records.insert(
        undeclared_id,
        ProductLinkRecord {
            backend_symbol: "dep_hidden".to_string(),
        },
    );
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("link records for undeclared callable IDs should be rejected");

    assert!(
        err.contains("link record")
            && err.contains("backend symbol")
            && err.contains("interface callable declaration"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_empty_link_record_backend_symbol() {
    let (base, _cleanup) = temp_test_dir("empty_link_record_backend_symbol");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let function_id = product_def_id(0, 0);
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products
        .link
        .records
        .get_mut(&function_id)
        .unwrap()
        .backend_symbol = String::new();
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("empty backend symbols should be rejected");

    assert!(
        err.contains("0::0") && err.contains("empty backend symbol"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_object_function_missing_link_record() {
    let (base, _cleanup) = temp_test_dir("object_function_missing_link_record");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products.bodies.functions.clear();
    products.link.records.clear();
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("object-backed concrete functions require link records");

    assert!(
        err.contains("missing link record")
            && err.contains("backend symbol")
            && err.contains("function"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_object_concrete_function_body_without_link_record() {
    let (base, _cleanup) = temp_test_dir("object_concrete_function_body_missing_link_record");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products.link.records.clear();
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("object-backed concrete function bodies still require link records");

    assert!(
        err.contains("missing link record")
            && err.contains("backend symbol")
            && err.contains("function"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_object_impl_method_missing_link_record() {
    let (base, _cleanup) = temp_test_dir("object_impl_method_missing_link_record");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let artifact_path = base.join("dep.rkca");
    let impl_id = DefId::new(CrateId(0), LocalDefId(7));
    let method_id = DefId::new(CrateId(0), LocalDefId(8));
    let product_impl_id = ProductDefId::from(impl_id);
    let product_method_id = ProductDefId::from(method_id);
    let mut method = test_function(method_id, "make");
    method.is_method = true;
    let imp = HirImpl {
        id: impl_id,
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
        methods: HashMap::from([("make".to_string(), method)]),
    };

    let mut identity_table = ProductIdentityTable::default();
    identity_table.local_crate = Some(ProductCrateId(0));
    identity_table
        .display_names
        .insert(product_impl_id, "Box".to_string());
    identity_table
        .display_names
        .insert(product_method_id, "Box::make".to_string());
    let mut interface = ProductInterface::default();
    interface
        .impls
        .insert(product_impl_id, ProductImplInterface::from(&imp));
    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dep".to_string()),
        identity_table,
        interface,
        bodies: ProductBodies::default(),
        link: ProductLinkData {
            object_path: Some(object_path),
            records: BTreeMap::new(),
        },
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: Default::default(),
        proc_macros: Vec::new(),
    };
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("object-backed concrete impl methods require link records");

    assert!(
        err.contains("missing link record")
            && err.contains("backend symbol")
            && err.contains("impl")
            && err.contains("method 'make'"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_rejects_object_concrete_impl_method_when_sibling_is_generic() {
    let (base, _cleanup) = temp_test_dir("object_impl_concrete_method_generic_sibling");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let artifact_path = base.join("dep.rkca");
    let impl_id = DefId::new(CrateId(0), LocalDefId(7));
    let method_id = DefId::new(CrateId(0), LocalDefId(8));
    let generic_method_id = DefId::new(CrateId(0), LocalDefId(9));
    let product_impl_id = ProductDefId::from(impl_id);
    let product_method_id = ProductDefId::from(method_id);
    let product_generic_method_id = ProductDefId::from(generic_method_id);

    let mut method = test_function(method_id, "make");
    method.is_method = true;

    let generic_param = crate::types::GenericParamId {
        owner: generic_method_id,
        index: 0,
    };
    let mut generic_method = test_function(generic_method_id, "id");
    generic_method.is_method = true;
    generic_method
        .generic_params
        .push(GenericParamDecl::type_param(generic_param, "T"));

    let imp = HirImpl {
        id: impl_id,
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
        methods: HashMap::from([
            ("make".to_string(), method),
            ("id".to_string(), generic_method),
        ]),
    };

    let mut identity_table = ProductIdentityTable::default();
    identity_table.local_crate = Some(ProductCrateId(0));
    identity_table
        .display_names
        .insert(product_impl_id, "Box".to_string());
    identity_table
        .display_names
        .insert(product_method_id, "Box::make".to_string());
    identity_table
        .display_names
        .insert(product_generic_method_id, "Box::id".to_string());
    let mut interface = ProductInterface::default();
    interface
        .impls
        .insert(product_impl_id, ProductImplInterface::from(&imp));
    let mut bodies = ProductBodies::default();
    bodies.generic_impls.insert(product_impl_id, imp);
    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dep".to_string()),
        identity_table,
        interface,
        bodies,
        link: ProductLinkData {
            object_path: Some(object_path),
            records: BTreeMap::new(),
        },
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: Default::default(),
        proc_macros: Vec::new(),
    };
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("object-backed concrete impl methods require link records");

    assert!(
        err.contains("missing link record")
            && err.contains("backend symbol")
            && err.contains("impl")
            && err.contains("method 'make'"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_allows_generic_function_without_link_record() {
    let (base, _cleanup) = temp_test_dir("generic_function_without_link_record");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let artifact_path = base.join("dep.rkca");
    let product_id = product_def_id(0, 0);
    let function_id = product_hir_def_id(product_id);
    let mut function = test_function(function_id, "dep::id");
    function.generic_params.push(GenericParamDecl::type_param(
        crate::types::GenericParamId {
            owner: function_id,
            index: 0,
        },
        "T",
    ));

    let mut identity_table = ProductIdentityTable::default();
    identity_table.local_crate = Some(ProductCrateId(0));
    identity_table
        .display_names
        .insert(product_id, "dep::id".to_string());
    let mut interface = ProductInterface::default();
    interface
        .functions
        .insert(product_id, ProductFunctionInterface::from(&function));
    let mut bodies = ProductBodies::default();
    bodies.functions.insert(product_id, function);
    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dep".to_string()),
        identity_table,
        interface,
        bodies,
        link: ProductLinkData {
            object_path: Some(object_path),
            records: BTreeMap::new(),
        },
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: Default::default(),
        proc_macros: Vec::new(),
    };
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();
}

#[test]
fn load_product_artifact_rejects_body_only_function_link_record_without_interface_declaration() {
    let (base, _cleanup) = temp_test_dir("body_only_function_link_record");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let body_only_id = product_def_id(0, 9);
    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products.bodies.functions.insert(
        body_only_id,
        test_function(product_hir_def_id(body_only_id), "hidden"),
    );
    products.link.records.insert(
        body_only_id,
        ProductLinkRecord {
            backend_symbol: "dep_hidden".to_string(),
        },
    );
    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .expect_err("body-only function link records should be rejected as backend symbols");

    assert!(
        err.contains("link record")
            && err.contains("backend symbol")
            && err.contains("interface callable declaration"),
        "unexpected error: {err}"
    );
}

#[test]
fn load_product_artifact_registers_object_backed_crate() {
    let base = std::env::temp_dir().join(format!(
        "rock_product_loader_{}_{}",
        std::process::id(),
        "object_crate"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let function_id = DefId::new(CrateId(1), LocalDefId(7));
    let product_id = ProductDefId::from(function_id);
    let mut identity_table = ProductIdentityTable::default();
    identity_table.local_crate = Some(ProductCrateId(1));
    identity_table
        .display_names
        .insert(product_id, "dep::answer".to_string());
    identity_table
        .export_names
        .insert("answer".to_string(), product_id);
    let mut interface = ProductInterface::default();
    interface.functions.insert(
        product_id,
        ProductFunctionInterface::from(&test_function(function_id, "dep::answer")),
    );
    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dep".to_string()),
        identity_table,
        interface,
        bodies: Default::default(),
        link: ProductLinkData {
            object_path: Some(object_path.clone()),
            records: BTreeMap::from([(
                product_id,
                ProductLinkRecord {
                    backend_symbol: "dep_answer".to_string(),
                },
            )]),
        },
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: Default::default(),
        proc_macros: Vec::new(),
    };
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();

    let dep = ctx.extern_crate("dep").unwrap();
    assert!(dep.link().is_object_backed());
    assert_eq!(dep.link().object_path().cloned(), Some(object_path));
    assert_eq!(dep.link().backend_symbol(function_id), Some("dep_answer"));
    let interface = dep.metadata().interface();
    assert!(interface
        .function_by_canonical_name("dep::answer")
        .is_some());
    assert_eq!(
        interface
            .root_export_ids
            .get("answer")
            .map(|export| export.source.as_str()),
        Some("dep::answer")
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn load_product_artifact_resolves_common_relative_object_path() {
    let base = std::env::temp_dir().join(format!(
        "rock_product_loader_{}_{}",
        std::process::id(),
        "relative_object"
    ));
    let _ = fs::remove_dir_all(&base);
    let build_dir = base.join("build");
    fs::create_dir_all(&build_dir).unwrap();
    let artifact_path = build_dir.join("dep.rkca");
    let object_path = build_dir.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dep".to_string()),
        identity_table: ProductIdentityTable {
            local_crate: Some(ProductCrateId(0)),
            ..Default::default()
        },
        interface: ProductInterface::default(),
        bodies: Default::default(),
        link: ProductLinkData {
            object_path: Some(PathBuf::from("build/dep.o")),
            records: Default::default(),
        },
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: Default::default(),
        proc_macros: Vec::new(),
    };
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();

    let dep = ctx.extern_crate("dep").unwrap();
    assert_eq!(
        dep.link().object_path().cloned(),
        Some(object_path.canonicalize().unwrap())
    );

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn load_product_artifact_prefers_artifact_relative_object_over_cwd() {
    let test_name = format!(
        "rock_product_loader_conflict_{}_{}",
        std::process::id(),
        "artifact_relative"
    );
    let base = std::env::temp_dir().join(&test_name);
    let _ = fs::remove_dir_all(&base);
    let artifact_dir = base.join("artifacts");
    fs::create_dir_all(&artifact_dir).unwrap();
    let artifact_path = artifact_dir.join("dep.rkca");

    let object_path = PathBuf::from("..")
        .join("target")
        .join(&test_name)
        .join("dep.o");
    let cwd_object_path = std::env::current_dir().unwrap().join(&object_path);
    let artifact_object_path = artifact_dir.join(&object_path);
    fs::create_dir_all(cwd_object_path.parent().unwrap()).unwrap();
    fs::create_dir_all(artifact_object_path.parent().unwrap()).unwrap();
    fs::write(&cwd_object_path, b"wrong object").unwrap();
    fs::write(&artifact_object_path, b"right object").unwrap();

    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dep".to_string()),
        identity_table: ProductIdentityTable {
            local_crate: Some(ProductCrateId(0)),
            ..Default::default()
        },
        interface: ProductInterface::default(),
        bodies: Default::default(),
        link: ProductLinkData {
            object_path: Some(object_path),
            records: Default::default(),
        },
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: Default::default(),
        proc_macros: Vec::new(),
    };
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();

    let dep = ctx.extern_crate("dep").unwrap();
    assert_eq!(
        dep.link().object_path().cloned(),
        Some(artifact_object_path.canonicalize().unwrap())
    );

    let _ = fs::remove_dir_all(&base);
    let _ = fs::remove_dir_all(
        std::env::current_dir()
            .unwrap()
            .join("..")
            .join("target")
            .join(test_name),
    );
}

use super::*;

#[test]
fn load_product_artifact_records_root_export_ids() {
    let base = std::env::temp_dir().join(format!(
        "rock_product_loader_{}_{}",
        std::process::id(),
        "root_export_ids"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();

    let dep = ctx.extern_crate("dep").unwrap();
    let interface = dep.metadata().interface();
    let exported = interface.root_export_ids.get("answer").unwrap();
    let function = interface.function_by_canonical_name("dep::answer").unwrap();

    assert_eq!(exported.source, "dep::answer");
    assert_eq!(exported.id, function.id);
    assert_eq!(dep.link().backend_symbol(function.id), Some("dep_answer"));

    let _ = fs::remove_dir_all(base);
}

#[test]
fn load_product_artifact_qualifies_method_like_root_export_sources() {
    let base = std::env::temp_dir().join(format!(
        "rock_product_loader_{}_{}",
        std::process::id(),
        "method_like_root_export"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let artifact_path = base.join("dep.rkca");

    let trait_id = DefId::new(CrateId(1), LocalDefId(8));
    let method_id = DefId::new(CrateId(1), LocalDefId(9));
    let product_trait_id = ProductDefId::from(trait_id);
    let product_method_id = ProductDefId::from(method_id);
    let mut identity_table = ProductIdentityTable::default();
    identity_table.local_crate = Some(ProductCrateId(1));
    identity_table
        .display_names
        .insert(product_trait_id, "Show".to_string());
    identity_table
        .display_names
        .insert(product_method_id, "Show::println".to_string());
    identity_table
        .export_names
        .insert("Show::println".to_string(), product_method_id);

    let method = test_function(method_id, "println");
    let mut bodies = ProductBodies::default();
    bodies
        .trait_default_methods
        .insert(product_method_id, method.clone());
    let mut interface = ProductInterface::default();
    interface.traits.insert(
        product_trait_id,
        ProductTraitInterface::from(&HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Show".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("println".to_string(), method)]),
            signatures: HashMap::new(),
        }),
    );

    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dep".to_string()),
        identity_table,
        interface,
        bodies,
        link: ProductLinkData {
            object_path: None,
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
    let interface = dep.metadata().interface();
    let exported = interface.root_export_ids.get("Show::println").unwrap();

    assert_eq!(exported.source, "dep::Show::println");
    assert_eq!(exported.id.local, LocalDefId(9));
    assert_eq!(
        dep.metadata()
            .resolver()
            .item_paths
            .get("dep::Show::println"),
        Some(&exported.id)
    );
    assert_eq!(
        dep.metadata().resolver().item_names_by_id.get(&exported.id),
        Some(&"dep::Show::println".to_string())
    );

    let _ = fs::remove_dir_all(base);
}

#[test]
fn load_product_artifact_rejects_root_export_without_canonical_id() {
    let base = std::env::temp_dir().join(format!(
        "rock_product_loader_{}_{}",
        std::process::id(),
        "bad_root_export_id"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let missing_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(99),
    };
    products
        .identity_table
        .display_names
        .insert(missing_id, "dep::missing".to_string());
    products
        .identity_table
        .export_names
        .insert("missing".to_string(), missing_id);
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .unwrap_err();

    assert!(
        err.contains("root export 'missing'") && err.contains("canonical DefId"),
        "unexpected error: {}",
        err
    );

    let _ = fs::remove_dir_all(base);
}

#[test]
fn load_product_artifact_rejects_root_export_without_display_name() {
    let base = std::env::temp_dir().join(format!(
        "rock_product_loader_{}_{}",
        std::process::id(),
        "bad_root_export_display_name"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let artifact_path = base.join("dep.rkca");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();

    let mut products = product_with_function("dep", ProductCrateId(0), 0, object_path);
    let missing_name_id = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(99),
    };
    products
        .identity_table
        .export_names
        .insert("missing".to_string(), missing_name_id);
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    let err = ctx
        .load_product_artifact_from_path(artifact_path)
        .unwrap_err();

    assert!(
        err.contains("root export 'missing'") && err.contains("display name"),
        "unexpected error: {}",
        err
    );

    let _ = fs::remove_dir_all(base);
}

#[test]
fn load_product_artifact_records_id_backed_prelude_exports() {
    let base = std::env::temp_dir().join(format!(
        "rock_product_loader_{}_{}",
        std::process::id(),
        "id_backed_prelude_exports"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let artifact_path = base.join("std.rkca");
    let object_path = base.join("std.o");
    fs::write(&object_path, []).unwrap();

    let struct_id = DefId::new(CrateId(1), LocalDefId(7));
    let product_id = ProductDefId::from(struct_id);
    let mut identity_table = ProductIdentityTable::default();
    identity_table.local_crate = Some(ProductCrateId(1));
    identity_table
        .display_names
        .insert(product_id, "std::prelude::String".to_string());
    identity_table
        .prelude_export_names
        .insert("Text".to_string(), product_id);

    let mut interface = ProductInterface::default();
    interface.structs.insert(
        product_id,
        ProductStructInterface::from(&HirStruct {
            id: struct_id,
            name: "String".to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        }),
    );

    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("std".to_string()),
        identity_table,
        interface,
        bodies: Default::default(),
        link: ProductLinkData {
            object_path: Some(object_path.clone()),
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

    let std = ctx.extern_crate("std").unwrap();
    let export = std.metadata().prelude_export_ids().get("Text").unwrap();
    assert_eq!(export.source, "std::prelude::String");
    assert_eq!(export.id.crate_id, CrateId(1));

    let _ = fs::remove_dir_all(base);
}

#[test]
fn load_product_artifact_rejects_prelude_export_id_without_display_name() {
    let base = std::env::temp_dir().join(format!(
        "rock_product_loader_{}_{}",
        std::process::id(),
        "bad_id_backed_prelude_export"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let artifact_path = base.join("std.rkca");
    let object_path = base.join("std.o");
    fs::write(&object_path, []).unwrap();

    let product_id = ProductDefId {
        crate_id: ProductCrateId(1),
        local_id: ProductLocalDefId(7),
    };
    let mut identity_table = ProductIdentityTable::default();
    identity_table.local_crate = Some(ProductCrateId(1));
    identity_table
        .prelude_export_names
        .insert("String".to_string(), product_id);

    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("std".to_string()),
        identity_table,
        interface: ProductInterface::default(),
        bodies: Default::default(),
        link: ProductLinkData {
            object_path: Some(object_path.clone()),
            records: Default::default(),
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
        .unwrap_err();

    assert!(
        err.contains("prelude export 'String'") && err.contains("display name"),
        "unexpected error: {}",
        err
    );

    let _ = fs::remove_dir_all(base);
}

#[test]
fn load_product_artifact_rejects_sidecar_id_backed_prelude_export() {
    let base = std::env::temp_dir().join(format!(
        "rock_product_loader_{}_{}",
        std::process::id(),
        "sidecar_id_backed_prelude_export"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let artifact_path = base.join("demo.rkca");
    let object_path = base.join("demo.o");
    fs::write(&object_path, []).unwrap();

    let trait_id = DefId::new(CrateId(1), LocalDefId(7));
    let trait_product_id = ProductDefId::from(trait_id);
    let method_id = DefId::new(CrateId(1), LocalDefId(8));
    let method_product_id = ProductDefId::from(method_id);
    let mut identity_table = ProductIdentityTable::default();
    identity_table.local_crate = Some(ProductCrateId(1));
    identity_table
        .display_names
        .insert(trait_product_id, "Trait".to_string());
    identity_table
        .display_names
        .insert(method_product_id, "default_method".to_string());
    identity_table
        .prelude_export_names
        .insert("default_method".to_string(), method_product_id);

    let method = test_function(method_id, "default_method");
    let mut bodies = ProductBodies::default();
    bodies
        .trait_default_methods
        .insert(method_product_id, method.clone());
    let mut interface = ProductInterface::default();
    interface.traits.insert(
        trait_product_id,
        ProductTraitInterface::from(&HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Trait".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("default_method".to_string(), method)]),
            signatures: HashMap::new(),
        }),
    );

    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("demo".to_string()),
        identity_table,
        interface,
        bodies,
        link: ProductLinkData {
            object_path: Some(object_path.clone()),
            records: Default::default(),
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
        .unwrap_err();

    assert!(
        err.contains("prelude export 'default_method'")
            && err.contains("canonical prelude item ID"),
        "unexpected error: {}",
        err
    );

    let _ = fs::remove_dir_all(base);
}

#[test]
fn load_product_artifact_derives_missing_prelude_exports_from_prelude_items() {
    let base = std::env::temp_dir().join(format!(
        "rock_product_loader_{}_{}",
        std::process::id(),
        "derived_prelude_exports"
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let artifact_path = base.join("std.rkca");
    let object_path = base.join("std.o");
    fs::write(&object_path, []).unwrap();

    let struct_id = DefId::new(CrateId(1), LocalDefId(7));
    let product_id = ProductDefId::from(struct_id);
    let mut identity_table = ProductIdentityTable::default();
    identity_table.local_crate = Some(ProductCrateId(1));
    identity_table
        .display_names
        .insert(product_id, "std::prelude::String".to_string());

    let mut interface = ProductInterface::default();
    interface.structs.insert(
        product_id,
        ProductStructInterface::from(&HirStruct {
            id: struct_id,
            name: "String".to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        }),
    );

    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("std".to_string()),
        identity_table,
        interface,
        bodies: Default::default(),
        link: ProductLinkData {
            object_path: Some(object_path.clone()),
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

    let std = ctx.extern_crate("std").unwrap();
    let interface = std.metadata().interface();
    let exported_struct = interface
        .struct_by_canonical_name("std::prelude::String")
        .unwrap();
    let prelude_export = std.metadata().prelude_export_ids().get("String").unwrap();
    assert_eq!(prelude_export.source, "std::prelude::String");
    assert_eq!(prelude_export.id, exported_struct.id);

    let _ = fs::remove_dir_all(base);
}

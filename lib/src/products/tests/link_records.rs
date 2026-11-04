use super::*;

#[test]
fn compiler_products_preserve_backend_symbols_in_link_records() {
    let hir = resolved_hir_for_products();
    let plain_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(0)));
    let mut records = BTreeMap::new();
    records.insert(
        plain_id,
        ProductLinkRecord {
            backend_symbol: "rock_main".to_string(),
        },
    );

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData {
            object_path: None,
            records,
        },
    )
    .expect("test HIR has valid product language items");

    assert_eq!(
        products
            .link
            .records
            .get(&plain_id)
            .map(|record| &record.backend_symbol),
        Some(&"rock_main".to_string())
    );
}

#[test]
#[should_panic(expected = "trait default method should have a preferred body")]
fn compiler_products_do_not_fallback_trait_defaults_by_display_name() {
    let missing_id = DefId::new(CrateId(0), LocalDefId(7));
    let preferred = super::PreferredTraitDefaultMethods {
        by_id: HashMap::new(),
    };

    let _ = preferred.get(missing_id, "Debug", "show");
}

#[test]
fn compiler_products_record_impl_display_names_without_export_names() {
    let hir = resolved_hir_for_products();

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    assert!(products
        .identity_table
        .display_names
        .values()
        .any(|name| name == "Box as Show"));
    assert!(!products
        .identity_table
        .export_names
        .contains_key("Box as Show"));
}

#[test]
fn compiler_products_do_not_export_colliding_impl_method_display_names() {
    let method_a_id = DefId::new(CrateId(0), LocalDefId(10));
    let method_b_id = DefId::new(CrateId(0), LocalDefId(11));
    let impl_a_id = DefId::new(CrateId(0), LocalDefId(12));
    let impl_b_id = DefId::new(CrateId(0), LocalDefId(13));
    let owner_id = DefId::new(CrateId(0), LocalDefId(14));
    let impls = HashMap::from([
        (
            impl_a_id,
            HirImpl {
                id: impl_a_id,
                owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                    id: owner_id,
                    args: Vec::new(),
                }),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([(
                    "show".to_string(),
                    test_function(method_a_id, "show", Vec::new()),
                )]),
            },
        ),
        (
            impl_b_id,
            HirImpl {
                id: impl_b_id,
                owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                    id: owner_id,
                    args: Vec::new(),
                }),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([(
                    "show".to_string(),
                    test_function(method_b_id, "show", Vec::new()),
                )]),
            },
        ),
    ]);
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::from([(
            owner_id,
            HirStruct {
                id: owner_id,
                name: "Box".to_string(),
                generic_params: Vec::new(),
                fields: Vec::new(),
            },
        )]),
        HashMap::new(),
        HashMap::new(),
        impls,
        HashMap::new(),
        HirNameTables {
            structs_by_name: HashMap::from([("Box".to_string(), owner_id)]),
            ..HirNameTables::default()
        },
        HirLanguageItems::default(),
        &HashMap::from([(owner_id, "Box".to_string())]),
    );
    let hir = ResolvedHirProgram::new(
        program,
        ResolverTables::default(),
        BTreeSet::from([impl_a_id, impl_b_id, method_a_id, method_b_id]),
        CrateId(0),
        local_def_ids_after(22),
    );

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    let colliding_methods = products
        .identity_table
        .display_names
        .values()
        .filter(|name| *name == "Box::show")
        .count();
    assert_eq!(colliding_methods, 2);
    assert!(!products
        .identity_table
        .export_names
        .contains_key("Box::show"));
}

#[test]
fn compiler_products_drop_ambiguous_export_name_collisions() {
    let mut identity_table = ProductIdentityTable::default();
    let first = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(1),
    };
    let second = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(2),
    };

    super::record_export_name(&mut identity_table, first, "same");
    super::record_export_name(&mut identity_table, second, "same");

    assert!(!identity_table.export_names.contains_key("same"));
}

#[test]
fn compiler_products_keep_export_name_ambiguous_after_third_collision() {
    let mut identity_table = ProductIdentityTable::default();
    let first = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(1),
    };
    let second = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(2),
    };
    let third = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(3),
    };

    super::record_export_name(&mut identity_table, first, "same");
    super::record_export_name(&mut identity_table, second, "same");
    super::record_export_name(&mut identity_table, third, "same");

    assert!(!identity_table.export_names.contains_key("same"));
}

#[test]
fn compiler_products_drop_link_records_for_ambiguous_link_ids() {
    let shared_id = DefId::new(CrateId(0), LocalDefId(0));
    let shared_product_id = ProductDefId::from(shared_id);
    let symbol = "ambiguous_impl_symbol".to_string();
    let mut records = BTreeMap::new();
    records.insert(
        shared_product_id,
        ProductLinkRecord {
            backend_symbol: symbol.clone(),
        },
    );
    let mut functions = HashMap::new();
    functions.insert(shared_id, test_function(shared_id, "plain", Vec::new()));
    let owner_id = DefId::new(CrateId(0), LocalDefId(1));
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        functions,
        HashMap::from([(
            owner_id,
            HirStruct {
                id: owner_id,
                name: "Box".to_string(),
                generic_params: generic_params(owner_id, &["T"]),
                fields: Vec::new(),
            },
        )]),
        HashMap::new(),
        HashMap::new(),
        HashMap::from([(
            shared_id,
            HirImpl {
                id: shared_id,
                owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: generic_params(shared_id, &["T"]),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                    id: owner_id,
                    args: vec![Type::Generic(crate::types::GenericParamId {
                        owner: shared_id,
                        index: 0,
                    })],
                }),
                trait_name: Some("Show".to_string()),
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            },
        )]),
        HashMap::new(),
        HirNameTables {
            functions_by_name: HashMap::from([("plain".to_string(), shared_id)]),
            structs_by_name: HashMap::from([("Box".to_string(), owner_id)]),
            ..HirNameTables::default()
        },
        HirLanguageItems::default(),
        &HashMap::from([
            (shared_id, "plain".to_string()),
            (owner_id, "Box".to_string()),
        ]),
    );
    let hir = ResolvedHirProgram::new(
        program,
        ResolverTables::default(),
        BTreeSet::from([shared_id]),
        CrateId(0),
        IdGen::new(),
    );

    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData {
            object_path: None,
            records,
        },
    )
    .expect("test HIR has valid product language items");

    assert!(!products
        .link
        .records
        .values()
        .any(|record| record.backend_symbol == symbol));
}

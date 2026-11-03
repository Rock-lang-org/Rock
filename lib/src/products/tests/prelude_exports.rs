use super::*;

#[test]
fn compiler_products_persist_resolver_aliases_by_product_id() {
    let alias_id = DefId::new(CrateId(0), LocalDefId(4));
    let mut hir = resolved_hir_for_products();
    hir.resolver.insert_import_alias_with_name(
        "short".to_string(),
        "demo::plain".to_string(),
        alias_id,
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

    assert_eq!(
        products.identity_table.import_alias_names.get("short"),
        Some(&ProductDefId::from(alias_id))
    );
}

#[test]
fn compiler_products_do_not_export_dependency_aliases_as_current_roots() {
    let dependency_id = DefId::new(CrateId(1), LocalDefId(42));
    let mut hir = resolved_hir_for_products();
    hir.resolver.insert_export_alias_with_name(
        "std::alloc::Global".to_string(),
        "std::alloc::Global".to_string(),
        dependency_id,
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

    assert!(!products
        .identity_table
        .export_names
        .contains_key("std::alloc::Global"));
}

#[test]
fn compiler_products_record_prelude_export_ids() {
    let hir = resolved_hir_for_products();
    let mut products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let plain_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(0)));

    products.record_prelude_export_ids([(
        "plain".to_string(),
        crate::crate_artifact::ArtifactExport {
            source: "plain".to_string(),
            id: DefId::new(CrateId(0), LocalDefId(0)),
        },
    )]);

    assert_eq!(
        products.identity_table.prelude_export_names.get("plain"),
        Some(&plain_id)
    );
}

#[test]
fn compiler_products_record_prelude_export_ids_without_string_payload() {
    let hir = resolved_hir_for_products();
    let mut products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let plain_def_id = DefId::new(CrateId(0), LocalDefId(0));
    let plain_product_id = ProductDefId::from(plain_def_id);

    products.record_prelude_export_ids([(
        "plain".to_string(),
        crate::crate_artifact::ArtifactExport {
            source: "plain".to_string(),
            id: plain_def_id,
        },
    )]);

    assert_eq!(
        products.identity_table.prelude_export_names.get("plain"),
        Some(&plain_product_id)
    );
}

#[test]
fn compiler_products_roundtrip_preserves_prelude_export_ids() {
    let hir = resolved_hir_for_products();
    let mut products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let plain_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(0)));
    products.record_prelude_export_ids([(
        "plain".to_string(),
        crate::crate_artifact::ArtifactExport {
            source: "plain".to_string(),
            id: DefId::new(CrateId(0), LocalDefId(0)),
        },
    )]);

    let bytes = products.to_artifact_bytes().unwrap();
    let decoded = CompilerProducts::from_artifact_bytes(&bytes).unwrap();

    assert_eq!(
        decoded.identity_table.prelude_export_names.get("plain"),
        Some(&plain_id)
    );
}

#[test]
fn compiler_products_record_prelude_export_ids_prefer_explicit_export_id() {
    let hir = resolved_hir_for_products();
    let mut products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let plain_def_id = DefId::new(CrateId(0), LocalDefId(0));
    let plain_id = ProductDefId::from(plain_def_id);

    products.record_prelude_export_ids([(
        "prelude_alias".to_string(),
        crate::crate_artifact::ArtifactExport {
            source: "plain".to_string(),
            id: plain_def_id,
        },
    )]);

    assert_eq!(
        products
            .identity_table
            .prelude_export_names
            .get("prelude_alias"),
        Some(&plain_id)
    );
}

#[test]
fn compiler_products_record_prelude_export_ids_fall_back_on_mismatched_source_id() {
    let hir = resolved_hir_for_products();
    let mut products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let plain_def_id = DefId::new(CrateId(0), LocalDefId(0));
    let identity_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(1)));

    products.record_prelude_export_ids([(
        "prelude_alias".to_string(),
        crate::crate_artifact::ArtifactExport {
            source: "identity".to_string(),
            id: plain_def_id,
        },
    )]);

    assert_eq!(
        products
            .identity_table
            .prelude_export_names
            .get("prelude_alias"),
        Some(&identity_id)
    );
}

#[test]
fn compiler_products_record_prelude_export_ids_skip_non_item_explicit_ids() {
    let hir = resolved_hir_for_products();
    let mut products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");
    let impl_def_id = DefId::new(CrateId(0), LocalDefId(5));
    let impl_id = ProductDefId::from(impl_def_id);

    assert!(products.interface.impls.contains_key(&impl_id));
    assert_eq!(
        products.identity_table.display_names.get(&impl_id),
        Some(&"Box as Show".to_string())
    );

    products.record_prelude_export_ids([(
        "impl_alias".to_string(),
        crate::crate_artifact::ArtifactExport {
            source: "Box as Show".to_string(),
            id: impl_def_id,
        },
    )]);

    assert!(products.identity_table.prelude_export_names.is_empty());
    assert!(!products
        .identity_table
        .prelude_export_names
        .contains_key("impl_alias"));
}

#[test]
fn compiler_products_drop_aliases_for_ambiguous_product_ids() {
    let shared_id = DefId::new(CrateId(0), LocalDefId(0));
    let mut functions = HashMap::new();
    functions.insert(shared_id, test_function(shared_id, "plain", Vec::new()));
    let mut resolver = ResolverTables::default();
    resolver.insert_import_alias_with_name(
        "short".to_string(),
        "demo::plain".to_string(),
        shared_id,
    );
    resolver.insert_module_alias_with_name(
        "mod_short".to_string(),
        "demo::plain".to_string(),
        shared_id,
    );
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
        resolver,
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
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    assert!(!products
        .identity_table
        .import_alias_names
        .contains_key("short"));
    assert!(!products
        .identity_table
        .module_alias_names
        .contains_key("mod_short"));
}

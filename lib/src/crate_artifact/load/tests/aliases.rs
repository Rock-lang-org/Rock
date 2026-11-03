use super::*;

#[test]
fn intern_loaded_interface_types_rejects_one_display_alias_for_distinct_ids() {
    let first = DefId::new(CrateId(0), LocalDefId(1));
    let second = DefId::new(CrateId(0), LocalDefId(2));
    let mut second_function = test_function(second, "payload_second");
    second_function.ret_type = Type::Bool;
    second_function.body = HirBlock {
        stmts: Vec::new(),
        ty: Type::Unit,
    };

    let mut interface = ArtifactCrateInterface::default();
    interface.insert_function(
        "same_alias".to_string(),
        test_function(first, "payload_first"),
    );
    interface.insert_function("same_alias".to_string(), second_function);
    let mut context = crate::type_context::TypeContext::new();

    let error = super::super::intern_loaded_interface_types(
        &interface,
        &ResolverTables::default(),
        &mut context,
    )
    .expect_err("distinct artifact IDs must not share one display alias");

    assert!(error.contains("artifact function display alias 'same_alias'"));
}

#[test]
fn artifact_name_tables_keep_resolver_aliases_on_the_payload_def_id() {
    let id = DefId::new(CrateId(0), LocalDefId(3));
    let function = test_function(id, "payload_display");
    let mut interface = ArtifactCrateInterface::default();
    interface.insert_function("dep::canonical".to_string(), function.clone());
    let mut resolver = ResolverTables::default();
    resolver
        .item_names_by_id
        .insert(id, "dep::canonical".to_string());
    resolver.import_aliases.insert("short".to_string(), id);
    let unresolved =
        crate::crate_artifact::types::hir_function_from_interface(&interface.functions[&id]);

    let names = super::super::artifact_hir_name_tables(
        &interface,
        &resolver,
        &HashMap::from([(id, unresolved)]),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    )
    .expect("resolver alias should remain attached to the exact artifact DefId");

    assert_eq!(names.functions_by_name["short"], id);
    assert_eq!(names.functions_by_name["dep::canonical"], id);
}

#[test]
fn artifact_alias_conflicts_are_stable_across_reverse_insertion() {
    fn conflict(reverse: bool) -> String {
        let first = DefId::new(CrateId(0), LocalDefId(4));
        let second = DefId::new(CrateId(0), LocalDefId(5));
        let mut resolver = ResolverTables::default();
        let (first_source, second_source) = if reverse {
            (first, second)
        } else {
            (second, first)
        };
        resolver.item_paths.insert("same".to_string(), first_source);
        resolver
            .item_names_by_id
            .insert(second_source, "same".to_string());
        let functions = HashMap::from([
            (
                first,
                crate::crate_artifact::types::hir_function_from_interface(
                    &ProductFunctionInterface::from(&test_function(first, "first")),
                ),
            ),
            (
                second,
                crate::crate_artifact::types::hir_function_from_interface(
                    &ProductFunctionInterface::from(&test_function(second, "second")),
                ),
            ),
        ]);

        super::super::artifact_hir_name_tables(
            &ArtifactCrateInterface::default(),
            &resolver,
            &functions,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        )
        .expect_err("same artifact alias must have a stable conflict")
    }

    let expected = "artifact function display alias 'same' resolves to both DefId { crate_id: CrateId(0), local: LocalDefId(4) } and DefId { crate_id: CrateId(0), local: LocalDefId(5) }";
    assert_eq!(conflict(false), expected);
    assert_eq!(conflict(true), expected);
}

#[test]
fn load_product_artifact_remaps_id_backed_import_aliases() {
    let (base, _cleanup) = temp_test_dir("id_backed_import_aliases");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 1, object_path);
    products
        .identity_table
        .import_alias_names
        .insert("short".to_string(), product_def_id(0, 1));

    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();
    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();
    let record = ctx.extern_crate("dep").unwrap();
    let resolver = record.metadata().resolver();

    assert_eq!(
        resolver.resolve_item_or_alias("short"),
        resolver.resolve_item_or_alias("dep::answer")
    );
}

#[test]
fn load_product_artifact_drops_aliases_without_canonical_bindings() {
    let (base, _cleanup) = temp_test_dir("stale_id_backed_aliases");
    let object_path = base.join("dep.o");
    fs::write(&object_path, []).unwrap();
    let mut products = product_with_function("dep", ProductCrateId(0), 1, object_path);
    products
        .identity_table
        .import_alias_names
        .insert("stale".to_string(), product_def_id(0, 99));
    products
        .identity_table
        .module_alias_names
        .insert("stale_mod".to_string(), product_def_id(0, 100));

    let artifact_path = base.join("dep.rkca");
    products.write_artifact_to_path(&artifact_path).unwrap();
    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path(artifact_path).unwrap();
    let record = ctx.extern_crate("dep").unwrap();
    let resolver = record.metadata().resolver();

    assert_eq!(resolver.resolve_item_or_alias("stale"), None);
    assert_eq!(resolver.resolve_item_or_alias("stale_mod"), None);
}

#[test]
fn resolver_from_products_ignores_impl_display_names() {
    let impl_id = DefId::new(CrateId(1), LocalDefId(8));
    let product_id = ProductDefId::from(impl_id);
    let mut identity_table = ProductIdentityTable::default();
    identity_table
        .display_names
        .insert(product_id, "Box as Show".to_string());

    let mut interface = ProductInterface::default();
    interface.impls.insert(
        product_id,
        ProductImplInterface::from(&HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
            trait_name: Some("Show".to_string()),
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        }),
    );
    let products = CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dep".to_string()),
        identity_table,
        interface,
        bodies: Default::default(),
        link: ProductLinkData::default(),
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: Default::default(),
        proc_macros: Vec::new(),
    };

    let remap = super::super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::new(),
    };
    let resolver =
        super::super::resolver_from_products(&products, "dep", &remap, &CrateContext::new())
            .unwrap();

    assert!(!resolver.item_paths.contains_key("Box as Show"));
}

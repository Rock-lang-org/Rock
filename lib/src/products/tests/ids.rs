use super::*;

#[test]
fn product_def_id_preserves_def_id_raw_parts() {
    let def_id = DefId::new(CrateId(7), LocalDefId(42));

    let product_id = ProductDefId::from(def_id);

    assert_eq!(product_id.crate_id, ProductCrateId(7));
    assert_eq!(product_id.local_id, ProductLocalDefId(42));
}

#[test]
fn product_def_id_is_usable_as_stable_btree_key() {
    let first = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(2),
    };
    let second = ProductDefId {
        crate_id: ProductCrateId(0),
        local_id: ProductLocalDefId(1),
    };

    let mut map = BTreeMap::new();
    map.insert(first, "second item".to_string());
    map.insert(second, "first item".to_string());

    let keys = map.keys().copied().collect::<Vec<_>>();
    assert_eq!(keys, vec![second, first]);
}

#[test]
fn compiler_products_roundtrip_preserves_product_def_ids() {
    let hir = resolved_hir_for_products();
    let products = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("demo".to_string()),
        &hir,
        vec![ProductDependencyIdentity::artifact_object(
            "dep".to_string(),
            std::path::PathBuf::from("build/dep.rkca"),
        )],
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .expect("test HIR has valid language items");

    let bytes = bincode::serialize(&products).unwrap();
    let decoded: CompilerProducts = bincode::deserialize(&bytes).unwrap();

    let generic_id = ProductDefId::from(DefId::new(CrateId(0), LocalDefId(1)));
    assert!(decoded.interface.functions.contains_key(&generic_id));
    assert!(decoded.bodies.functions.contains_key(&generic_id));
    assert_eq!(
        decoded.identity_table.export_names.get("identity"),
        Some(&generic_id)
    );
    assert_eq!(decoded.dependencies[0].name, "dep");
    assert_eq!(
        decoded.dependencies[0].artifact_path,
        std::path::PathBuf::from("build/dep.rkca")
    );
}

#[test]
fn compiler_products_preserve_generic_impls_with_real_local_id_zero() {
    let mut hir = resolved_hir_for_products();
    let first_id = DefId::new(CrateId(0), LocalDefId(0));
    let second_id = DefId::new(CrateId(0), LocalDefId(99));
    hir.current_def_ids.insert(second_id);
    rebuild_program(
        &mut hir.program,
        |_, _, _, _, impls, _, _, effective_methods| {
            *impls = [
                HirImplFor::<AcceptedHir> {
                    id: first_id,
                    owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                    type_name: "Box".to_string(),
                    type_generics: generic_params(first_id, &["T"]),
                    receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                        id: DefId::new(CrateId(0), LocalDefId(2)),
                        args: vec![Type::Generic(crate::types::GenericParamId {
                            owner: first_id,
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
                HirImplFor::<AcceptedHir> {
                    id: second_id,
                    owner: crate::hir::HirImplOwner::Named("Maybe".to_string()),
                    type_name: "Maybe".to_string(),
                    type_generics: generic_params(second_id, &["T"]),
                    receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Enum {
                        id: DefId::new(CrateId(0), LocalDefId(3)),
                        args: vec![Type::Generic(crate::types::GenericParamId {
                            owner: second_id,
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
            ]
            .into_iter()
            .map(|impl_def| (impl_def.id, impl_def))
            .collect();
            effective_methods.clear();
        },
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

    assert_eq!(products.interface.impls.len(), 2);
    assert_eq!(products.bodies.generic_impls.len(), 2);
}

#[test]
fn compiler_products_preserve_real_impl_id_zero_without_collision() {
    let impl_id = DefId::new(CrateId(0), LocalDefId(0));
    let owner_id = DefId::new(CrateId(0), LocalDefId(1));
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
        HashMap::from([(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                    id: owner_id,
                    args: Vec::new(),
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
            structs_by_name: HashMap::from([("Box".to_string(), owner_id)]),
            ..HirNameTables::default()
        },
        HirLanguageItems::default(),
        &HashMap::from([(owner_id, "Box".to_string())]),
    );
    let hir = ResolvedHirProgram::new(
        program,
        ResolverTables::default(),
        BTreeSet::from([impl_id]),
        CrateId(0),
        local_def_ids_after(1),
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
    let product_id = ProductDefId::from(impl_id);

    assert!(products.interface.impls.contains_key(&product_id));
    assert_eq!(products.interface.impls[&product_id].id, impl_id);
}

#[test]
fn compiler_products_preserve_real_extern_id_zero_without_collision() {
    let extern_id = DefId::new(CrateId(0), LocalDefId(0));
    let hir = ResolvedHirProgram::new(
        HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(
                extern_id,
                HirExtern {
                    id: extern_id,
                    name: "puts".to_string(),
                    params: vec![Type::Pointer(Box::new(Type::U8))],
                    ret: Type::I32,
                    variadic: false,
                    is_unsafe: false,
                },
            )]),
            HirNameTables {
                externs_by_name: HashMap::from([("puts".to_string(), extern_id)]),
                ..HirNameTables::default()
            },
            HirLanguageItems::default(),
            &HashMap::new(),
        ),
        ResolverTables::default(),
        BTreeSet::from([extern_id]),
        CrateId(0),
        local_def_ids_after(1),
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
    let product_id = ProductDefId::from(extern_id);

    assert!(products.interface.externs.contains_key(&product_id));
    assert_eq!(products.interface.externs[&product_id].id, extern_id);
}

#[test]
fn compiler_products_preserve_link_records_for_real_local_id_zero() {
    let impl_def_id = DefId::new(CrateId(0), LocalDefId(0));
    let mut records = BTreeMap::new();
    records.insert(
        ProductDefId::from(impl_def_id),
        ProductLinkRecord {
            backend_symbol: "impl_box_show".to_string(),
        },
    );
    let owner_id = DefId::new(CrateId(0), LocalDefId(1));
    let program = HirProgram::from_id_parts_with_names_and_canonical_names(
        HashMap::new(),
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
            impl_def_id,
            HirImpl {
                id: impl_def_id,
                owner: crate::hir::HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: generic_params(impl_def_id, &["T"]),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                    id: owner_id,
                    args: vec![Type::Generic(crate::types::GenericParamId {
                        owner: impl_def_id,
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
            structs_by_name: HashMap::from([("Box".to_string(), owner_id)]),
            ..HirNameTables::default()
        },
        HirLanguageItems::default(),
        &HashMap::from([(owner_id, "Box".to_string())]),
    );
    let hir = ResolvedHirProgram::new(
        program,
        ResolverTables::default(),
        BTreeSet::from([impl_def_id]),
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
    let impl_id = products.interface.impls.keys().copied().next().unwrap();

    assert_eq!(impl_id, ProductDefId::from(impl_def_id));
    assert_eq!(
        products.interface.impls[&impl_id].id,
        super::product_def_id_to_def_id(impl_id)
    );
    assert_eq!(
        products
            .link
            .records
            .get(&impl_id)
            .map(|record| &record.backend_symbol),
        Some(&"impl_box_show".to_string())
    );
}

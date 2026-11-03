use super::*;

#[test]
fn extern_crate_link_makes_object_path_required_for_object_backed_dependencies() {
    let link = ExternCrateLink::object(PathBuf::from("/dep/dep.o"), BTreeMap::new());

    assert!(link.is_object_backed());
    assert_eq!(link.object_path(), Some(&PathBuf::from("/dep/dep.o")));
}

#[test]
fn extern_crate_ref_imported_function_symbol_requires_link_record() {
    let function_id = DefId::new(CrateId(9), LocalDefId(1));
    let function = hir_function_with_id(function_id, "answer", Some("dep::answer"), &[]);
    let mut interface = ArtifactCrateInterface::default();
    interface.insert_function("dep::answer".to_string(), function.clone());
    let metadata = ExternCrateMetadata::new(interface, ResolverTables::default(), BTreeMap::new());
    let record = ExternCrateRecord::new(
        CrateId(9),
        "dep".to_string(),
        metadata,
        ExternCrateBodies::default(),
        ExternCrateLink::object(PathBuf::from("/dep/dep.o"), BTreeMap::new()),
    );
    let mut store = ExternCrateStore::default();
    store.insert(record).unwrap();
    let dep = store.by_name("dep").unwrap();

    assert_eq!(dep.imported_function_symbol("dep::answer", &function), None);
}

#[test]
fn extern_crate_ref_imported_impl_method_symbol_requires_link_record() {
    let impl_id = DefId::new(CrateId(9), LocalDefId(2));
    let method_id = DefId::new(CrateId(9), LocalDefId(3));
    let mut method = hir_function_with_id(method_id, "make", Some("dep::Box::make"), &[]);
    method.is_method = true;
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: Vec::new().into(),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods: HashMap::from([("make".to_string(), method.clone())]),
    };
    let mut interface = ArtifactCrateInterface::default();
    interface.insert_impl(imp.clone());
    let metadata = ExternCrateMetadata::new(interface, ResolverTables::default(), BTreeMap::new());
    let record = ExternCrateRecord::new(
        CrateId(9),
        "dep".to_string(),
        metadata,
        ExternCrateBodies::default(),
        ExternCrateLink::object(PathBuf::from("/dep/dep.o"), BTreeMap::new()),
    );
    let mut store = ExternCrateStore::default();
    store.insert(record).unwrap();
    let dep = store.by_name("dep").unwrap();

    assert!(!dep.provides_impl_method_body(&imp, &method));
    assert_eq!(dep.imported_impl_method_symbol(&imp, "make", &method), None);
}

#[test]
fn extern_crate_ref_imported_impl_method_allows_concrete_trait_args() {
    let impl_id = DefId::new(CrateId(9), LocalDefId(4));
    let method_id = DefId::new(CrateId(9), LocalDefId(5));
    let mut method = hir_function_with_id(method_id, "+", Some("dep::I64_+"), &[]);
    method.is_method = true;
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("I64".to_string()),
        type_name: "I64".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: Vec::new().into(),
        trait_name: Some("Add".to_string()),
        trait_id: Some(DefId::new(CrateId(9), LocalDefId(6))),
        trait_generics: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: DefId::new(CrateId(9), LocalDefId(6)),
                index: 0,
            },
            "Rhs",
        )],
        trait_arg_types: vec![Type::I64],
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods: HashMap::from([("+".to_string(), method.clone())]),
    };
    let mut interface = ArtifactCrateInterface::default();
    interface.insert_impl(imp.clone());
    let metadata = ExternCrateMetadata::new(interface, ResolverTables::default(), BTreeMap::new());
    let record = ExternCrateRecord::new(
        CrateId(9),
        "dep".to_string(),
        metadata,
        ExternCrateBodies::default(),
        ExternCrateLink::object(
            PathBuf::from("/dep/dep.o"),
            BTreeMap::from([(method_id, "dep_I64_add".to_string())]),
        ),
    );
    let mut store = ExternCrateStore::default();
    store.insert(record).unwrap();
    let dep = store.by_name("dep").unwrap();

    assert!(dep.provides_impl_method_body(&imp, &method));
    assert_eq!(
        dep.imported_impl_method_symbol(&imp, "+", &method),
        Some("dep_I64_add".to_string())
    );
}

#[test]
fn extern_crate_ref_concrete_impl_body_rejects_generic_methods_with_link_records() {
    let impl_id = DefId::new(CrateId(9), LocalDefId(7));
    let method_id = DefId::new(CrateId(9), LocalDefId(8));
    let mut method = hir_function_with_id(method_id, "make", Some("dep::Box::make"), &["T"]);
    method.is_method = true;
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: Vec::new().into(),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods: HashMap::from([("make".to_string(), method.clone())]),
    };
    let mut interface = ArtifactCrateInterface::default();
    interface.insert_impl(imp.clone());
    let metadata = ExternCrateMetadata::new(interface, ResolverTables::default(), BTreeMap::new());
    let record = ExternCrateRecord::new(
        CrateId(9),
        "dep".to_string(),
        metadata,
        ExternCrateBodies::default(),
        ExternCrateLink::object(
            PathBuf::from("/dep/dep.o"),
            BTreeMap::from([(method_id, "dep_Box_make".to_string())]),
        ),
    );
    let mut store = ExternCrateStore::default();
    store.insert(record).unwrap();
    let dep = store.by_name("dep").unwrap();

    assert!(!dep.provides_impl_body(&imp));
    assert!(!dep.provides_impl_method_body(&imp, &method));
    assert_eq!(dep.imported_impl_method_symbol(&imp, "make", &method), None);
}

#[test]
fn extern_crate_ref_imported_symbols_use_explicit_link_records() {
    let function_id = DefId::new(CrateId(9), LocalDefId(1));
    let function = hir_function_with_id(function_id, "answer", Some("dep::answer"), &[]);
    let mut interface = ArtifactCrateInterface::default();
    interface.insert_function("dep::answer".to_string(), function.clone());
    let metadata = ExternCrateMetadata::new(interface, ResolverTables::default(), BTreeMap::new());
    let record = ExternCrateRecord::new(
        CrateId(9),
        "dep".to_string(),
        metadata,
        ExternCrateBodies::default(),
        ExternCrateLink::object(
            PathBuf::from("/dep/dep.o"),
            BTreeMap::from([(function_id, "dep_answer".to_string())]),
        ),
    );
    let mut store = ExternCrateStore::default();
    store.insert(record).unwrap();
    let dep = store.by_name("dep").unwrap();

    assert_eq!(
        dep.imported_function_symbol("dep::answer", &function),
        Some("dep_answer".to_string())
    );
}

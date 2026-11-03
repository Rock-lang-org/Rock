use super::*;

#[test]
fn crate_context_adds_artifact_records_to_extern_store() {
    let mut ctx = CrateContext::new();
    let crate_id = CrateId(5);
    ctx.add_extern_crate(ExternCrateRecord::new(
        crate_id,
        "dep".to_string(),
        ExternCrateMetadata::empty_for_test(),
        ExternCrateBodies::default(),
        ExternCrateLink::metadata_only(BTreeMap::new()),
    ))
    .expect("extern crate should insert");

    assert!(ctx.has_crate("dep"));
    assert!(!ctx.has_source_crate("dep"));
    assert!(ctx.has_extern_crate("dep"));
    assert_eq!(ctx.extern_crate("dep").unwrap().crate_id(), crate_id);
    assert_eq!(ctx.extern_crates().count(), 1);
}

#[test]
fn extern_crate_store_records_artifact_capabilities_by_session_crate_id() {
    let crate_id = CrateId(42);
    let answer_id = DefId::new(crate_id, LocalDefId(7));
    let mut interface = ArtifactCrateInterface::default();
    interface.insert_function(
        "dep::answer".to_string(),
        hir_function_with_id(answer_id, "answer", Some("dep::answer"), &[]),
    );
    interface.root_export_ids.insert(
        "answer".to_string(),
        ArtifactExport {
            source: "dep::answer".to_string(),
            id: answer_id,
        },
    );

    let mut resolver = ResolverTables::default();
    resolver
        .item_paths
        .insert("dep::answer".to_string(), answer_id);
    resolver
        .item_names_by_id
        .insert(answer_id, "dep::answer".to_string());

    let record = ExternCrateRecord::new(
        crate_id,
        "dep".to_string(),
        ExternCrateMetadata::new(
            interface,
            resolver,
            BTreeMap::from([(
                "pipe".to_string(),
                ArtifactExport {
                    source: "dep::prelude::pipe".to_string(),
                    id: answer_id,
                },
            )]),
        ),
        ExternCrateBodies::default(),
        ExternCrateLink::object(
            PathBuf::from("/dep/dep.o"),
            BTreeMap::from([(answer_id, "dep_answer".to_string())]),
        ),
    );
    let mut store = ExternCrateStore::default();
    store.insert(record).expect("extern record should insert");

    let dep = store.by_name("dep").expect("dep extern crate should exist");
    assert_eq!(dep.crate_id(), crate_id);
    assert_eq!(
        dep.metadata().interface().canonical_name(answer_id),
        Some("dep::answer")
    );
    assert_eq!(
        dep.metadata()
            .prelude_export_ids()
            .get("pipe")
            .map(|export| export.source.as_str()),
        Some("dep::prelude::pipe")
    );
    assert_eq!(dep.link().object_path(), Some(&PathBuf::from("/dep/dep.o")));
    assert_eq!(dep.link().backend_symbol(answer_id), Some("dep_answer"));
}

#[test]
fn extern_crate_store_rejects_duplicate_names_and_ids() {
    let mut store = ExternCrateStore::default();
    let first = ExternCrateRecord::new(
        CrateId(1),
        "dep".to_string(),
        ExternCrateMetadata::empty_for_test(),
        ExternCrateBodies::default(),
        ExternCrateLink::metadata_only(BTreeMap::new()),
    );
    store
        .insert(first)
        .expect("first extern crate should insert");

    let duplicate_name = ExternCrateRecord::new(
        CrateId(2),
        "dep".to_string(),
        ExternCrateMetadata::empty_for_test(),
        ExternCrateBodies::default(),
        ExternCrateLink::metadata_only(BTreeMap::new()),
    );
    let name_error = store
        .insert(duplicate_name)
        .expect_err("duplicate extern crate name should fail");
    assert!(name_error.contains("duplicate external crate name 'dep'"));

    let duplicate_id = ExternCrateRecord::new(
        CrateId(1),
        "other".to_string(),
        ExternCrateMetadata::empty_for_test(),
        ExternCrateBodies::default(),
        ExternCrateLink::metadata_only(BTreeMap::new()),
    );
    let id_error = store
        .insert(duplicate_id)
        .expect_err("duplicate extern crate id should fail");
    assert!(id_error.contains("duplicate external crate id 1"));
}

#[test]
fn extern_crate_store_indexes_and_collects_object_link_inputs() {
    let metadata_id = CrateId(4);
    let object_id = CrateId(5);
    let mut store = ExternCrateStore::default();
    store
        .insert(ExternCrateRecord::new(
            metadata_id,
            "meta".to_string(),
            ExternCrateMetadata::empty_for_test(),
            ExternCrateBodies::default(),
            ExternCrateLink::metadata_only(BTreeMap::new()),
        ))
        .expect("metadata-only extern crate should insert");
    store
        .insert(ExternCrateRecord::new(
            object_id,
            "obj".to_string(),
            ExternCrateMetadata::empty_for_test(),
            ExternCrateBodies::default(),
            ExternCrateLink::object(PathBuf::from("/dep/obj.o"), BTreeMap::new()),
        ))
        .expect("object-backed extern crate should insert");

    assert!(store.contains_name("meta"));
    assert!(store.contains_name("obj"));
    assert_eq!(store.by_crate_id(metadata_id).unwrap().name(), "meta");
    assert_eq!(store.by_crate_id(object_id).unwrap().name(), "obj");
    assert_eq!(
        store.iter().map(|dep| dep.name()).collect::<Vec<_>>(),
        vec!["meta", "obj"]
    );

    let inputs = store.link_inputs();
    assert_eq!(inputs.object_crate_names, vec!["obj".to_string()]);
    assert_eq!(inputs.object_paths, vec![PathBuf::from("/dep/obj.o")]);
}

#[test]
fn extern_crate_bodies_expose_categories_without_raw_bundle_access() {
    let id = DefId::new(CrateId(3), LocalDefId(1));
    let trait_id = DefId::new(CrateId(3), LocalDefId(2));
    let impl_id = DefId::new(CrateId(3), LocalDefId(3));
    let mut bundle = ArtifactCrossCrateHir {
        generic_functions: BTreeMap::new(),
        traits_with_defaults: BTreeMap::new(),
        generic_impls: Vec::new(),
    };
    bundle
        .generic_functions
        .insert(id, hir_function_with_id(id, "id", Some("dep::id"), &["T"]));
    bundle.traits_with_defaults.insert(
        trait_id,
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Trait".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    bundle.generic_impls.push(HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: impl_id,
                index: 0,
            },
            "T",
        )],
        receiver_pattern: Vec::new().into(),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::new(),
    });

    let bodies = ExternCrateBodies::from_cross_crate_hir(bundle);

    assert_eq!(
        bodies
            .providers()
            .generic_function(id)
            .expect("generic body provider")
            .name,
        "id"
    );
    assert!(bodies.providers().trait_with_defaults(trait_id).is_some());
    assert!(bodies.providers().generic_impl(impl_id).is_some());
}

#[test]
fn crate_context_collects_dependency_link_inputs_from_provider_capabilities() {
    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(ExternCrateRecord::new(
        CrateId(9),
        "dep".to_string(),
        ExternCrateMetadata::empty_for_test(),
        ExternCrateBodies::default(),
        ExternCrateLink::object(PathBuf::from("/dep/dep.o"), BTreeMap::new()),
    ))
    .expect("extern crate should insert");

    let inputs = ctx.dependency_link_inputs();

    assert_eq!(inputs.object_crate_names, vec!["dep".to_string()]);
    assert_eq!(inputs.object_paths, vec![PathBuf::from("/dep/dep.o")]);
}

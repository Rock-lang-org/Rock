use super::*;

fn loaded_object_backed_function_crate_with_backend_symbols(
    crate_name: &str,
    canonical_name: &str,
    func: HirFunction,
    object_path: PathBuf,
    backend_symbols: BTreeMap<DefId, String>,
) -> ExternCrateRecord {
    let mut interface = ArtifactCrateInterface::default();
    interface.insert_function(canonical_name.to_string(), func);

    let mut resolver = ResolverTables::default();
    let def_id = DefId::new(CrateId(0), LocalDefId(0));
    resolver
        .item_paths
        .insert(canonical_name.to_string(), def_id);
    resolver
        .item_names_by_id
        .insert(def_id, canonical_name.to_string());

    ExternCrateRecord::new(
        CrateId(1),
        crate_name.to_string(),
        ExternCrateMetadata::new(interface, resolver, BTreeMap::new()),
        ExternCrateBodies::from_cross_crate_hir(ArtifactCrossCrateHir {
            generic_functions: BTreeMap::new(),
            traits_with_defaults: BTreeMap::new(),
            generic_impls: vec![],
        }),
        ExternCrateLink::object(object_path, backend_symbols),
    )
}

#[test]
fn process_with_crates_uses_link_provider_for_object_backed_functions() {
    let answer_id = DefId::new(CrateId(0), LocalDefId(0));
    let func = simple_hir_function("answer", Some("dep::answer"), answer_id);
    let loaded = loaded_object_backed_function_crate_with_backend_symbols(
        "dep",
        "dep::answer",
        func,
        PathBuf::from("dep.o"),
        BTreeMap::from([(answer_id, "dep::answer".to_string())]),
    );
    let mut crate_ctx = CrateContext::new();
    crate_ctx.add_extern_crate(loaded).unwrap();

    let program =
        crate::mono::hir_types::HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            crate::hir::HirNameTables::default(),
            &HashMap::new(),
        );

    let mut mono = Monomorphizer::new();
    let output = wrapped_process_with_crates(&mut mono, program, &crate_ctx);

    assert!(output
        .instances
        .values()
        .any(|record| record.symbols.backend_symbol == "dep::answer" && record.provided_by_object));
}

#[test]
fn process_with_crates_uses_artifact_backend_symbols_for_object_backed_functions() {
    let answer_id = DefId::new(CrateId(0), LocalDefId(7));
    let func = simple_hir_function("answer", Some("dep::answer"), answer_id);
    let loaded = loaded_object_backed_function_crate_with_backend_symbols(
        "dep",
        "dep::answer",
        func,
        PathBuf::from("dep.o"),
        BTreeMap::from([(answer_id, "artifact_dep_answer".to_string())]),
    );
    let mut crate_ctx = CrateContext::new();
    crate_ctx.add_extern_crate(loaded).unwrap();

    let program =
        crate::mono::hir_types::HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            crate::hir::HirNameTables::default(),
            &HashMap::new(),
        );

    let mut mono = Monomorphizer::new();
    let output = wrapped_process_with_crates(&mut mono, program, &crate_ctx);

    let record = output
        .instances
        .values()
        .find(|record| record.provided_by_object)
        .expect("object-backed function instance should be recorded");
    assert_eq!(record.symbols.source_name, "dep::answer");
    assert_eq!(record.symbols.backend_symbol, "artifact_dep_answer");
}

#[test]
fn test_load_external_generic_functions_keeps_concrete_object_backed_trait_impls() {
    let trait_id = DefId::new(CrateId(0), LocalDefId(99));
    let byte_slice_ty = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Slice(Box::new(Type::U8))),
    };

    let concrete_method_id = DefId::new(CrateId(0), LocalDefId(1));
    let mut concrete_method = println_method(byte_slice_ty.clone());
    concrete_method.id = concrete_method_id;
    let concrete_impl = HirImpl {
        id: DefId::new(CrateId(0), LocalDefId(0)),
        owner: HirImplOwner::BuiltinSlice,
        type_name: "&[U8]".to_string(),
        type_generics: vec![],
        receiver_pattern: vec![Type::U8].into(),
        trait_name: Some("Show".to_string()),
        trait_id: Some(trait_id),
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("println".to_string(), concrete_method)]),
    };
    let generic_impl = HirImpl {
        id: DefId::new(CrateId(0), LocalDefId(0)),
        owner: HirImplOwner::BuiltinSlice,
        type_name: "Array".to_string(),
        type_generics: vec![GenericParamDecl::type_param(
            crate::types::GenericParamId {
                owner: crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(0)),
                index: 0,
            },
            "T",
        )],
        receiver_pattern: vec![Type::Generic(crate::types::GenericParamId {
            owner: crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(0)),
            index: 0,
        })]
        .into(),
        trait_name: Some("Show".to_string()),
        trait_id: Some(trait_id),
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([(
            "println".to_string(),
            println_method(Type::Slice(Box::new(Type::Generic(
                crate::types::GenericParamId {
                    owner: crate::ids::DefId::new(
                        crate::ids::CrateId(0),
                        crate::ids::LocalDefId(0),
                    ),
                    index: 0,
                },
            )))),
        )]),
    };

    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(loaded_object_backed_crate_with_backend_symbols(
        vec![concrete_impl.clone()],
        vec![generic_impl],
        BTreeMap::from([(concrete_method_id, "&[U8]_println".to_string())]),
    ))
    .unwrap();

    let mut mono = Monomorphizer::new();
    mono.load_external_generic_functions(&ctx);

    let show_impls = mono.trait_impls.get(&trait_id).unwrap();
    assert!(show_impls.iter().any(|imp| imp.type_name == "&[U8]"));

    let recv = HirExpr {
        kind: HirExprKind::Var("bytes".to_string()),
        ty: byte_slice_ty,
        span: Span::test(),
    };
    let mut expr = HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(recv.clone()),
            "println".to_string(),
            vec![],
            Some(ReceiverMode::Move),
            HirMethodCallTarget::trait_method(
                DefId::new(CrateId(0), LocalDefId(999)),
                DefId::new(CrateId(0), LocalDefId(998)),
                Vec::new(),
                crate::hir::HirTraitDispatchKind::TraitBound,
            ),
        ),
        ty: Type::I32,
        span: Span::test(),
    };

    let _ = mono.monomorphize_trait_method_call("println", &[recv], &mut expr);

    assert!(matches!(expr.kind, HirExprKind::MethodCall(..)));
}

#[test]
fn load_external_generic_functions_keeps_generic_trait_impls_out_of_inherent_table() {
    let generic_trait_impl = HirImpl {
        id: DefId::new(CrateId(0), LocalDefId(0)),
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: vec![GenericParamDecl::type_param(
            crate::types::GenericParamId {
                owner: crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(0)),
                index: 0,
            },
            "T",
        )],
        receiver_pattern: vec![Type::Generic(crate::types::GenericParamId {
            owner: crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(0)),
            index: 0,
        })]
        .into(),
        trait_name: Some("Show".to_string()),
        trait_id: Some(DefId::new(CrateId(0), LocalDefId(99))),
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([(
            "println".to_string(),
            println_method(Type::Struct {
                id: DefId::new(CrateId(0), LocalDefId(10)),
                args: vec![Type::Generic(crate::types::GenericParamId {
                    owner: crate::ids::DefId::new(
                        crate::ids::CrateId(0),
                        crate::ids::LocalDefId(0),
                    ),
                    index: 0,
                })],
            }),
        )]),
    };

    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(loaded_object_backed_crate(vec![], vec![generic_trait_impl]))
        .unwrap();

    let mut mono = Monomorphizer::new();
    mono.load_external_generic_functions(&ctx);

    assert!(!mono
        .generic_impls
        .values()
        .any(|imp| imp.type_name == "Box"));
    assert!(mono
        .trait_impls
        .get(&DefId::new(CrateId(0), LocalDefId(99)))
        .is_some_and(|impls| impls.iter().any(|imp| imp.type_name == "Box")));
}

#[test]
fn load_external_generic_functions_ignores_module_aliases_for_generic_functions() {
    let function_id = DefId::new(CrateId(0), LocalDefId(80));
    let generic_param = crate::types::GenericParamId {
        owner: function_id,
        index: 0,
    };
    let mut function = simple_hir_function("identity", Some("dep::generic::identity"), function_id);
    function.generic_params = vec![GenericParamDecl::type_param(generic_param, "T")];
    function.ret_type = Type::Generic(generic_param);
    function.body.ty = Type::Generic(generic_param);
    let canonical_name = "dep::generic::identity".to_string();

    let mut resolver = ResolverTables::default();
    resolver
        .item_paths
        .insert(canonical_name.clone(), function_id);
    resolver
        .item_names_by_id
        .insert(function_id, canonical_name.clone());
    resolver
        .module_aliases
        .insert("identity".to_string(), function_id);
    let loaded = ExternCrateRecord::new(
        CrateId(1),
        "dep".to_string(),
        ExternCrateMetadata::new(ArtifactCrateInterface::default(), resolver, BTreeMap::new()),
        ExternCrateBodies::from_cross_crate_hir(ArtifactCrossCrateHir {
            generic_functions: BTreeMap::from([(function_id, function)]),
            traits_with_defaults: BTreeMap::new(),
            generic_impls: Vec::new(),
        }),
        ExternCrateLink::metadata_only(BTreeMap::new()),
    );
    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(loaded).unwrap();
    let mut mono = Monomorphizer::new();

    mono.load_external_generic_functions(&ctx);

    assert!(mono.generic_functions.contains_key(&function_id));
}

#[test]
fn load_external_generic_functions_skips_concrete_object_backed_inherent_impls() {
    let impl_id = DefId::new(CrateId(0), LocalDefId(70));
    let method_id = DefId::new(CrateId(0), LocalDefId(71));
    let mut method = simple_hir_function("from_str", Some("std::String_from_str"), method_id);
    method.params = vec![HirParam {
        name: "s".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::Reference {
            mutable: false,
            inner: Box::new(Type::Str),
        },
        mutable: false,
        is_ref: false,
    }];
    method.ret_type = Type::Struct {
        id: DefId::new(CrateId(0), LocalDefId(72)),
        args: Vec::new(),
    };
    let concrete_impl = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("String".to_string()),
        type_name: "String".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: Vec::new().into(),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("from_str".to_string(), method)]),
    };
    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(loaded_object_backed_crate(
        vec![concrete_impl.clone()],
        vec![concrete_impl],
    ))
    .unwrap();

    let mut mono = Monomorphizer::new();
    mono.load_external_generic_functions(&ctx);

    assert!(!mono
        .generic_impls
        .values()
        .any(|imp| imp.type_name == "String"));
}

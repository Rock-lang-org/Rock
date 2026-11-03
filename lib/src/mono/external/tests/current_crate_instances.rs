use super::*;

#[test]
fn process_with_crates_records_current_crate_concrete_function_instance() {
    let function_id = DefId::new(CrateId(0), LocalDefId(30));
    let function = concrete_function(function_id, "main", Type::I32);
    let program =
        crate::mono::hir_types::HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            HashMap::from([(function_id, function)]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            crate::hir::HirNameTables {
                functions_by_name: HashMap::from([("main".to_string(), function_id)]),
                ..crate::hir::HirNameTables::default()
            },
            &HashMap::from([(function_id, "main".to_string())]),
        );
    let ctx = CrateContext::new();
    let mut mono = Monomorphizer::new();
    mono.resolver
        .item_paths
        .insert("main".to_string(), function_id);

    let output = wrapped_process_with_crates(&mut mono, program, &ctx);

    let record = output
        .instances
        .values()
        .find(|record| record.origin == crate::mono::InstanceOrigin::Function(function_id))
        .expect("current-crate concrete function should be an instance");
    assert!(record.substitution.is_empty());
    assert_eq!(record.symbols.source_name, "main");
    assert_eq!(record.symbols.backend_symbol, "main");
    assert!(output.pre_mir_instance_bodies.contains(record.id));
    assert!(!record.provided_by_object);
}

#[test]
fn process_with_crates_records_current_crate_concrete_impl_method_instance() {
    let impl_id = DefId::new(CrateId(0), LocalDefId(40));
    let method_id = DefId::new(CrateId(0), LocalDefId(41));
    let mut method = println_method(Type::I64);
    method.id = method_id;
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: vec![],
        receiver_pattern: vec![].into(),
        trait_name: None,
        trait_id: None,
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("println".to_string(), method)]),
    };
    let program =
        crate::mono::hir_types::HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(impl_id, imp)]),
            HashMap::new(),
            crate::hir::HirNameTables::default(),
            &HashMap::from([(impl_id, "Box".to_string())]),
        );
    let ctx = CrateContext::new();
    let mut mono = Monomorphizer::new();

    let output = wrapped_process_with_crates(&mut mono, program, &ctx);

    let record = output
        .instances
        .values()
        .find(|record| {
            record.origin
                == crate::mono::InstanceOrigin::ImplMethod {
                    owner: crate::mono::InstanceImplOwner::Named(impl_id),
                    method: method_id,
                }
        })
        .expect("current-crate concrete impl method should be an instance");
    assert!(record.substitution.is_empty());
    assert_eq!(record.symbols.source_name, "Box::println");
    assert!(record
        .symbols
        .backend_symbol
        .starts_with("__rock_impl_d40_h"));
    assert!(record.symbols.backend_symbol.ends_with("_none"));
    assert!(output.pre_mir_instance_bodies.contains(record.id));
    assert!(!record.provided_by_object);
}

#[test]
fn process_with_crates_does_not_eagerly_emit_dependency_generic_impl_methods() {
    let impl_id = DefId::new(CrateId(1), LocalDefId(10));
    let method_id = DefId::new(CrateId(1), LocalDefId(11));
    let mut method = concrete_function(method_id, "capacity", Type::I32);
    method.is_method = true;
    let generic_impl = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Vec".to_string()),
        type_name: "Vec".to_string(),
        type_generics: vec![GenericParamDecl::type_param(
            crate::types::GenericParamId {
                owner: impl_id,
                index: 0,
            },
            "T",
        )],
        receiver_pattern: vec![Type::Generic(crate::types::GenericParamId {
            owner: impl_id,
            index: 0,
        })]
        .into(),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("capacity".to_string(), method)]),
    };
    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(loaded_object_backed_crate(
        Vec::new(),
        vec![generic_impl.clone()],
    ))
    .unwrap();
    let program =
        crate::mono::hir_types::HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(impl_id, generic_impl)]),
            HashMap::new(),
            crate::hir::HirNameTables::default(),
            &HashMap::from([(impl_id, "Vec".to_string())]),
        );

    let mut mono = Monomorphizer::new();
    let output = wrapped_process_with_crates(&mut mono, program, &ctx);

    assert!(
        output.instances.is_empty(),
        "dependency generic impl methods should specialize only when called: {:?}",
        output.instances
    );
}

#[test]
fn process_with_crates_does_not_emit_unused_dependency_drop_glue_for_interned_types() {
    let drop_trait_id = DefId::new(CrateId(1), LocalDefId(1));
    let impl_id = DefId::new(CrateId(1), LocalDefId(10));
    let method_id = DefId::new(CrateId(1), LocalDefId(11));
    let box_id = DefId::new(CrateId(1), LocalDefId(20));
    let generic = crate::types::GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let box_generic = Type::Struct {
        id: box_id,
        args: vec![Type::Generic(generic)],
    };
    let drop_method = HirFunction {
        id: method_id,
        name: "drop".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: box_generic.clone(),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::Unit,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::Unit,
        },
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Move),
        is_unsafe: false,
    };
    let drop_impl = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: vec![GenericParamDecl::type_param(generic, "T")],
        receiver_pattern: vec![Type::Generic(generic)].into(),
        trait_name: Some("Drop".to_string()),
        trait_id: Some(drop_trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("drop".to_string(), drop_method)]),
    };
    let mut resolver = ResolverTables::default();
    resolver
        .item_names_by_id
        .insert(box_id, "std::Box".to_string());
    let loaded = ExternCrateRecord::new(
        CrateId(1),
        "std".to_string(),
        ExternCrateMetadata::new(ArtifactCrateInterface::default(), resolver, BTreeMap::new())
            .with_language_items(crate::hir::HirLanguageItems {
                drop: Some(crate::language_items::DropLanguageItems {
                    trait_id: drop_trait_id,
                    method_id,
                }),
                ..crate::hir::HirLanguageItems::default()
            }),
        ExternCrateBodies::from_cross_crate_hir(ArtifactCrossCrateHir {
            generic_functions: BTreeMap::new(),
            traits_with_defaults: BTreeMap::new(),
            generic_impls: vec![drop_impl],
        }),
        ExternCrateLink::object(PathBuf::from("std.o"), BTreeMap::new()),
    );
    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(loaded).unwrap();
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
    mono.type_context.intern_type(&Type::Struct {
        id: box_id,
        args: vec![Type::I32],
    });
    let output = wrapped_process_with_crates(&mut mono, program, &ctx);

    assert!(
        output.instances.is_empty(),
        "unused dependency Drop impls should not be emitted from the global type table: {:?}",
        output.instances
    );
}

#[test]
fn process_with_crates_records_concrete_trait_default_instance() {
    let trait_id = DefId::new(CrateId(0), LocalDefId(50));
    let method_id = DefId::new(CrateId(0), LocalDefId(51));
    let default_method = concrete_function(method_id, "default_value", Type::I32);
    let trait_def = crate::hir::HirTraitFor::<crate::hir::AcceptedHir> {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Provider".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::from([("default_value".to_string(), default_method)]),
        signatures: HashMap::new(),
    };
    let program =
        crate::mono::hir_types::HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(trait_id, trait_def)]),
            HashMap::new(),
            HashMap::new(),
            crate::hir::HirNameTables {
                traits_by_name: HashMap::from([("Provider".to_string(), trait_id)]),
                ..crate::hir::HirNameTables::default()
            },
            &HashMap::from([(trait_id, "Provider".to_string())]),
        );
    let ctx = CrateContext::new();
    let mut mono = Monomorphizer::new();

    let output = wrapped_process_with_crates(&mut mono, program, &ctx);

    let record = output
        .instances
        .values()
        .find(|record| {
            record.origin
                == crate::mono::InstanceOrigin::TraitDefault {
                    trait_id,
                    method: method_id,
                }
        })
        .expect("concrete trait default should be an instance");
    assert!(record.substitution.is_empty());
    assert_eq!(record.symbols.source_name, "Provider::default_value");
    assert!(record
        .symbols
        .backend_symbol
        .starts_with("__rock_trait_default_d50_h"));
    assert!(record.symbols.backend_symbol.ends_with("_none"));
    assert!(output.pre_mir_instance_bodies.contains(record.id));
    assert!(!record.provided_by_object);
}

#[test]
fn process_with_crates_processes_concrete_trait_default_body_before_registration() {
    let generic_id = DefId::new(CrateId(0), LocalDefId(60));
    let generic_param_id = crate::types::GenericParamId {
        owner: generic_id,
        index: 0,
    };
    let generic_type = Type::Generic(generic_param_id);
    let generic_function = HirFunction {
        id: generic_id,
        name: "identity".to_string(),
        generic_params: vec![GenericParamDecl::type_param(generic_param_id, "T")],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "value".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: generic_type.clone(),
            mutable: false,
            is_ref: false,
        }],
        ret_type: generic_type.clone(),
        body: HirBlock {
            stmts: vec![HirStmt::Return(Some(HirExpr {
                kind: HirExprKind::Var("value".to_string()),
                ty: generic_type.clone(),
                span: Span::test(),
            }))],
            ty: generic_type,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };

    let trait_id = DefId::new(CrateId(0), LocalDefId(61));
    let method_id = DefId::new(CrateId(0), LocalDefId(62));
    let mut default_method = concrete_function(method_id, "default_value", Type::I32);
    default_method.body = HirBlock {
        stmts: vec![HirStmt::Return(Some(HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: "identity".to_string(),
                        target: HirVarTarget::Function(generic_id),
                    }),
                    ty: Type::function(vec![Type::I32], Type::I32),
                    span: Span::test(),
                }),
                vec![HirExpr {
                    kind: HirExprKind::IntLiteral(7),
                    ty: Type::I32,
                    span: Span::test(),
                }],
                Some(HirCallTarget::Function(generic_id)),
            ),
            ty: Type::I32,
            span: Span::test(),
        }))],
        ty: Type::I32,
    };
    let trait_def = crate::hir::HirTraitFor::<crate::hir::AcceptedHir> {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Provider".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::from([("default_value".to_string(), default_method)]),
        signatures: HashMap::new(),
    };
    let program =
        crate::mono::hir_types::HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            HashMap::from([(generic_id, generic_function)]),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(trait_id, trait_def)]),
            HashMap::new(),
            HashMap::new(),
            crate::hir::HirNameTables {
                functions_by_name: HashMap::from([("identity".to_string(), generic_id)]),
                traits_by_name: HashMap::from([("Provider".to_string(), trait_id)]),
                ..crate::hir::HirNameTables::default()
            },
            &HashMap::from([
                (generic_id, "identity".to_string()),
                (trait_id, "Provider".to_string()),
            ]),
        );
    let ctx = CrateContext::new();
    let mut mono = Monomorphizer::new();
    mono.resolver
        .item_paths
        .insert("identity".to_string(), generic_id);

    let output = wrapped_process_with_crates(&mut mono, program, &ctx);

    let trait_record = output
        .instances
        .values()
        .find(|record| {
            record.origin
                == crate::mono::InstanceOrigin::TraitDefault {
                    trait_id,
                    method: method_id,
                }
        })
        .expect("concrete trait default should be an instance");
    let trait_body = output
        .pre_mir_instance_bodies
        .get(trait_record.id)
        .expect("trait default has body");
    let HirStmt::Return(Some(returned)) = &trait_body.body.stmts[0] else {
        panic!("trait default should return the generic call");
    };
    let HirExprKind::Call(callee, _, _) = &returned.kind else {
        panic!("trait default return should remain a call");
    };
    let HirExprKind::ResolvedVar(reference) = &callee.kind else {
        panic!("trait default call callee should be a resolved instance target");
    };
    let instance_id = match reference.target {
        HirVarTarget::Instance(instance_id) => instance_id,
        HirVarTarget::Function(id) if id == generic_id => output
            .instances
            .values()
            .find(|record| record.origin == crate::mono::InstanceOrigin::Function(generic_id))
            .map(|record| record.id)
            .expect("generic call specialization should be registered"),
        ref target => panic!("trait default call callee has unexpected target {target:?}"),
    };
    assert_eq!(reference.name, "identity");

    let specialization = output
        .instances
        .get(&instance_id)
        .expect("trait default should register the generic specialization it calls");
    assert_eq!(
        specialization.origin,
        crate::mono::InstanceOrigin::Function(generic_id)
    );
    assert_eq!(specialization.substitution.len(), 1);
    assert_eq!(
        output.type_context.type_for(specialization.substitution[0]),
        Type::I32
    );
    assert_eq!(
        specialization.symbols.backend_symbol,
        mono.backend_symbol_for_origin(&specialization.origin, &specialization.substitution)
    );
}

use super::*;

#[test]
fn process_with_crates_records_object_backed_instances_without_re_emitting() {
    let byte_slice_ty = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Slice(Box::new(Type::U8))),
    };
    let impl_id = DefId::new(CrateId(0), LocalDefId(0));
    let method_id = DefId::new(CrateId(0), LocalDefId(1));
    let mut method = println_method(byte_slice_ty);
    method.id = method_id;

    let concrete_impl = HirImpl {
        id: impl_id,
        owner: HirImplOwner::BuiltinSlice,
        type_name: "&[U8]".to_string(),
        type_generics: vec![],
        receiver_pattern: vec![Type::U8].into(),
        trait_name: Some("Show".to_string()),
        trait_id: Some(DefId::new(CrateId(0), LocalDefId(99))),
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("println".to_string(), method)]),
    };

    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(loaded_object_backed_crate_with_backend_symbols(
        vec![concrete_impl],
        vec![],
        BTreeMap::from([(method_id, "&[U8]_println".to_string())]),
    ))
    .unwrap();

    let mut mono = Monomorphizer::new();
    let output = wrapped_process_with_crates(
        &mut mono,
        crate::mono::hir_types::HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            crate::hir::HirNameTables::default(),
            &HashMap::new(),
        ),
        &ctx,
    );

    assert!(output
        .instances
        .values()
        .any(|record| record.provided_by_object));
    let record = output
        .instances
        .values()
        .find(|record| record.provided_by_object)
        .expect("object-backed method instance should be recorded");
    assert_eq!(
        record.origin,
        crate::mono::InstanceOrigin::ImplMethod {
            owner: crate::mono::InstanceImplOwner::BuiltinSlice,
            method: method_id,
        }
    );
    assert_eq!(record.symbols.backend_symbol, "&[U8]_println");
    assert!(!output
        .program
        .names
        .functions_by_name
        .keys()
        .any(|name| name.starts_with("std::")));
}

#[test]
fn process_with_crates_records_concrete_method_from_mixed_object_backed_impl() {
    let trait_id = DefId::new(CrateId(0), LocalDefId(99));
    let impl_id = DefId::new(CrateId(0), LocalDefId(20));
    let concrete_method_id = DefId::new(CrateId(0), LocalDefId(21));
    let generic_method_id = DefId::new(CrateId(0), LocalDefId(22));
    let generic_param = crate::types::GenericParamId {
        owner: generic_method_id,
        index: 0,
    };

    let mut concrete_method = println_method(Type::I64);
    concrete_method.id = concrete_method_id;
    let generic_method = HirFunction {
        id: generic_method_id,
        name: "identity".to_string(),
        generic_params: vec![GenericParamDecl::type_param(generic_param, "T")],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "value".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Generic(generic_param),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::Generic(generic_param),
        body: HirBlock {
            stmts: vec![HirStmt::Return(Some(HirExpr {
                kind: HirExprKind::Var("value".to_string()),
                ty: Type::Generic(generic_param),
                span: Span::test(),
            }))],
            ty: Type::Generic(generic_param),
        },
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Move),
        is_unsafe: false,
    };
    let mixed_impl = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: Vec::new().into(),
        trait_name: Some("Show".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([
            ("println".to_string(), concrete_method),
            ("identity".to_string(), generic_method),
        ]),
    };

    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(loaded_object_backed_crate_with_backend_symbols(
        vec![mixed_impl],
        Vec::new(),
        BTreeMap::from([(concrete_method_id, "artifact_box_println".to_string())]),
    ))
    .unwrap();

    let mut mono = Monomorphizer::new();
    let output = wrapped_process_with_crates(
        &mut mono,
        crate::mono::hir_types::HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            crate::hir::HirNameTables::default(),
            &HashMap::new(),
        ),
        &ctx,
    );

    let object_records = output
        .instances
        .values()
        .filter(|record| record.provided_by_object)
        .collect::<Vec<_>>();
    assert_eq!(object_records.len(), 1);
    let concrete_record = object_records[0];
    assert_eq!(
        concrete_record.origin,
        crate::mono::InstanceOrigin::ImplMethod {
            owner: crate::mono::InstanceImplOwner::Named(impl_id),
            method: concrete_method_id,
        }
    );
    assert_eq!(
        concrete_record.symbols.backend_symbol,
        "artifact_box_println"
    );
    assert!(!output.instances.values().any(|record| {
        record.origin
            == crate::mono::InstanceOrigin::ImplMethod {
                owner: crate::mono::InstanceImplOwner::Named(impl_id),
                method: generic_method_id,
            }
    }));
}

#[test]
fn record_imported_impl_allocates_object_method_instances_by_method_def_id() {
    fn imported_origins(
        method_ids: &[DefId],
    ) -> Vec<(crate::mono::InstanceId, crate::mono::InstanceOrigin)> {
        let impl_id = DefId::new(CrateId(1), LocalDefId(30));
        let trait_id = DefId::new(CrateId(1), LocalDefId(31));
        let methods = method_ids
            .iter()
            .map(|method_id| {
                let mut method = println_method(Type::I64);
                method.id = *method_id;
                (format!("method_{}", method_id.local.0), method)
            })
            .collect::<HashMap<_, _>>();
        let imp = HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: Vec::new().into(),
            trait_name: Some("Show".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods,
        };
        let backend_symbols = method_ids
            .iter()
            .map(|method_id| (*method_id, format!("artifact_{}", method_id.local.0)))
            .collect();
        let mut ctx = CrateContext::new();
        ctx.add_extern_crate(loaded_object_backed_crate_with_backend_symbols(
            vec![imp.clone()],
            Vec::new(),
            backend_symbols,
        ))
        .expect("object-backed dependency should load");
        let dep = ctx
            .extern_crates()
            .next()
            .expect("dependency should be available");
        let mut mono = Monomorphizer::new();

        mono.record_imported_impl("std", &imp, Some(dep));

        mono.instances
            .records()
            .map(|record| (record.id, record.origin.clone()))
            .collect()
    }

    let first_method_id = DefId::new(CrateId(1), LocalDefId(32));
    let second_method_id = DefId::new(CrateId(1), LocalDefId(33));
    let impl_id = DefId::new(CrateId(1), LocalDefId(30));
    let expected = vec![
        (
            crate::mono::InstanceId(0),
            crate::mono::InstanceOrigin::ImplMethod {
                owner: crate::mono::InstanceImplOwner::Named(impl_id),
                method: first_method_id,
            },
        ),
        (
            crate::mono::InstanceId(1),
            crate::mono::InstanceOrigin::ImplMethod {
                owner: crate::mono::InstanceImplOwner::Named(impl_id),
                method: second_method_id,
            },
        ),
    ];

    for method_order in [
        [first_method_id, second_method_id],
        [second_method_id, first_method_id],
    ] {
        for _ in 0..16 {
            assert_eq!(imported_origins(&method_order), expected);
        }
    }
}

#[test]
fn process_with_crates_uses_artifact_backend_symbols_for_object_backed_methods() {
    let impl_id = DefId::new(CrateId(0), LocalDefId(10));
    let method_id = DefId::new(CrateId(0), LocalDefId(11));
    let mut method = println_method(Type::I64);
    method.id = method_id;

    let concrete_impl = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: vec![],
        receiver_pattern: vec![].into(),
        trait_name: Some("Show".to_string()),
        trait_id: Some(DefId::new(CrateId(0), LocalDefId(99))),
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("println".to_string(), method)]),
    };

    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(loaded_object_backed_crate_with_backend_symbols(
        vec![concrete_impl],
        vec![],
        BTreeMap::from([(method_id, "artifact_box_println".to_string())]),
    ))
    .unwrap();

    let mut mono = Monomorphizer::new();
    let output = wrapped_process_with_crates(
        &mut mono,
        crate::mono::hir_types::HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            crate::hir::HirNameTables::default(),
            &HashMap::new(),
        ),
        &ctx,
    );

    let record = output
        .instances
        .values()
        .find(|record| record.provided_by_object)
        .expect("object-backed method instance should be recorded");
    assert_eq!(record.symbols.source_name, "std::println");
    assert_eq!(record.symbols.backend_symbol, "artifact_box_println");
}

#[test]
fn object_backed_impls_with_same_display_names_use_distinct_ids() {
    fn object_backed_box_impl(
        crate_id: CrateId,
        impl_local: u32,
        method_local: u32,
        trait_local: u32,
    ) -> HirImpl {
        let impl_id = DefId::new(crate_id, LocalDefId(impl_local));
        let method_id = DefId::new(crate_id, LocalDefId(method_local));
        let trait_id = DefId::new(crate_id, LocalDefId(trait_local));
        let mut method = println_method(Type::I64);
        method.id = method_id;

        HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Show".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("println".to_string(), method)]),
        }
    }

    let first_impl = object_backed_box_impl(CrateId(1), 10, 11, 12);
    let second_impl = object_backed_box_impl(CrateId(2), 10, 11, 12);
    let first_method_id = first_impl.methods["println"].id;
    let second_method_id = second_impl.methods["println"].id;
    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(loaded_object_backed_crate_named_with_backend_symbols(
        CrateId(1),
        "first_dep",
        vec![first_impl.clone()],
        vec![],
        BTreeMap::from([(first_method_id, "first_box_println".to_string())]),
    ))
    .unwrap();
    ctx.add_extern_crate(loaded_object_backed_crate_named_with_backend_symbols(
        CrateId(2),
        "second_dep",
        vec![second_impl.clone()],
        vec![],
        BTreeMap::from([(second_method_id, "second_box_println".to_string())]),
    ))
    .unwrap();

    let mut mono = Monomorphizer::new();
    let output = wrapped_process_with_crates(
        &mut mono,
        crate::mono::hir_types::HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            crate::hir::HirNameTables::default(),
            &HashMap::new(),
        ),
        &ctx,
    );

    let object_records = output
        .instances
        .values()
        .filter(|record| record.provided_by_object)
        .collect::<Vec<_>>();
    assert_eq!(object_records.len(), 2);
    assert!(object_records.iter().any(|record| {
        record.origin
            == crate::mono::InstanceOrigin::ImplMethod {
                owner: crate::mono::InstanceImplOwner::Named(first_impl.id),
                method: first_method_id,
            }
            && record.symbols.backend_symbol == "first_box_println"
    }));
    assert!(object_records.iter().any(|record| {
        record.origin
            == crate::mono::InstanceOrigin::ImplMethod {
                owner: crate::mono::InstanceImplOwner::Named(second_impl.id),
                method: second_method_id,
            }
            && record.symbols.backend_symbol == "second_box_println"
    }));
}

#[test]
fn generic_artifact_method_specialization_records_method_def_id_origin() {
    let impl_id = DefId::new(CrateId(0), LocalDefId(10));
    let method_id = DefId::new(CrateId(0), LocalDefId(11));
    let trait_id = DefId::new(CrateId(0), LocalDefId(99));
    let mut method = println_method(Type::Struct {
        id: DefId::new(CrateId(0), LocalDefId(10)),
        args: vec![Type::Generic(crate::types::GenericParamId {
            owner: impl_id,
            index: 0,
        })],
    });
    method.id = method_id;
    let generic_trait_impl = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
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
        trait_name: Some("Show".to_string()),
        trait_id: Some(trait_id),
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("println".to_string(), method)]),
    };

    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(loaded_object_backed_crate(vec![], vec![generic_trait_impl]))
        .unwrap();

    let recv = HirExpr {
        kind: HirExprKind::Var("boxed".to_string()),
        ty: Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(10)),
            args: vec![Type::I64],
        },
        span: Span::test(),
    };
    let mut selected_target = HirMethodCallTarget::impl_method(
        impl_id,
        method_id,
        Some(crate::hir::HirSelectedTraitMember {
            trait_id,
            member_id: method_id,
            trait_args: Vec::new(),
        }),
    );
    selected_target.owner_substitution = vec![crate::hir::HirTypeBinding {
        param: crate::types::GenericParamId {
            owner: impl_id,
            index: 0,
        },
        ty: Type::I64,
    }];
    let mut expr = HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(recv.clone()),
            "println".to_string(),
            vec![],
            Some(ReceiverMode::Move),
            selected_target,
        ),
        ty: Type::I32,
        span: Span::test(),
    };
    let mut mono = Monomorphizer::new();
    mono.resolver
        .item_names_by_id
        .insert(impl_id, "Box".to_string());
    mono.load_external_generic_functions(&ctx);
    mono.set_impl_receiver_pattern_for_test(
        impl_id,
        crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(10)),
            args: vec![Type::Generic(crate::types::GenericParamId {
                owner: impl_id,
                index: 0,
            })],
        }),
    );

    let _ = mono.monomorphize_trait_method_call("println", &[recv], &mut expr);

    let origins = mono
        .instances
        .records()
        .map(|record| record.origin.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        origins,
        vec![crate::mono::InstanceOrigin::ImplMethod {
            owner: crate::mono::InstanceImplOwner::Named(impl_id),
            method: method_id,
        }]
    );
}

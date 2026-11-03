use super::*;

fn function_with_id(id: DefId, receiver: Option<ReceiverMode>) -> HirFunction {
    let mut function = empty_function(receiver);
    function.id = id;
    function
}

#[test]
fn test_lookup_self_receiver_uses_targeted_trait_impl() {
    let mut mono = Monomorphizer::new();
    let trait_id = DefId::new(CrateId(0), LocalDefId(11));
    let method_id = DefId::new(CrateId(0), LocalDefId(12));

    let first_impl = HirImpl {
        id: DefId::new(CrateId(0), LocalDefId(0)),
        owner: HirImplOwner::Named("Foo".to_string()),
        trait_name: Some("TraitA".to_string()),
        trait_id: Some(DefId::new(CrateId(0), LocalDefId(10))),
        type_name: "Foo".to_string(),
        type_generics: vec![],
        receiver_pattern: vec![].into(),
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("other".to_string(), empty_function(None))]),
    };
    let second_impl = HirImpl {
        id: DefId::new(CrateId(0), LocalDefId(0)),
        owner: HirImplOwner::Named("Foo".to_string()),
        trait_name: Some("TraitB".to_string()),
        trait_id: Some(trait_id),
        type_name: "Foo".to_string(),
        type_generics: vec![],
        receiver_pattern: vec![].into(),
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([(
            "target".to_string(),
            function_with_id(method_id, Some(ReceiverMode::Mut)),
        )]),
    };

    mono.trait_impls
        .insert(DefId::new(CrateId(0), LocalDefId(10)), vec![first_impl]);
    mono.trait_impls.insert(trait_id, vec![second_impl]);

    assert_eq!(
        mono.lookup_self_receiver(Some(&HirMethodCallTarget::trait_method(
            trait_id,
            method_id,
            Vec::new(),
            crate::hir::HirTraitDispatchKind::TraitBound,
        ))),
        Some(ReceiverMode::Mut)
    );
}

#[test]
fn test_lookup_self_receiver_does_not_pick_first_same_named_trait_method() {
    let mut mono = Monomorphizer::new();
    let first_trait_id = DefId::new(CrateId(0), LocalDefId(10));
    let second_trait_id = DefId::new(CrateId(0), LocalDefId(11));
    let first_method_id = DefId::new(CrateId(0), LocalDefId(30));
    let second_method_id = DefId::new(CrateId(0), LocalDefId(31));

    let first_impl = HirImpl {
        id: DefId::new(CrateId(0), LocalDefId(20)),
        owner: HirImplOwner::Named("Foo".to_string()),
        trait_name: Some("TraitA".to_string()),
        trait_id: Some(first_trait_id),
        type_name: "Foo".to_string(),
        type_generics: vec![],
        receiver_pattern: vec![].into(),
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([(
            "target".to_string(),
            function_with_id(first_method_id, Some(ReceiverMode::Move)),
        )]),
    };
    let second_impl = HirImpl {
        id: DefId::new(CrateId(0), LocalDefId(21)),
        owner: HirImplOwner::Named("Foo".to_string()),
        trait_name: Some("TraitB".to_string()),
        trait_id: Some(second_trait_id),
        type_name: "Foo".to_string(),
        type_generics: vec![],
        receiver_pattern: vec![].into(),
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([(
            "target".to_string(),
            function_with_id(second_method_id, Some(ReceiverMode::Mut)),
        )]),
    };

    mono.trait_impls.insert(first_trait_id, vec![first_impl]);
    mono.trait_impls.insert(second_trait_id, vec![second_impl]);

    assert_eq!(
        mono.lookup_self_receiver(Some(&HirMethodCallTarget::trait_method(
            second_trait_id,
            second_method_id,
            Vec::new(),
            crate::hir::HirTraitDispatchKind::TraitBound,
        ))),
        Some(ReceiverMode::Mut)
    );
}

fn generic_self_method(_type_name: &str, impl_id: DefId, method_id: DefId) -> HirFunction {
    let generic = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    HirFunction {
        id: method_id,
        name: "map".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Struct {
                id: DefId::new(CrateId(0), LocalDefId(700)),
                args: vec![Type::Generic(generic)],
            },
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::I64,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::I64,
        },
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Move),
        is_unsafe: false,
    }
}

fn generic_impl_for_process_test(
    type_name: &str,
    impl_id: DefId,
    trait_id: Option<DefId>,
    method_id: DefId,
) -> HirImpl {
    HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named(type_name.to_string()),
        type_name: type_name.to_string(),
        type_generics: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: impl_id,
                index: 0,
            },
            "T",
        )],
        receiver_pattern: vec![Type::Generic(GenericParamId {
            owner: impl_id,
            index: 0,
        })]
        .into(),
        trait_name: trait_id.map(|_| "MapTrait".to_string()),
        trait_id,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([(
            "map".to_string(),
            generic_self_method(type_name, impl_id, method_id),
        )]),
    }
}

fn selected_process_method_target(impl_id: DefId, method_id: DefId) -> HirMethodCallTarget {
    let mut target = HirMethodCallTarget::impl_method(impl_id, method_id, None);
    target.owner_substitution = vec![crate::hir::HirTypeBinding {
        param: GenericParamId {
            owner: impl_id,
            index: 0,
        },
        ty: Type::I64,
    }];
    target
}

fn seed_box_receiver_pattern(mono: &mut Monomorphizer, impl_id: DefId) {
    mono.set_impl_receiver_pattern_for_test(
        impl_id,
        crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(700)),
            args: vec![Type::Generic(GenericParamId {
                owner: impl_id,
                index: 0,
            })],
        }),
    );
}

#[test]
fn process_selected_inherent_method_skips_trait_specialization_path() {
    let mut mono = Monomorphizer::new();
    let impl_id = DefId::new(CrateId(0), LocalDefId(701));
    let inherent_method_id = DefId::new(CrateId(0), LocalDefId(702));
    let trait_method_id = DefId::new(CrateId(0), LocalDefId(703));
    let trait_id = DefId::new(CrateId(0), LocalDefId(704));
    mono.resolver
        .item_names_by_id
        .insert(DefId::new(CrateId(0), LocalDefId(700)), "Box".to_string());
    mono.generic_impls.insert(
        impl_id,
        generic_impl_for_process_test("Box", impl_id, None, inherent_method_id),
    );
    mono.trait_impls.insert(
        trait_id,
        vec![generic_impl_for_process_test(
            "Box",
            impl_id,
            Some(trait_id),
            trait_method_id,
        )],
    );
    seed_box_receiver_pattern(&mut mono, impl_id);
    let recv = HirExpr {
        kind: HirExprKind::Var("box".to_string()),
        ty: Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(700)),
            args: vec![Type::I64],
        },
        span: crate::lexer::Span::test(),
    };
    let mut expr = HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(recv),
            "map".to_string(),
            Vec::new(),
            Some(ReceiverMode::Move),
            selected_process_method_target(impl_id, inherent_method_id),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let origins = mono
        .instances
        .records()
        .map(|record| record.origin.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        origins,
        vec![InstanceOrigin::ImplMethod {
            owner: InstanceImplOwner::Named(impl_id),
            method: inherent_method_id,
        }]
    );
}

#[test]
fn process_selected_generic_method_call_uses_instance_target() {
    let mut mono = Monomorphizer::new();
    let impl_id = DefId::new(CrateId(0), LocalDefId(921));
    let method_id = DefId::new(CrateId(0), LocalDefId(922));
    mono.resolver
        .item_names_by_id
        .insert(DefId::new(CrateId(0), LocalDefId(700)), "Box".to_string());
    mono.generic_impls.insert(
        impl_id,
        generic_impl_for_process_test("Box", impl_id, None, method_id),
    );
    seed_box_receiver_pattern(&mut mono, impl_id);
    let recv = HirExpr {
        kind: HirExprKind::Var("box".to_string()),
        ty: Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(700)),
            args: vec![Type::I64],
        },
        span: crate::lexer::Span::test(),
    };
    let mut expr = HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(recv),
            "map".to_string(),
            Vec::new(),
            Some(ReceiverMode::Move),
            selected_process_method_target(impl_id, method_id),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(callee, _, _) = &expr.kind else {
        panic!("method call should lower to a direct call");
    };
    let HirExprKind::ResolvedVar(reference) = &callee.kind else {
        panic!("method callee should be an instance-backed resolved var");
    };
    let HirVarTarget::Instance(instance_id) = reference.target else {
        panic!("method callee should target an instance");
    };
    let record = mono
        .instances
        .record(instance_id)
        .expect("instance is recorded");
    assert_eq!(
        record.origin,
        InstanceOrigin::ImplMethod {
            owner: InstanceImplOwner::Named(impl_id),
            method: method_id,
        }
    );
}

#[test]
fn process_targetless_generic_method_call_does_not_specialize_by_name() {
    let mut mono = Monomorphizer::new();
    let source_span = crate::lexer::Span {
        file_path: "/test.rk".into(),
        start: 0,
        end: 3,
    };
    let impl_id = DefId::new(CrateId(0), LocalDefId(941));
    let method_id = DefId::new(CrateId(0), LocalDefId(942));
    mono.resolver
        .item_names_by_id
        .insert(DefId::new(CrateId(0), LocalDefId(700)), "Box".to_string());
    mono.generic_impls.insert(
        impl_id,
        generic_impl_for_process_test("Box", impl_id, None, method_id),
    );
    seed_box_receiver_pattern(&mut mono, impl_id);
    let recv = HirExpr {
        kind: HirExprKind::Var("box".to_string()),
        ty: Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(700)),
            args: vec![Type::I64],
        },
        span: source_span.clone(),
    };
    let mut expr = HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(recv),
            "map".to_string(),
            Vec::new(),
            Some(ReceiverMode::Move),
            HirMethodCallTarget::trait_method(
                DefId::new(CrateId(0), LocalDefId(999)),
                DefId::new(CrateId(0), LocalDefId(998)),
                Vec::new(),
                crate::hir::HirTraitDispatchKind::TraitBound,
            ),
        ),
        ty: Type::I64,
        span: source_span,
    };

    mono.process_expr(&mut expr);

    assert!(
        matches!(expr.kind, HirExprKind::MethodCall(..)),
        "targetless semantic method dispatch must not be rediscovered by name"
    );
    assert!(mono.instances.records().next().is_none());
}

#[test]
fn process_reused_generic_method_call_keeps_instance_target() {
    let mut mono = Monomorphizer::new();
    let impl_id = DefId::new(CrateId(0), LocalDefId(931));
    let method_id = DefId::new(CrateId(0), LocalDefId(932));
    mono.resolver
        .item_names_by_id
        .insert(DefId::new(CrateId(0), LocalDefId(700)), "Box".to_string());
    mono.generic_impls.insert(
        impl_id,
        generic_impl_for_process_test("Box", impl_id, None, method_id),
    );
    seed_box_receiver_pattern(&mut mono, impl_id);

    let mut first = HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(HirExpr {
                kind: HirExprKind::Var("first".to_string()),
                ty: Type::Struct {
                    id: DefId::new(CrateId(0), LocalDefId(700)),
                    args: vec![Type::I64],
                },
                span: crate::lexer::Span::test(),
            }),
            "map".to_string(),
            Vec::new(),
            Some(ReceiverMode::Move),
            selected_process_method_target(impl_id, method_id),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };
    let mut second = first.clone();

    mono.process_expr(&mut first);
    mono.process_expr(&mut second);

    let first_id = match &first.kind {
        HirExprKind::Call(callee, _, _) => match &callee.kind {
            HirExprKind::ResolvedVar(reference) => match reference.target {
                HirVarTarget::Instance(id) => id,
                _ => panic!("first call should target an instance"),
            },
            _ => panic!("first call callee should be resolved"),
        },
        _ => panic!("first method call should lower to a direct call"),
    };
    let second_id = match &second.kind {
        HirExprKind::Call(callee, _, _) => match &callee.kind {
            HirExprKind::ResolvedVar(reference) => match reference.target {
                HirVarTarget::Instance(id) => id,
                _ => panic!("second call should target an instance"),
            },
            _ => panic!("second call callee should be resolved"),
        },
        _ => panic!("second method call should lower to a direct call"),
    };

    assert_eq!(first_id, second_id);
}

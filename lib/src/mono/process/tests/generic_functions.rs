use super::*;

#[test]
fn process_targetless_generic_function_call_does_not_specialize_by_name() {
    let mut mono = Monomorphizer::new();
    let id = DefId::new(CrateId(0), LocalDefId(910));
    let generic = HirFunction {
        id,
        name: "identity".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: id,
                index: 0,
            },
            "T",
        )],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "value".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Generic(GenericParamId {
                owner: id,
                index: 0,
            }),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::Generic(GenericParamId {
            owner: id,
            index: 0,
        }),
        body: HirBlock {
            stmts: vec![HirStmt::Return(Some(HirExpr {
                kind: HirExprKind::Var("value".to_string()),
                ty: Type::Generic(GenericParamId {
                    owner: id,
                    index: 0,
                }),
                span: crate::lexer::Span::test(),
            }))],
            ty: Type::Generic(GenericParamId {
                owner: id,
                index: 0,
            }),
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    mono.register_generic_function("identity".to_string(), generic);
    mono.resolver.item_paths.insert("identity".to_string(), id);
    let mut expr = HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::Var("identity".to_string()),
                ty: Type::function(vec![Type::I64], Type::I64),
                span: crate::lexer::Span::test(),
            }),
            vec![HirExpr {
                kind: HirExprKind::IntLiteral(21),
                ty: Type::I64,
                span: crate::lexer::Span::test(),
            }],
            None,
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(callee, _, target) = &expr.kind else {
        panic!("expression should remain a call");
    };
    assert!(matches!(&callee.kind, HirExprKind::Var(name) if name == "identity"));
    assert_eq!(target, &None);
    assert!(mono.instances.records().next().is_none());
}

#[test]
fn process_targetless_generic_function_value_does_not_specialize_by_name() {
    let mut mono = Monomorphizer::new();
    let id = DefId::new(CrateId(0), LocalDefId(911));
    let generic = generic_identity_for_process_test(id, "identity");
    mono.register_generic_function("identity".to_string(), generic);
    mono.resolver.item_paths.insert("identity".to_string(), id);

    let mut expr = HirExpr {
        kind: HirExprKind::Var("identity".to_string()),
        ty: Type::function(vec![Type::I64], Type::I64),
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    assert!(matches!(expr.kind, HirExprKind::Var(ref name) if name == "identity"));
    assert!(mono.instances.records().next().is_none());
}

#[test]
fn process_resolved_generic_function_value_uses_def_id_not_reference_name() {
    let mut mono = Monomorphizer::new();
    let identity_id = DefId::new(CrateId(0), LocalDefId(970));
    let wrong_id = DefId::new(CrateId(0), LocalDefId(971));
    let identity = generic_identity_for_process_test(identity_id, "identity");
    let wrong = generic_identity_for_process_test(wrong_id, "wrong_alias");

    mono.register_generic_function("identity".to_string(), identity.clone());
    mono.register_generic_function("wrong_alias".to_string(), wrong.clone());
    mono.resolver
        .item_paths
        .insert("identity".to_string(), identity_id);
    mono.resolver
        .item_paths
        .insert("wrong_alias".to_string(), wrong_id);

    let mut expr = HirExpr {
        kind: HirExprKind::ResolvedVar(HirVarRef {
            name: "wrong_alias".to_string(),
            target: HirVarTarget::Function(identity_id),
        }),
        ty: Type::function(vec![Type::I64], Type::I64),
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::ResolvedVar(reference) = &expr.kind else {
        panic!("function value should remain a resolved var");
    };
    let HirVarTarget::Instance(instance_id) = reference.target else {
        panic!("resolved generic function value should target an instance");
    };
    let record = mono
        .instances
        .record(instance_id)
        .expect("instance is recorded");
    assert_eq!(record.origin, InstanceOrigin::Function(identity_id));
    assert_ne!(record.origin, InstanceOrigin::Function(wrong_id));
}

#[test]
fn process_resolved_generic_call_uses_def_id_not_reference_name() {
    let mut mono = Monomorphizer::new();
    let identity_id = DefId::new(CrateId(0), LocalDefId(972));
    let wrong_id = DefId::new(CrateId(0), LocalDefId(973));
    let identity = generic_identity_for_process_test(identity_id, "identity");
    let wrong = generic_identity_for_process_test(wrong_id, "wrong_alias");

    mono.register_generic_function("identity".to_string(), identity.clone());
    mono.register_generic_function("wrong_alias".to_string(), wrong.clone());
    mono.resolver
        .item_paths
        .insert("identity".to_string(), identity_id);
    mono.resolver
        .item_paths
        .insert("wrong_alias".to_string(), wrong_id);

    let mut expr = HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "wrong_alias".to_string(),
                    target: HirVarTarget::Function(identity_id),
                }),
                ty: Type::function(vec![Type::I64], Type::I64),
                span: crate::lexer::Span::test(),
            }),
            vec![HirExpr {
                kind: HirExprKind::IntLiteral(42),
                ty: Type::I64,
                span: crate::lexer::Span::test(),
            }],
            Some(HirCallTarget::Function(identity_id)),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(callee, _, target) = &expr.kind else {
        panic!("expression should remain a call");
    };
    let HirExprKind::ResolvedVar(reference) = &callee.kind else {
        panic!("callee should be instance-backed");
    };
    let HirVarTarget::Instance(instance_id) = reference.target else {
        panic!("callee should target a monomorphized instance");
    };
    let record = mono
        .instances
        .record(instance_id)
        .expect("instance is recorded");
    assert_eq!(target, &Some(HirCallTarget::Instance(instance_id)));
    assert_eq!(record.origin, InstanceOrigin::Function(identity_id));
    assert_ne!(record.origin, InstanceOrigin::Function(wrong_id));
}

#[test]
fn process_external_generic_function_value_uses_def_id_not_reference_name() {
    let mut mono = Monomorphizer::new();
    mono.register_test_dependency_identity(CrateId(1));
    let identity_id = DefId::new(CrateId(1), LocalDefId(984));
    let wrong_id = DefId::new(CrateId(1), LocalDefId(985));
    let identity = generic_identity_for_process_test(identity_id, "identity");
    let wrong = generic_identity_for_process_test(wrong_id, "wrong_alias");

    mono.register_external_generic_function("dep::identity".to_string(), identity);
    mono.register_external_generic_function("wrong_alias".to_string(), wrong);
    mono.resolver
        .item_paths
        .insert("dep::identity".to_string(), identity_id);
    mono.resolver
        .item_paths
        .insert("wrong_alias".to_string(), wrong_id);

    let mut expr = HirExpr {
        kind: HirExprKind::ResolvedVar(HirVarRef {
            name: "wrong_alias".to_string(),
            target: HirVarTarget::Extern(identity_id),
        }),
        ty: Type::function(vec![Type::I64], Type::I64),
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::ResolvedVar(reference) = &expr.kind else {
        panic!("external function value should remain a resolved var");
    };
    let HirVarTarget::Instance(instance_id) = reference.target else {
        panic!("external generic function value should target an instance");
    };
    let record = mono
        .instances
        .record(instance_id)
        .expect("instance is recorded");
    assert_eq!(record.origin, InstanceOrigin::Function(identity_id));
    assert_ne!(record.origin, InstanceOrigin::Function(wrong_id));
}

#[test]
fn process_external_generic_call_uses_def_id_not_reference_name() {
    let mut mono = Monomorphizer::new();
    mono.register_test_dependency_identity(CrateId(1));
    let identity_id = DefId::new(CrateId(1), LocalDefId(986));
    let wrong_id = DefId::new(CrateId(1), LocalDefId(987));
    let identity = generic_identity_for_process_test(identity_id, "identity");
    let wrong = generic_identity_for_process_test(wrong_id, "wrong_alias");

    mono.register_external_generic_function("dep::identity".to_string(), identity);
    mono.register_external_generic_function("wrong_alias".to_string(), wrong);
    mono.resolver
        .item_paths
        .insert("dep::identity".to_string(), identity_id);
    mono.resolver
        .item_paths
        .insert("wrong_alias".to_string(), wrong_id);

    let mut expr = HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "wrong_alias".to_string(),
                    target: HirVarTarget::Extern(identity_id),
                }),
                ty: Type::function(vec![Type::I64], Type::I64),
                span: crate::lexer::Span::test(),
            }),
            vec![HirExpr {
                kind: HirExprKind::IntLiteral(42),
                ty: Type::I64,
                span: crate::lexer::Span::test(),
            }],
            Some(HirCallTarget::Function(identity_id)),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(callee, _, target) = &expr.kind else {
        panic!("expression should remain a call");
    };
    let HirExprKind::ResolvedVar(reference) = &callee.kind else {
        panic!("callee should be instance-backed");
    };
    let HirVarTarget::Instance(instance_id) = reference.target else {
        panic!("callee should target a monomorphized instance");
    };
    let record = mono
        .instances
        .record(instance_id)
        .expect("instance is recorded");
    assert_eq!(target, &Some(HirCallTarget::Instance(instance_id)));
    assert_eq!(record.origin, InstanceOrigin::Function(identity_id));
    assert_ne!(record.origin, InstanceOrigin::Function(wrong_id));
}

#[test]
fn generic_function_by_id_requires_def_id_name_metadata() {
    let mut mono = Monomorphizer::new();
    let id = DefId::new(CrateId(0), LocalDefId(977));
    let function = generic_identity_for_process_test(id, "identity");

    mono.generic_functions.insert(id, function);

    assert!(mono.generic_function_by_id(id).is_none());
}

#[test]
fn generic_function_payloads_with_matching_display_names_remain_distinct_by_def_id() {
    let mut mono = Monomorphizer::new();
    let first_id = DefId::new(CrateId(0), LocalDefId(978));
    let second_id = DefId::new(CrateId(1), LocalDefId(978));

    mono.register_generic_function(
        "shared".to_string(),
        generic_identity_for_process_test(first_id, "stale-payload-name"),
    );
    mono.register_generic_function(
        "shared".to_string(),
        generic_identity_for_process_test(second_id, "stale-payload-name"),
    );
    assert_eq!(mono.generic_functions.len(), 2);
    assert_eq!(
        mono.generic_function_by_id(second_id)
            .expect("selected DefId should retain its payload")
            .1
            .id,
        second_id
    );
}

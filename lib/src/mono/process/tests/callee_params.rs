use super::*;

#[test]
fn local_type_tracking_uses_hir_local_id_not_source_name() {
    let mut mono = Monomorphizer::new();
    let mut function = empty_function(None);
    function.body = HirBlock {
        stmts: vec![
            HirStmt::Let {
                name: "value".to_string(),
                local_id: HirLocalId(0),
                ty: Type::I64,
                value: HirExpr {
                    kind: HirExprKind::IntLiteral(1),
                    ty: Type::I64,
                    span: crate::lexer::Span::test(),
                },
                mutable: false,
            },
            HirStmt::Let {
                name: "value".to_string(),
                local_id: HirLocalId(1),
                ty: Type::Bool,
                value: HirExpr {
                    kind: HirExprKind::BoolLiteral(true),
                    ty: Type::Bool,
                    span: crate::lexer::Span::test(),
                },
                mutable: false,
            },
            HirStmt::Expr(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "value".to_string(),
                    target: HirVarTarget::Local(HirLocalId(0)),
                }),
                ty: Type::Unit,
                span: crate::lexer::Span::test(),
            }),
        ],
        ty: Type::Unit,
    };

    let processed = mono.process_function(function);
    let HirStmt::Expr(expr) = &processed.body.stmts[2] else {
        panic!("expected expression statement");
    };

    assert_eq!(expr.ty, Type::I64);
}

#[test]
fn parameter_type_tracking_uses_hir_local_id_not_source_name() {
    let mut mono = Monomorphizer::new();
    let mut function = empty_function(None);
    function.params = vec![HirParam {
        name: "value".to_string(),
        local_id: HirLocalId(0),
        ty: Type::I64,
        mutable: false,
        is_ref: false,
    }];
    function.ret_type = Type::I64;
    function.body = HirBlock {
        stmts: vec![
            HirStmt::Let {
                name: "value".to_string(),
                local_id: HirLocalId(1),
                ty: Type::Bool,
                value: HirExpr {
                    kind: HirExprKind::BoolLiteral(true),
                    ty: Type::Bool,
                    span: crate::lexer::Span::test(),
                },
                mutable: false,
            },
            HirStmt::Expr(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "value".to_string(),
                    target: HirVarTarget::Local(HirLocalId(0)),
                }),
                ty: Type::Bool,
                span: crate::lexer::Span::test(),
            }),
        ],
        ty: Type::I64,
    };

    let processed = mono.process_function(function);
    let HirStmt::Expr(expr) = &processed.body.stmts[1] else {
        panic!("expected expression statement");
    };

    assert_eq!(expr.ty, Type::I64);
}

#[test]
fn process_resolved_callee_param_lookup_uses_def_id_not_reference_name() {
    let mut mono = Monomorphizer::new();
    let apply_id = DefId::new(CrateId(0), LocalDefId(978));
    let identity_id = DefId::new(CrateId(0), LocalDefId(979));
    let wrong_id = DefId::new(CrateId(0), LocalDefId(980));
    let identity = generic_identity_for_process_test(identity_id, "identity");
    let apply = callback_acceptor_for_process_test(
        apply_id,
        "apply",
        Type::function(vec![Type::I64], Type::I64),
    );
    let wrong = callback_acceptor_for_process_test(
        wrong_id,
        "wrong_alias",
        Type::function(vec![Type::I32], Type::I32),
    );

    mono.register_concrete_function("apply".to_string(), apply);
    mono.register_generic_function("identity".to_string(), identity);
    mono.register_generic_function("wrong_alias".to_string(), wrong);
    mono.resolver
        .item_paths
        .insert("apply".to_string(), apply_id);
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
                    target: HirVarTarget::Function(apply_id),
                }),
                ty: Type::function(vec![Type::function(vec![Type::I64], Type::I64)], Type::I64),
                span: crate::lexer::Span::test(),
            }),
            vec![HirExpr {
                kind: HirExprKind::Var("identity".to_string()),
                ty: Type::function(vec![Type::I64], Type::I64),
                span: crate::lexer::Span::test(),
            }],
            Some(HirCallTarget::Function(apply_id)),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(_, args, _) = &expr.kind else {
        panic!("expression should remain a call");
    };
    assert_eq!(args[0].ty, Type::function(vec![Type::I64], Type::I64));
}

#[test]
fn process_resolved_generic_function_argument_uses_def_id_not_reference_name() {
    let mut mono = Monomorphizer::new();
    let apply_id = DefId::new(CrateId(0), LocalDefId(981));
    let identity_id = DefId::new(CrateId(0), LocalDefId(982));
    let wrong_id = DefId::new(CrateId(0), LocalDefId(983));
    let apply = callback_acceptor_for_process_test(
        apply_id,
        "apply",
        Type::function(vec![Type::I64], Type::I64),
    );
    let identity = generic_identity_for_process_test(identity_id, "identity");
    let wrong = generic_identity_for_process_test(wrong_id, "wrong_alias");

    mono.register_concrete_function("apply".to_string(), apply);
    mono.register_generic_function("identity".to_string(), identity);
    mono.register_generic_function("wrong_alias".to_string(), wrong);
    mono.resolver
        .item_paths
        .insert("apply".to_string(), apply_id);
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
                    name: "apply".to_string(),
                    target: HirVarTarget::Function(apply_id),
                }),
                ty: Type::function(vec![Type::function(vec![Type::I64], Type::I64)], Type::I64),
                span: crate::lexer::Span::test(),
            }),
            vec![HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "wrong_alias".to_string(),
                    target: HirVarTarget::Function(identity_id),
                }),
                ty: Type::function(
                    vec![Type::Generic(GenericParamId {
                        owner: identity_id,
                        index: 0,
                    })],
                    Type::Generic(GenericParamId {
                        owner: identity_id,
                        index: 0,
                    }),
                ),
                span: crate::lexer::Span::test(),
            }],
            Some(HirCallTarget::Function(apply_id)),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(_, args, _) = &expr.kind else {
        panic!("expression should remain a call");
    };
    let HirExprKind::ResolvedVar(reference) = &args[0].kind else {
        panic!("function argument should be instance-backed");
    };
    let HirVarTarget::Instance(instance_id) = reference.target else {
        panic!("function argument should target a monomorphized instance");
    };
    let record = mono
        .instances
        .record(instance_id)
        .expect("instance is recorded");
    assert_eq!(record.origin, InstanceOrigin::Function(identity_id));
    assert_ne!(record.origin, InstanceOrigin::Function(wrong_id));
    assert_eq!(args[0].ty, Type::function_item(vec![Type::I64], Type::I64, crate::types::FunctionSafety::Safe));
}

#[test]
fn process_targetless_generic_function_argument_does_not_specialize_by_name() {
    let mut mono = Monomorphizer::new();
    let apply_id = DefId::new(CrateId(0), LocalDefId(991));
    let identity_id = DefId::new(CrateId(0), LocalDefId(992));
    let apply = callback_acceptor_for_process_test(
        apply_id,
        "apply",
        Type::function(vec![Type::I64], Type::I64),
    );
    let identity = generic_identity_for_process_test(identity_id, "identity");

    mono.register_concrete_function("apply".to_string(), apply);
    mono.register_generic_function("identity".to_string(), identity);
    mono.resolver
        .item_paths
        .insert("apply".to_string(), apply_id);
    mono.resolver
        .item_paths
        .insert("identity".to_string(), identity_id);

    let mut expr = HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "apply".to_string(),
                    target: HirVarTarget::Function(apply_id),
                }),
                ty: Type::function(vec![Type::function(vec![Type::I64], Type::I64)], Type::I64),
                span: crate::lexer::Span::test(),
            }),
            vec![HirExpr {
                kind: HirExprKind::Var("identity".to_string()),
                ty: Type::function(vec![Type::I64], Type::I64),
                span: crate::lexer::Span::test(),
            }],
            Some(HirCallTarget::Function(apply_id)),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(_, args, _) = &expr.kind else {
        panic!("expression should remain a call");
    };
    assert!(matches!(&args[0].kind, HirExprKind::Var(name) if name == "identity"));
    assert!(mono.instances.records().next().is_none());
}

#[test]
fn process_targetless_concrete_callee_does_not_specialize_argument_by_name() {
    let mut mono = Monomorphizer::new();
    let apply_id = DefId::new(CrateId(0), LocalDefId(1988));
    let identity_id = DefId::new(CrateId(0), LocalDefId(1989));
    let apply = callback_acceptor_for_process_test(
        apply_id,
        "apply",
        Type::function(vec![Type::I64], Type::I64),
    );
    let identity = generic_identity_for_process_test(identity_id, "identity");

    mono.register_concrete_function("apply".to_string(), apply);
    mono.register_generic_function("identity".to_string(), identity);
    mono.resolver
        .item_paths
        .insert("identity".to_string(), identity_id);

    let mut expr = HirExpr {
        kind: HirExprKind::Call(
            Box::new(HirExpr {
                kind: HirExprKind::Var("apply".to_string()),
                ty: Type::function(vec![Type::function(vec![Type::I64], Type::I64)], Type::I64),
                span: crate::lexer::Span::test(),
            }),
            vec![HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "identity".to_string(),
                    target: HirVarTarget::Function(identity_id),
                }),
                ty: Type::function(
                    vec![Type::Generic(GenericParamId {
                        owner: identity_id,
                        index: 0,
                    })],
                    Type::Generic(GenericParamId {
                        owner: identity_id,
                        index: 0,
                    }),
                ),
                span: crate::lexer::Span::test(),
            }],
            None,
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(_, args, _) = &expr.kind else {
        panic!("expression should remain a call");
    };
    assert!(matches!(args[0].kind, HirExprKind::ResolvedVar(_)));
    assert!(mono.instances.records().next().is_none());
}

#[test]
fn process_external_generic_function_argument_uses_def_id_not_reference_name() {
    let mut mono = Monomorphizer::new();
    mono.register_test_dependency_identity(CrateId(1));
    let apply_id = DefId::new(CrateId(0), LocalDefId(988));
    let identity_id = DefId::new(CrateId(1), LocalDefId(989));
    let wrong_id = DefId::new(CrateId(1), LocalDefId(990));
    let apply = callback_acceptor_for_process_test(
        apply_id,
        "apply",
        Type::function(vec![Type::I64], Type::I64),
    );
    let identity = generic_identity_for_process_test(identity_id, "identity");
    let wrong = generic_identity_for_process_test(wrong_id, "wrong_alias");

    mono.register_concrete_function("apply".to_string(), apply);
    mono.register_external_generic_function("dep::identity".to_string(), identity);
    mono.register_external_generic_function("wrong_alias".to_string(), wrong);
    mono.resolver
        .item_paths
        .insert("apply".to_string(), apply_id);
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
                    name: "apply".to_string(),
                    target: HirVarTarget::Function(apply_id),
                }),
                ty: Type::function(vec![Type::function(vec![Type::I64], Type::I64)], Type::I64),
                span: crate::lexer::Span::test(),
            }),
            vec![HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "wrong_alias".to_string(),
                    target: HirVarTarget::Extern(identity_id),
                }),
                ty: Type::function(
                    vec![Type::Generic(GenericParamId {
                        owner: identity_id,
                        index: 0,
                    })],
                    Type::Generic(GenericParamId {
                        owner: identity_id,
                        index: 0,
                    }),
                ),
                span: crate::lexer::Span::test(),
            }],
            Some(HirCallTarget::Function(apply_id)),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(_, args, _) = &expr.kind else {
        panic!("expression should remain a call");
    };
    let HirExprKind::ResolvedVar(reference) = &args[0].kind else {
        panic!("function argument should be instance-backed");
    };
    let HirVarTarget::Instance(instance_id) = reference.target else {
        panic!("function argument should target a monomorphized instance");
    };
    let record = mono
        .instances
        .record(instance_id)
        .expect("instance is recorded");
    assert_eq!(record.origin, InstanceOrigin::Function(identity_id));
    assert_ne!(record.origin, InstanceOrigin::Function(wrong_id));
}

#[test]
fn process_external_callee_param_lookup_uses_def_id() {
    let mut mono = Monomorphizer::new();
    mono.register_test_dependency_identity(CrateId(1));
    let apply_id = DefId::new(CrateId(1), LocalDefId(991));
    let identity_id = DefId::new(CrateId(1), LocalDefId(992));
    let wrong_id = DefId::new(CrateId(1), LocalDefId(993));
    let apply_generic_id = GenericParamId {
        owner: apply_id,
        index: 0,
    };
    let apply = callback_acceptor_for_process_test(
        apply_id,
        "apply",
        Type::function(vec![Type::I64], Type::I64),
    );
    let identity = generic_identity_for_process_test(identity_id, "identity");
    let wrong = generic_identity_for_process_test(wrong_id, "wrong_alias");

    mono.register_external_generic_function("dep::apply".to_string(), apply);
    mono.register_external_generic_function("dep::identity".to_string(), identity);
    mono.register_external_generic_function("wrong_alias".to_string(), wrong);
    mono.resolver
        .item_paths
        .insert("dep::apply".to_string(), apply_id);
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
                    name: "dep::apply".to_string(),
                    target: HirVarTarget::Extern(apply_id),
                }),
                ty: Type::function(
                    vec![Type::function(vec![Type::I64], Type::I64)],
                    Type::Generic(apply_generic_id),
                ),
                span: crate::lexer::Span::test(),
            }),
            vec![HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "wrong_alias".to_string(),
                    target: HirVarTarget::Extern(identity_id),
                }),
                ty: Type::function(
                    vec![Type::Generic(GenericParamId {
                        owner: identity_id,
                        index: 0,
                    })],
                    Type::Generic(GenericParamId {
                        owner: identity_id,
                        index: 0,
                    }),
                ),
                span: crate::lexer::Span::test(),
            }],
            Some(HirCallTarget::Function(apply_id)),
        ),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    };

    mono.process_expr(&mut expr);

    let HirExprKind::Call(_, args, _) = &expr.kind else {
        panic!("expression should remain a call");
    };
    let HirExprKind::ResolvedVar(reference) = &args[0].kind else {
        panic!("function argument should be instance-backed");
    };
    let HirVarTarget::Instance(instance_id) = reference.target else {
        panic!("function argument should target a monomorphized instance");
    };
    let record = mono
        .instances
        .record(instance_id)
        .expect("instance is recorded");
    assert_eq!(record.origin, InstanceOrigin::Function(identity_id));
    assert_ne!(record.origin, InstanceOrigin::Function(wrong_id));
}

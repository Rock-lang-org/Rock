use super::*;

#[test]
fn static_method_value_lambda_carries_exact_target() {
    let resolved = compile_source_to_resolved_hir_for_test(
            "struct Math\n\nimpl Math\n    double = value -> value\n\nmain = ->\n    operation = Math::double\n    operation 6\n",
        );
    let program = resolved.program.program();
    let imp = program
        .impls
        .values()
        .find(|imp| imp.methods.contains_key("double"))
        .expect("static impl");
    let method_id = imp.methods["double"].id;
    let (_, main) = program.test_function_by_name("main").expect("main");
    let value = main
        .body
        .stmts
        .iter()
        .find_map(|stmt| match stmt {
            HirStmtFor::Let { value, .. } => Some(value),
            _ => None,
        })
        .expect("static method value binding");
    let HirExprKindFor::Lambda { body, .. } = &value.kind else {
        panic!("expected static method value lambda, got {:?}", value.kind);
    };
    let [HirStmtFor::Expr(HirExprFor {
        kind: HirExprKindFor::Call(_, _, Some(HirCallTarget::StaticMethod(target))),
        ..
    })] = body.stmts.as_slice()
    else {
        panic!("expected exact static call in generated lambda")
    };

    assert_eq!(target.method.impl_id(), Some(imp.id));
    assert_eq!(target.method.method_id(), Some(method_id));
}

fn ident_pattern(name: &str) -> Pattern {
    Pattern {
        binding: Some(ident(name)),
        kind: PatternKind::Ident(IdentPattern {
            name: ident(name),
            mut_: false,
        }),
    }
}

fn var_expr(name: &str) -> Expression {
    Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
        operand: Operand::Ident(IdentifierPath {
            path: vec![IdentOrType::Ident(ident(name))],
        }),
        secondaries: None,
        type_annotation: None,
    }))
}

#[test]
fn test_lower_lambda_carries_capture_metadata() {
    let mut lowerer = Lowerer::new_for_test();
    let lambda = LambdaDecl {
        parameters: vec![ident_pattern("x")],
        body: Block {
            statements: vec![Statement::Expression(var_expr("base"))],
        },
        arrow_kind: LambdaArrowKind::Normal,
        span: crate::lexer::Span::test(),
    };

    let hir = lowerer.with_test_body_context(|lowerer| {
        let base_id = lowerer.fresh_local_id();
        lowerer
            .scope
            .define_local("base".to_string(), Type::I64, false, base_id);
        lowerer.lower_lambda(&lambda)
    });
    let Type::Function {
        callable_kind,
        captures: type_captures,
        ..
    } = &hir.ty
    else {
        panic!("expected lambda function type");
    };
    assert_eq!(*callable_kind, CallableKind::Fn);
    assert_eq!(type_captures[0].kind, CaptureKind::Move);
    assert_eq!(type_captures[0].ty, Type::I64);
    match hir.kind {
        HirExprKind::Lambda { captures, .. } => {
            assert_eq!(captures.len(), 1);
            assert_eq!(captures[0].name, "base");
        }
        _ => panic!("expected lambda"),
    }
}

#[test]
fn lower_lambda_reference_pattern_wraps_parameter_type() {
    let mut lowerer = Lowerer::new_for_test();
    let lambda = LambdaDecl {
        parameters: vec![Pattern {
            binding: None,
            kind: PatternKind::Reference {
                pattern: Box::new(ident_pattern("value")),
                mutable: false,
            },
        }],
        body: Block {
            statements: vec![Statement::Expression(var_expr("value"))],
        },
        arrow_kind: LambdaArrowKind::Normal,
        span: crate::lexer::Span::test(),
    };

    let hir = lowerer.with_test_body_context(|lowerer| lowerer.lower_lambda(&lambda));
    let HirExprKind::Lambda { params, .. } = &hir.kind else {
        panic!("expected lambda");
    };
    let Type::Function {
        params: function_params,
        ..
    } = &hir.ty
    else {
        panic!("expected function type");
    };

    assert_eq!(params[0].ty, function_params[0]);
    assert!(matches!(
        &params[0].ty,
        Type::Reference {
            mutable: false,
            inner
        } if matches!(inner.as_ref(), Type::TypeVar(_))
    ));
}

#[test]
fn outer_lambda_does_not_capture_local_used_by_nested_lambda() {
    let mut lowerer = Lowerer::new_for_test();
    let captures = lowerer.with_test_body_context(|lowerer| {
        let next_id = lowerer.fresh_local_id();
        lowerer
            .scope
            .define_local("next".to_string(), Type::I64, false, next_id);
        let nested_body = HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: "next".to_string(),
                    target: HirVarTarget::Local(next_id),
                }),
                ty: Type::I64,
                span: crate::lexer::Span::test(),
            })],
            ty: Type::I64,
        };
        let body = HirBlock {
            stmts: vec![
                HirStmt::Let {
                    name: "next".to_string(),
                    local_id: next_id,
                    ty: Type::I64,
                    value: HirExpr {
                        kind: HirExprKind::IntLiteral(1),
                        ty: Type::I64,
                        span: crate::lexer::Span::test(),
                    },
                    mutable: false,
                },
                HirStmt::Expr(HirExpr {
                    kind: HirExprKind::Lambda {
                        params: Vec::new(),
                        body: nested_body,
                        captures: Vec::new(),
                    },
                    ty: Type::function(Vec::new(), Type::I64),
                    span: crate::lexer::Span::test(),
                }),
            ],
            ty: Type::function(Vec::new(), Type::I64),
        };

        lowerer.collect_lambda_captures(&body, &[])
    });

    assert!(captures.is_empty());
}

#[test]
fn lower_lambda_captures_resolved_local_reference() {
    let mut lowerer = Lowerer::new_for_test();
    let lambda = LambdaDecl {
        parameters: vec![ident_pattern("x")],
        body: Block {
            statements: vec![Statement::Expression(var_expr("base"))],
        },
        arrow_kind: LambdaArrowKind::Normal,
        span: crate::lexer::Span::test(),
    };

    let hir = lowerer.with_test_body_context(|lowerer| {
        let base_id = lowerer.fresh_local_id();
        lowerer
            .scope
            .define_local("base".to_string(), Type::I64, false, base_id);
        lowerer.lower_lambda(&lambda)
    });
    match hir.kind {
        HirExprKind::Lambda { captures, .. } => {
            assert_eq!(captures.len(), 1);
            assert_eq!(captures[0].name, "base");
        }
        _ => panic!("expected lambda"),
    }
}

#[test]
fn lambda_capture_records_captured_local_id() {
    let mut lowerer = Lowerer::new_for_test();
    let lambda = LambdaDecl {
        parameters: vec![ident_pattern("x")],
        body: Block {
            statements: vec![Statement::Expression(var_expr("base"))],
        },
        arrow_kind: LambdaArrowKind::Normal,
        span: crate::lexer::Span::test(),
    };

    let (captured_id, hir) = lowerer.with_test_body_context(|lowerer| {
        let captured_id = lowerer.fresh_local_id();
        lowerer
            .scope
            .define_local("base".to_string(), Type::I64, false, captured_id);
        (captured_id, lowerer.lower_lambda(&lambda))
    });
    match hir.kind {
        HirExprKind::Lambda { captures, .. } => {
            assert_eq!(captures.len(), 1);
            assert_eq!(captures[0].name, "base");
            assert_eq!(captures[0].local_id, captured_id);
        }
        other => panic!("expected lambda, got {other:?}"),
    }
}

#[test]
fn lambda_captures_root_scope_lexical_function_value() {
    let mut lowerer = Lowerer::new_for_test();
    let lambda = LambdaDecl {
        parameters: vec![],
        body: Block {
            statements: vec![Statement::Expression(var_expr("callback"))],
        },
        arrow_kind: LambdaArrowKind::Normal,
        span: crate::lexer::Span::test(),
    };
    let callback_id = HirLocalId(91);
    lowerer.scope.define_local(
        "callback".to_string(),
        Type::function(Vec::new(), Type::I64),
        false,
        callback_id,
    );

    let hir = lowerer.lower_lambda(&lambda);

    let HirExprKind::Lambda { captures, .. } = hir.kind else {
        panic!("expected lambda");
    };
    assert_eq!(captures.len(), 1);
    assert_eq!(captures[0].name, "callback");
    assert_eq!(captures[0].local_id, callback_id);
}

#[test]
fn lambda_does_not_capture_explicit_top_level_function() {
    let mut lowerer = Lowerer::new_for_test();
    let lambda = LambdaDecl {
        parameters: vec![],
        body: Block {
            statements: vec![Statement::Expression(var_expr("callback"))],
        },
        arrow_kind: LambdaArrowKind::Normal,
        span: crate::lexer::Span::test(),
    };
    lowerer.scope.define_top_level(
        "callback".to_string(),
        Type::function(Vec::new(), Type::I64),
        false,
    );

    let hir = lowerer.lower_lambda(&lambda);

    let HirExprKind::Lambda { captures, .. } = hir.kind else {
        panic!("expected lambda");
    };
    assert!(captures.is_empty());
}

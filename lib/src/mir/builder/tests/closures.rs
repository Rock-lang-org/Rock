use super::*;

#[test]
fn closure_rvalue_uses_stable_closure_identity() {
    let function_id = DefId::new(CrateId(0), LocalDefId(50));
    let function = HirFunction {
        body: HirBlock {
            stmts: vec![HirStmt::Expr(expr(
                HirExprKind::Lambda {
                    params: vec![],
                    body: HirBlock {
                        stmts: vec![],
                        ty: Type::Unit,
                    },
                    captures: vec![],
                },
                Type::Unit,
            ))],
            ty: Type::Unit,
        },
        ..empty_function(function_id, "make_closure")
    };
    let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::from([(function_id, function)]),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        HirNameTables {
            functions_by_name: std::collections::HashMap::from([(
                "make_closure".to_string(),
                function_id,
            )]),
            ..HirNameTables::default()
        },
        &std::collections::HashMap::from([(function_id, "make_closure".to_string())]),
    );
    let mir = MirBuilder::build(&program);
    assert_mir_agreement_clean(&mir);
    let function = mir
        .function(crate::mir::MirFunctionId::Function(function_id))
        .expect("function MIR should exist");

    assert!(function
        .basic_blocks
        .iter()
        .flat_map(|block| &block.statements)
        .any(|stmt| matches!(
            &stmt.kind,
            StatementKind::Assign(_, Rvalue::Closure(closure))
                if closure.id.owner == crate::mir::MirFunctionId::Function(function_id)
                    && closure.display_name.starts_with("lambda_")
        )));
}

#[test]
fn build_function_emits_closure_body_function() {
    let function_id = DefId::new(CrateId(0), LocalDefId(20));
    let owner = crate::mir::MirFunctionId::Function(function_id);
    let lambda = HirExpr {
        kind: HirExprKind::Lambda {
            params: vec![crate::hir::HirParam {
                name: "x".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::I64,
                mutable: false,
                is_ref: false,
            }],
            body: HirBlock {
                stmts: vec![HirStmt::Expr(HirExpr {
                    kind: HirExprKind::Var("x".to_string()),
                    ty: Type::I64,
                    span: crate::lexer::Span::test(),
                })],
                ty: Type::I64,
            },
            captures: Vec::new(),
        },
        ty: Type::function(vec![Type::I64], Type::I64),
        span: crate::lexer::Span::test(),
    };
    let function = HirFunction {
        id: function_id,
        name: "make_lambda".to_string(),
        generic_params: Vec::new(),
        generic_bounds: std::collections::HashMap::new().into(),
        params: Vec::new(),
        ret_type: lambda.ty.clone(),
        body: HirBlock {
            stmts: vec![HirStmt::Expr(lambda)],
            ty: Type::function(vec![Type::I64], Type::I64),
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::from([(function_id, function)]),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        HirNameTables {
            functions_by_name: std::collections::HashMap::from([(
                "make_lambda".to_string(),
                function_id,
            )]),
            ..HirNameTables::default()
        },
        &std::collections::HashMap::from([(function_id, "make_lambda".to_string())]),
    );

    let mir = MirBuilder::build(&program);
    let closure_id = crate::mir::MirClosureId {
        owner,
        local_index: 0,
    };

    assert!(mir
        .function(crate::mir::MirFunctionId::Closure(Box::new(
            closure_id.clone()
        )))
        .is_some());
}

#[test]
fn build_function_binds_closure_body_captures() {
    let function_id = DefId::new(CrateId(0), LocalDefId(21));
    let owner = crate::mir::MirFunctionId::Function(function_id);
    let capture = crate::hir::HirClosureCapture {
        name: "captured".to_string(),
        local_id: crate::ids::HirLocalId(0),
        kind: crate::hir::HirClosureCaptureKind::SharedBorrow,
        mutable: false,
        ty: Type::I64,
    };
    let nested_lambda = HirExpr {
        kind: HirExprKind::Lambda {
            params: Vec::new(),
            body: HirBlock {
                stmts: vec![HirStmt::Expr(HirExpr {
                    kind: HirExprKind::Var("captured".to_string()),
                    ty: Type::I64,
                    span: crate::lexer::Span::test(),
                })],
                ty: Type::I64,
            },
            captures: vec![capture.clone()],
        },
        ty: Type::function(vec![], Type::I64),
        span: crate::lexer::Span::test(),
    };
    let lambda = HirExpr {
        kind: HirExprKind::Lambda {
            params: Vec::new(),
            body: HirBlock {
                stmts: vec![
                    HirStmt::Let {
                        name: "nested".to_string(),
                        local_id: crate::ids::HirLocalId(1),
                        ty: Type::function(vec![], Type::I64),
                        value: nested_lambda,
                        mutable: false,
                    },
                    HirStmt::Expr(HirExpr {
                        kind: HirExprKind::Var("captured".to_string()),
                        ty: Type::I64,
                        span: crate::lexer::Span::test(),
                    }),
                ],
                ty: Type::I64,
            },
            captures: vec![capture],
        },
        ty: Type::function(vec![], Type::I64),
        span: crate::lexer::Span::test(),
    };
    let function = HirFunction {
        id: function_id,
        name: "make_capturing_lambda".to_string(),
        generic_params: Vec::new(),
        generic_bounds: std::collections::HashMap::new().into(),
        params: vec![crate::hir::HirParam {
            name: "captured".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::I64,
            mutable: false,
            is_ref: false,
        }],
        ret_type: lambda.ty.clone(),
        body: HirBlock {
            stmts: vec![HirStmt::Expr(lambda)],
            ty: Type::function(vec![], Type::I64),
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::from([(function_id, function)]),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        HirNameTables {
            functions_by_name: std::collections::HashMap::from([(
                "make_capturing_lambda".to_string(),
                function_id,
            )]),
            ..HirNameTables::default()
        },
        &std::collections::HashMap::from([(function_id, "make_capturing_lambda".to_string())]),
    );

    let mir = MirBuilder::build(&program);
    let type_context = mir.type_context.clone();
    let closure_id = crate::mir::MirClosureId {
        owner,
        local_index: 0,
    };
    let closure_function = mir
        .function(crate::mir::MirFunctionId::Closure(Box::new(
            closure_id.clone(),
        )))
        .expect("closure MIR body should exist");
    let capture = closure_function
        .closure_captures
        .iter()
        .find(|capture| capture.name == "captured")
        .expect("nested closure body should record captured local");

    assert_eq!(closure_function.arg_count, 0);
    assert_eq!(closure_function.closure_captures.len(), 1);
    assert_eq!(
        type_context.type_for(closure_function.local_decls[capture.local.0].ty),
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        }
    );
    assert_eq!(
        closure_function.local_decls[capture.local.0]
            .name
            .as_deref(),
        Some("captured")
    );
    assert!(closure_function
        .basic_blocks
        .iter()
        .flat_map(|block| &block.statements)
        .any(|stmt| matches!(
            &stmt.kind,
            StatementKind::Assign(
                Place {
                    local: Local(0),
                    projection
                },
                Rvalue::Use(Operand::Copy(Place {
                    local,
                    projection: source_projection,
                }))
            ) if projection.is_empty()
                && *local == capture.local
                && source_projection == &[Projection::Deref]
        )));
}

#[test]
fn test_build_function_exposes_closure_captures() {
    let program = empty_program();
    let func = HirFunction {
        id: DefId::new(CrateId(0), LocalDefId(0)),
        name: "main".to_string(),
        generic_params: vec![],
        generic_bounds: std::collections::HashMap::new().into(),
        params: vec![],
        ret_type: Type::Unit,
        body: HirBlock {
            stmts: vec![],
            ty: Type::Unit,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };

    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    let mir_func = builder.build_function(
        MirFunctionId::Function(func.id),
        &func.name,
        &func,
        false,
        Some(func.id),
        None,
    );
    assert!(mir_func.closure_captures.is_empty());
    assert_eq!(
        mir_func.local_decls[0].origin(),
        crate::mir::MirOrigin::synthetic(crate::mir::MirSyntheticOrigin::Unknown, None)
    );
}

#[test]
fn test_build_function_with_lambda_still_builds() {
    let program = empty_program();
    let func = HirFunction {
        id: DefId::new(CrateId(0), LocalDefId(0)),
        name: "main".to_string(),
        generic_params: vec![],
        generic_bounds: std::collections::HashMap::new().into(),
        params: vec![],
        ret_type: Type::Unit,
        body: HirBlock {
            stmts: vec![HirStmt::Expr(expr(
                HirExprKind::Lambda {
                    params: vec![],
                    body: HirBlock {
                        stmts: vec![HirStmt::Expr(expr(HirExprKind::IntLiteral(1), Type::I64))],
                        ty: Type::I64,
                    },
                    captures: vec![],
                },
                Type::function(vec![], Type::I64),
            ))],
            ty: Type::I64,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };

    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    let mir_func = builder.build_function(
        MirFunctionId::Function(func.id),
        &func.name,
        &func,
        false,
        Some(func.id),
        None,
    );
    assert!(mir_func.closure_captures.is_empty());
}

#[test]
fn test_lower_expr_lambda_emits_closure_rvalue() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.current_function_id = Some(MirFunctionId::Function(DefId::new(
        CrateId(0),
        LocalDefId(0),
    )));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::I64),
        mutability: Mutability::Not,
        name: Some("outer".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    builder.var_map.insert("outer".to_string(), Local(0));

    let lambda = HirExpr {
        kind: HirExprKind::Lambda {
            params: vec![],
            body: HirBlock {
                stmts: vec![HirStmt::Expr(expr(
                    HirExprKind::Var("outer".to_string()),
                    Type::I64,
                ))],
                ty: Type::I64,
            },
            captures: vec![crate::hir::HirClosureCapture {
                name: "outer".to_string(),
                local_id: crate::ids::HirLocalId(0),
                kind: crate::hir::HirClosureCaptureKind::SharedBorrow,
                mutable: false,
                ty: Type::I64,
            }],
        },
        ty: Type::function(vec![], Type::I64),
        span: span(),
    };

    let dest = Place {
        local: Local(0),
        projection: vec![],
    };
    builder.lower_expr_with_context(&lambda, dest, false);
    assert!(matches!(
        &builder.blocks[0].statements.last().unwrap().kind,
        StatementKind::Assign(_, Rvalue::Closure(_))
    ));
    let nested = builder.closures.take_nested();
    assert_eq!(nested.len(), 1);
    assert_eq!(nested[0].local_decls[0].span, Some(lambda.span.clone()));
    assert_eq!(nested[0].local_decls[1].span, Some(lambda.span.clone()));
    assert_eq!(nested[0].closure_captures[0].span, Some(lambda.span));
}

use super::*;

#[test]
fn binary_op_lowers_to_mir_owned_operator() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    builder.blocks.push(BasicBlock {
        statements: Vec::new(),
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::I64),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    let add = expr(
        HirExprKind::BinOp(
            BinOp::Add,
            Box::new(expr(HirExprKind::IntLiteral(1), Type::I64)),
            Box::new(expr(HirExprKind::IntLiteral(2), Type::I64)),
        ),
        Type::I64,
    );

    builder.lower_expr(
        &add,
        Place {
            local: Local(0),
            projection: vec![],
        },
    );

    assert!(builder.blocks[0]
        .statements
        .iter()
        .any(|statement| matches!(
            &statement.kind,
            StatementKind::Assign(
                Place {
                    local: Local(0),
                    projection,
                },
                Rvalue::BinaryOp(crate::mir::MirBinOp::Add, _, _),
            ) if projection.is_empty()
        )));
}

#[test]
fn unary_op_lowers_to_mir_owned_operator() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    builder.blocks.push(BasicBlock {
        statements: Vec::new(),
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::Bool),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    let not = expr(
        HirExprKind::UnaryOp(
            UnaryOp::Not,
            Box::new(expr(HirExprKind::BoolLiteral(true), Type::Bool)),
        ),
        Type::Bool,
    );

    builder.lower_expr(
        &not,
        Place {
            local: Local(0),
            projection: vec![],
        },
    );

    assert!(builder.blocks[0]
        .statements
        .iter()
        .any(|statement| matches!(
            &statement.kind,
            StatementKind::Assign(
                Place {
                    local: Local(0),
                    projection,
                },
                Rvalue::UnaryOp(crate::mir::MirUnaryOp::Not, _),
            ) if projection.is_empty()
        )));
}

#[test]
fn numeric_cast_lowers_to_runtime_cast_rvalue() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let mut builder = builder_with_var(&program, &type_context, "n", Type::I64);
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::F64),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    let cast_expr = expr(
        HirExprKind::Cast(
            Box::new(expr(HirExprKind::Var("n".to_string()), Type::I64)),
            Type::F64,
        ),
        Type::F64,
    );

    builder.lower_expr_with_context(
        &cast_expr,
        Place {
            local: Local(1),
            projection: vec![],
        },
        false,
    );

    let f64_id = builder.type_id_for(&Type::F64);
    assert!(builder.blocks[0].statements.iter().any(|stmt| matches!(
        &stmt.kind,
        StatementKind::Assign(_, Rvalue::Cast(Operand::Copy(_), ty))
            if *ty == f64_id
    )));

    assert_builder_agreement_clean(builder, Type::Unit);
}

#[test]
fn test_lower_expr_cast_to_rvalue_cast() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::Pointer(Box::new(Type::I64))),
        mutability: Mutability::Not,
        name: Some("tmp".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    builder.var_map.insert("tmp".to_string(), Local(0));
    let dest = Place {
        local: Local(0),
        projection: vec![],
    };
    let cast_expr = expr(
        HirExprKind::Cast(
            Box::new(expr(
                HirExprKind::Var("tmp".to_string()),
                Type::Pointer(Box::new(Type::I64)),
            )),
            Type::Pointer(Box::new(Type::U8)),
        ),
        Type::Pointer(Box::new(Type::U8)),
    );

    let target_id = builder.type_id_for(&Type::Pointer(Box::new(Type::U8)));
    builder.lower_expr_with_context(&cast_expr, dest, false);
    assert!(builder.blocks[0].statements.iter().any(|stmt| matches!(
        &stmt.kind,
        StatementKind::Assign(_, Rvalue::Cast(Operand::Copy(_), ty))
            if *ty == target_id
    )));
}

#[test]
fn test_lower_expr_non_pointer_cast_remains_copy() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::I64),
        mutability: Mutability::Not,
        name: Some("tmp".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    builder.var_map.insert("tmp".to_string(), Local(0));
    let dest = Place {
        local: Local(0),
        projection: vec![],
    };
    let cast_expr = expr(
        HirExprKind::Cast(
            Box::new(expr(HirExprKind::Var("tmp".to_string()), Type::I64)),
            Type::I64,
        ),
        Type::I64,
    );

    builder.lower_expr_with_context(&cast_expr, dest, false);
    assert!(builder.blocks[0].statements.iter().any(|stmt| matches!(
        &stmt.kind,
        StatementKind::Assign(_, Rvalue::Use(Operand::Copy(_)))
    )));
}

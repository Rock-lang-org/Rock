use super::*;

#[test]
fn value_intrinsic_lowers_to_intrinsic_call_terminator() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    builder.direct_drop_types.insert(struct_ty(0));
    builder.blocks.push(BasicBlock {
        statements: vec![],
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

    let intrinsic_expr = expr(
        HirExprKind::Intrinsic {
            name: "I64Add".to_string(),
            args: vec![
                expr(HirExprKind::IntLiteral(1), Type::I64),
                expr(HirExprKind::IntLiteral(2), Type::I64),
            ],
        },
        Type::I64,
    );

    builder.lower_expr_with_context(
        &intrinsic_expr,
        Place {
            local: Local(0),
            projection: vec![],
        },
        false,
    );

    assert!(matches!(
        &builder.blocks[0].terminator,
        Some(Terminator::Call {
            func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                crate::mir::MirCallableKey::Intrinsic(intrinsic)
            ))),
            args,
            destination,
            span: Some(call_span),
            ..
        }) if *intrinsic == MirIntrinsicId::I64Add
            && args.len() == 2
            && destination.local == Local(0)
            && call_span == &intrinsic_expr.span
    ));

    assert_builder_agreement_clean(builder, Type::I64);
}

#[test]
#[should_panic(expected = "unknown intrinsic reached MIR lowering")]
fn unknown_intrinsic_source_name_panics_before_mir_callable_identity() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::Unit),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    let intrinsic_expr = expr(
        HirExprKind::Intrinsic {
            name: "NotARealIntrinsic".to_string(),
            args: vec![],
        },
        Type::Unit,
    );

    builder.lower_expr_with_context(
        &intrinsic_expr,
        Place {
            local: Local(0),
            projection: vec![],
        },
        false,
    );
}

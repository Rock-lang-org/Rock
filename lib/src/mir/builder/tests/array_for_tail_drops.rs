use super::*;

fn tail_helper_calls(builder: &MirBuilder<'_>) -> Vec<crate::ids::TypeId> {
    builder.blocks.iter().filter_map(|block| match &block.terminator {
        Some(Terminator::Call {
            func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                MirCallableKey::RuntimeHelper(crate::mir::MirRuntimeHelper::ArrayTailDrop(ty)),
            ))),
            args,
            ..
        }) => {
            assert_eq!(args.len(), 3);
            Some(*ty)
        }
        _ => None,
    }).collect()
}

fn drops_of_array_elements(builder: &MirBuilder<'_>) -> usize {
    builder
        .blocks
        .iter()
        .filter(|block| {
            matches!(
                &block.terminator,
                Some(Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. })
                    if place.projection.iter().any(|p| matches!(p, Projection::Index(_)))
            )
        })
        .count()
}

fn tail_loop_heads(builder: &MirBuilder<'_>) -> usize {
    // A tail-drop loop head is the block comparing the dynamic index to the length.
    builder
        .blocks
        .iter()
        .filter(|block| {
            block.statements.iter().any(|stmt| {
                stmt.cleanup
                    && matches!(
                        &stmt.kind,
                        StatementKind::Assign(
                            _,
                            Rvalue::BinaryOp(crate::mir::MirBinOp::Lt, _, _)
                        )
                    )
            })
        })
        .count()
}

/// `for x in arr` with a body that either returns or breaks, followed by a
/// post-loop `return`. Returns the builder after lowering.
fn lower_array_for_with_exits<'a>(
    program: &'a HirProgram,
    type_context: &'a RefCell<crate::type_context::TypeContext>,
    elem_ty: Type,
    cleanup_elems: bool,
    len: u64,
) -> MirBuilder<'a> {
    let array_ty = Type::Array(Box::new(elem_ty), crate::types::ArrayLen::Value(len));
    let mut builder = builder_with_var(program, type_context, "arr", array_ty.clone());
    if cleanup_elems {
        builder.direct_drop_types.insert(struct_ty(0));
    }
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
    builder.scope_locals.push(Vec::new());

    let body = HirBlock {
        stmts: vec![HirStmt::Expr(expr(
            HirExprKind::If {
                condition: Box::new(expr(HirExprKind::BoolLiteral(true), Type::Bool)),
                then_branch: HirBlock {
                    stmts: vec![HirStmt::Return(None)],
                    ty: Type::Unit,
                },
                else_branch: Some(HirBlock {
                    stmts: vec![HirStmt::Break(None)],
                    ty: Type::Unit,
                }),
            },
            Type::Unit,
        ))],
        ty: Type::Unit,
    };
    let for_expr = expr(
        HirExprKind::For {
            var: "x".to_string(),
            local_id: crate::ids::HirLocalId(0),
            iter: Box::new(expr(HirExprKind::Var("arr".to_string()), array_ty)),
            body,
        },
        Type::Unit,
    );
    builder.lower_expr(
        &for_expr,
        Place {
            local: Local(1),
            projection: vec![],
        },
    );
    // An exit after normal completion, like a `return` or `?` after the loop.
    builder.terminate_with_cleanup(0, Terminator::Return);
    builder
}

#[test]
fn array_for_tail_drops_only_on_exits_inside_the_loop() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let builder = lower_array_for_with_exits(&program, &type_context, struct_ty(0), true, 3);

    // `return` and `break` inside the body each drop the unvisited tail; the
    // exit after normal completion (`terminate_with_cleanup` at the end of the
    // helper) emits none.
    assert_eq!(tail_helper_calls(&builder).len(), 2);
    assert_eq!(drops_of_array_elements(&builder), 0);
    assert_eq!(tail_loop_heads(&builder), 0);
}

#[test]
fn array_for_exits_after_the_loop_emit_no_tail_drop_code() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let mut builder = lower_array_for_with_exits(&program, &type_context, struct_ty(0), true, 3);
    let drops_before = drops_of_array_elements(&builder);
    let loops_before = tail_loop_heads(&builder);
    let calls_before = tail_helper_calls(&builder);

    // The helper already ended the function; a fresh post-loop block with its own
    // exit must only end storage, never loop over elements again.
    builder.current_block = Some(builder.new_block());
    builder.finish_scope_locals(builder.scope_locals[0].clone());
    builder.terminate_with_cleanup(0, Terminator::Return);

    assert_eq!(drops_of_array_elements(&builder), drops_before);
    assert_eq!(tail_loop_heads(&builder), loops_before);
    assert_eq!(tail_helper_calls(&builder), calls_before);
}

#[test]
fn array_for_without_cleanup_elements_emits_no_tail_drops() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let builder = lower_array_for_with_exits(&program, &type_context, Type::I64, false, 3);

    assert_eq!(drops_of_array_elements(&builder), 0);
    assert_eq!(tail_loop_heads(&builder), 0);
    assert!(tail_helper_calls(&builder).is_empty());
}

#[test]
fn array_tail_helpers_share_by_element_type_and_elaborate_nested_cleanup() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let mut functions = std::collections::BTreeMap::new();
    // Multiple callers, with multiple exits each, reference a single helper.
    for index in 0..2 {
        let builder = lower_array_for_with_exits(&program, &type_context, struct_ty(0), true, 3 + index as u64);
        let id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(100 + index)));
        let ret_type = builder.type_id_for(&Type::Unit);
        functions.insert(id.clone(), MirFunction {
            id,
            name: format!("caller_{index}"),
            basic_blocks: builder.blocks,
            local_decls: builder.locals,
            closure_captures: Vec::new(),
            arg_count: 0,
            ret_type,
            ownership: builder.ownership,
        });
    }
    let drops = std::collections::HashSet::from([struct_ty(0)]);
    MirBuilder::populate_array_tail_helpers(&program, &type_context, &drops, &mut functions);
    assert_eq!(functions.values().filter(|f| matches!(f.id, MirFunctionId::ArrayTailDrop(_))).count(), 1);
    let helper = functions.values().find(|f| matches!(f.id, MirFunctionId::ArrayTailDrop(_))).unwrap();
    assert_eq!(helper.arg_count, 3);
    assert!(matches!(type_context.borrow().type_for(helper.local_decls[1].ty), Type::Pointer(_)));
    // Repeating collection must not create another helper identity.
    MirBuilder::populate_array_tail_helpers(&program, &type_context, &drops, &mut functions);
    assert_eq!(functions.len(), 3);

    let nested = Type::Tuple(vec![struct_ty(0), struct_ty(0)]);
    let elem = type_context.borrow_mut().intern_type(&nested);
    let mut builder = MirBuilder::with_instance_metadata(&program, &type_context, Default::default(), drops);
    let helper = builder.build_array_tail_helper(elem);
    let nested_drops = helper.basic_blocks.iter().filter(|block| matches!(
        &block.terminator,
        Some(Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. })
            if matches!(place.projection.as_slice(), [Projection::Index(_), Projection::Field { .. }])
    )).count();
    assert_eq!(nested_drops, 2);
}

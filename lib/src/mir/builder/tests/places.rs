use super::*;

#[test]
fn lower_place_resolves_local_var_target() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let mut builder = builder_with_var(&program, &type_context, "v", Type::I64);
    let place_expr = expr(
        HirExprKind::ResolvedVar(HirVarRef {
            name: "v".to_string(),
            target: HirVarTarget::Local(crate::ids::HirLocalId(0)),
        }),
        Type::I64,
    );

    let place = builder
        .lower_place(&place_expr)
        .expect("expected resolved local place");

    assert_eq!(place.local, Local(0));
    assert!(place.projection.is_empty());
}

#[test]
fn test_lower_place_field_projection() {
    let program = program_with_struct(vec!["a", "b"]);
    let type_context = test_type_context(&program);
    let mut builder = builder_with_var(&program, &type_context, "x", struct_ty(0));
    let owner = program.test_struct_by_name("Foo").unwrap().0;
    let place_expr = expr(
        HirExprKind::FieldAccess(
            Box::new(expr(HirExprKind::Var("x".to_string()), struct_ty(0))),
            "b".to_string(),
            Some(crate::hir::HirFieldLocation {
                owner,
                field_id: crate::ids::FieldId(1),
                name: "b".to_string(),
            }),
        ),
        Type::I64,
    );

    let place = builder
        .lower_place(&place_expr)
        .expect("expected field place");
    assert_eq!(place.local, Local(0));
    assert_eq!(
        place.projection,
        vec![Projection::Field {
            index: 1,
            identity: Some(crate::mir::MirFieldIdentity {
                owner,
                field_id: crate::ids::FieldId(1),
            }),
        }]
    );
}

#[test]
fn lower_place_field_projection_requires_field_sidecar() {
    let program = program_with_struct(vec!["a", "b"]);
    let type_context = test_type_context(&program);
    let mut builder = builder_with_var(&program, &type_context, "x", struct_ty(0));
    let place_expr = expr(
        HirExprKind::FieldAccess(
            Box::new(expr(HirExprKind::Var("x".to_string()), struct_ty(0))),
            "b".to_string(),
            None,
        ),
        Type::I64,
    );

    assert!(builder.lower_place(&place_expr).is_none());
}

#[test]
fn test_lower_place_field_projection_uses_owned_struct_when_index_is_stale() {
    let mut program = program_with_struct(vec!["a", "b"]);
    program
        .indexes
        .structs_by_id
        .remove(&DefId::new(CrateId(0), LocalDefId(0)));
    let type_context = test_type_context(&program);
    let mut builder = builder_with_var(&program, &type_context, "x", struct_ty(0));
    let owner = program.test_struct_by_name("Foo").unwrap().0;
    let place_expr = expr(
        HirExprKind::FieldAccess(
            Box::new(expr(HirExprKind::Var("x".to_string()), struct_ty(0))),
            "b".to_string(),
            Some(crate::hir::HirFieldLocation {
                owner,
                field_id: crate::ids::FieldId(1),
                name: "b".to_string(),
            }),
        ),
        Type::I64,
    );

    let place = builder
        .lower_place(&place_expr)
        .expect("expected field place from owned struct");
    assert_eq!(place.local, Local(0));
    assert_eq!(
        place.projection,
        vec![Projection::Field {
            index: 1,
            identity: Some(crate::mir::MirFieldIdentity {
                owner,
                field_id: crate::ids::FieldId(1),
            }),
        }]
    );
}

#[test]
fn test_lower_place_field_projection_ignores_mismatched_field_sidecar() {
    let program = program_with_struct(vec!["a", "b"]);
    let type_context = test_type_context(&program);
    let mut builder = builder_with_var(&program, &type_context, "x", struct_ty(0));
    let owner = program.test_struct_by_name("Foo").unwrap().0;
    let place_expr = expr(
        HirExprKind::FieldAccess(
            Box::new(expr(HirExprKind::Var("x".to_string()), struct_ty(0))),
            "a".to_string(),
            Some(crate::hir::HirFieldLocation {
                owner,
                field_id: crate::ids::FieldId(1),
                name: "b".to_string(),
            }),
        ),
        Type::I64,
    );

    assert!(builder.lower_place(&place_expr).is_none());
}

#[test]
fn test_lower_place_tuple_index_projection() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let mut builder = builder_with_var(
        &program,
        &type_context,
        "t",
        Type::Tuple(vec![Type::I64, Type::Bool]),
    );
    let place_expr = expr(
        HirExprKind::TupleIndex(
            Box::new(expr(
                HirExprKind::Var("t".to_string()),
                Type::Tuple(vec![Type::I64, Type::Bool]),
            )),
            1,
        ),
        Type::Bool,
    );

    let place = builder
        .lower_place(&place_expr)
        .expect("expected tuple place");
    assert_eq!(place.local, Local(0));
    assert_eq!(
        place.projection,
        vec![Projection::Field {
            index: 1,
            identity: None,
        }]
    );
}

#[test]
fn test_lower_place_computed_pointer_deref_projection() {
    let pointee_ty = Type::U8;
    let pointer_ty = Type::Pointer(Box::new(pointee_ty.clone()));
    let program = empty_program();
    let type_context = test_type_context(&program);
    let mut builder = builder_with_var(&program, &type_context, "ptr", pointer_ty.clone());
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    let pointer_offset = expr(
        HirExprKind::BinOp(
            crate::hir::BinOp::Add,
            Box::new(expr(
                HirExprKind::Var("ptr".to_string()),
                pointer_ty.clone(),
            )),
            Box::new(expr(HirExprKind::IntLiteral(1), Type::I64)),
        ),
        pointer_ty.clone(),
    );
    let place_expr = expr(HirExprKind::Deref(Box::new(pointer_offset)), pointee_ty);

    let place = builder
        .lower_place(&place_expr)
        .expect("expected computed pointer deref place");

    assert_eq!(place.projection, vec![Projection::Deref]);
    assert_eq!(
        builder.locals[place.local.0].ty,
        builder.type_id_for(&pointer_ty)
    );
}

#[test]
fn mir_index_mut_lowers_through_instance_reference() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let array_ty = Type::Array(Box::new(Type::I64), 1);
    let index_mut_ref_ty = Type::Reference {
        mutable: true,
        inner: Box::new(Type::I64),
    };
    let instance_id = InstanceId(7);
    let mut builder = builder_with_var(&program, &type_context, "arr", array_ty.clone());
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::Unit),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::Temporary,
    });
    builder.blocks.push(BasicBlock {
        statements: Vec::new(),
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));

    let callee = expr(
        HirExprKind::ResolvedVar(HirVarRef {
            name: "index_mut".to_string(),
            target: HirVarTarget::Instance(instance_id),
        }),
        Type::function(vec![array_ty.clone(), Type::I64], index_mut_ref_ty.clone()),
    );
    let call = expr(
        HirExprKind::Call(
            Box::new(callee),
            vec![
                expr(HirExprKind::Var("arr".to_string()), array_ty.clone()),
                expr(HirExprKind::IntLiteral(0), Type::I64),
            ],
            Some(crate::hir::HirCallTarget::Instance(instance_id)),
        ),
        index_mut_ref_ty,
    );
    let assignment = expr(
        HirExprKind::Assign(
            Box::new(expr(HirExprKind::Deref(Box::new(call)), Type::I64)),
            Box::new(expr(HirExprKind::IntLiteral(42), Type::I64)),
        ),
        Type::Unit,
    );

    builder.lower_expr(
        &assignment,
        Place {
            local: Local(1),
            projection: Vec::new(),
        },
    );

    let (returned_ref_local, returned_ref_is_stored, called_instance) = builder
        .blocks
        .iter()
        .filter_map(|block| block.terminator.as_ref())
        .find_map(|terminator| match terminator {
            Terminator::Call {
                func:
                    Operand::Constant(Constant::Callable(MirCallable::Resolved(
                        MirCallableKey::Instance(instance),
                    ))),
                destination,
                ..
            } => Some((
                destination.local,
                destination.projection.is_empty(),
                *instance,
            )),
            _ => None,
        })
        .expect("IndexMut should lower to an instance call");
    assert_eq!(called_instance, instance_id);
    assert!(returned_ref_is_stored);
    assert_eq!(
        builder.locals[returned_ref_local.0].ty,
        builder.type_id_for(&Type::Reference {
            mutable: true,
            inner: Box::new(Type::I64),
        })
    );

    assert!(builder
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .any(|statement| matches!(
            &statement.kind,
            StatementKind::Assign(
                Place { local, projection },
                Rvalue::Use(_),
            ) if *local == returned_ref_local
                && projection.as_slice() == [Projection::Deref]
        )));
    assert!(!builder
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .any(|statement| matches!(
            &statement.kind,
            StatementKind::Assign(
                Place { projection, .. },
                Rvalue::Use(_),
            ) if projection.iter().any(|projection| matches!(projection, Projection::Index(_)))
        )));
    assert!(!builder.locals.iter().any(|local| matches!(
        builder.type_context.borrow().type_for(local.ty),
        Type::Projection { .. }
    )));
}

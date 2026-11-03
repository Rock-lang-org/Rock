use super::*;

#[test]
fn guarded_enum_payload_binding_gracefully_skips_non_copy_payload() {
    let enum_id = DefId::new(CrateId(0), LocalDefId(40));
    let some_id = VariantId(1);
    let payload_ty = struct_ty(0);
    let program = option_program(vec![(
        some_id,
        "Some",
        HirVariantFields::Positional(vec![payload_ty.clone()]),
    )]);

    let type_context = test_type_context(&program);
    let mut builder = builder_with_var(&program, &type_context, "opt", option_ty());
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&payload_ty.clone()),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });

    let value_expr = expr(HirExprKind::Var("value".to_string()), payload_ty.clone());
    let match_expr = expr(
        HirExprKind::Match {
            scrutinee: Box::new(expr(HirExprKind::Var("opt".to_string()), option_ty())),
            arms: vec![match_arm(
                HirPattern::Enum(
                    "Option".to_string(),
                    "Some".to_string(),
                    Some(HirVariantLocation {
                        owner: enum_id,
                        variant_id: some_id,
                        name: "Some".to_string(),
                    }),
                    vec![HirPattern::Binding {
                        name: "value".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        mutable: false,
                    }],
                ),
                Some(expr(HirExprKind::BoolLiteral(false), Type::Bool)),
                value_expr,
            )],
        },
        payload_ty,
    );

    builder.lower_expr(
        &match_expr,
        Place {
            local: Local(1),
            projection: vec![],
        },
    );
}

#[test]
fn unguarded_enum_payload_binding_moves_non_copy_payload_without_dropping_scrutinee() {
    let enum_id = DefId::new(CrateId(0), LocalDefId(40));
    let some_id = VariantId(1);
    let payload_ty = struct_ty(0);
    let program = option_program(vec![(
        some_id,
        "Some",
        HirVariantFields::Positional(vec![payload_ty.clone()]),
    )]);

    let type_context = test_type_context(&program);
    let mut builder = builder_with_var(&program, &type_context, "opt", option_ty());
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&payload_ty.clone()),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });

    let value_expr = expr(HirExprKind::Var("value".to_string()), payload_ty.clone());
    let match_expr = expr(
        HirExprKind::Match {
            scrutinee: Box::new(expr(HirExprKind::Var("opt".to_string()), option_ty())),
            arms: vec![match_arm(
                HirPattern::Enum(
                    "Option".to_string(),
                    "Some".to_string(),
                    Some(HirVariantLocation {
                        owner: enum_id,
                        variant_id: some_id,
                        name: "Some".to_string(),
                    }),
                    vec![HirPattern::Binding {
                        name: "value".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        mutable: false,
                    }],
                ),
                None,
                value_expr,
            )],
        },
        payload_ty,
    );

    builder.lower_expr(
        &match_expr,
        Place {
            local: Local(1),
            projection: vec![],
        },
    );

    let value_local = builder
        .locals
        .iter()
        .position(|decl| decl.name.as_deref() == Some("value"))
        .map(Local)
        .expect("value binding local");
    let moved_scrutinee = builder
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|stmt| match &stmt.kind {
            StatementKind::Assign(
                Place { local, .. },
                Rvalue::Use(Operand::Move(Place { local: source, .. })),
            ) if *local == value_local => Some(*source),
            _ => None,
        })
        .expect("non-copy payload binding should move from scrutinee temp");
    assert!(!builder.blocks.iter().any(|block| matches!(
        &block.terminator,
        Some(Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. })
            if place.local == moved_scrutinee
    )));
}

#[test]
fn moving_later_enum_arm_does_not_suppress_nonmoving_arm_scrutinee_cleanup() {
    let enum_id = DefId::new(CrateId(0), LocalDefId(40));
    let none_id = VariantId(0);
    let some_id = VariantId(1);
    let payload_ty = struct_ty(0);
    let program = option_program(vec![
        (none_id, "None", HirVariantFields::Unit),
        (
            some_id,
            "Some",
            HirVariantFields::Positional(vec![payload_ty.clone()]),
        ),
    ]);
    let type_context = test_type_context(&program);
    let mut builder = builder_with_var(&program, &type_context, "opt", option_ty());
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

    let value_expr = expr(HirExprKind::Var("value".to_string()), payload_ty.clone());
    let match_expr = expr(
        HirExprKind::Match {
            scrutinee: Box::new(expr(HirExprKind::Var("opt".to_string()), option_ty())),
            arms: vec![
                match_arm(
                    HirPattern::Enum(
                        "Option".to_string(),
                        "None".to_string(),
                        Some(HirVariantLocation {
                            owner: enum_id,
                            variant_id: none_id,
                            name: "None".to_string(),
                        }),
                        vec![],
                    ),
                    None,
                    expr(HirExprKind::IntLiteral(0), Type::I64),
                ),
                match_arm(
                    HirPattern::Enum(
                        "Option".to_string(),
                        "Some".to_string(),
                        Some(HirVariantLocation {
                            owner: enum_id,
                            variant_id: some_id,
                            name: "Some".to_string(),
                        }),
                        vec![HirPattern::Binding {
                            name: "value".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            mutable: false,
                        }],
                    ),
                    None,
                    value_expr,
                ),
            ],
        },
        Type::I64,
    );

    builder.lower_expr(
        &match_expr,
        Place {
            local: Local(1),
            projection: vec![],
        },
    );

    let scrutinee_local = builder
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|stmt| match &stmt.kind {
            StatementKind::Assign(_, Rvalue::Discriminant(Place { local, .. })) => Some(*local),
            _ => None,
        })
        .expect("match discriminant should read scrutinee temp");
    assert!(builder.blocks.iter().any(|block| {
            block
                .statements
                .iter()
                .any(|stmt| matches!(&stmt.kind, StatementKind::StorageDead(local) if *local == scrutinee_local))
        }));
}

#[test]
fn returning_enum_match_arm_cleans_match_temps_before_return() {
    let enum_id = DefId::new(CrateId(0), LocalDefId(40));
    let none_id = VariantId(0);
    let program = option_program(vec![(none_id, "None", HirVariantFields::Unit)]);
    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::I64),
        mutability: Mutability::Mut,
        name: Some("return_place".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&option_ty()),
        mutability: Mutability::Not,
        name: Some("opt".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::I64),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    builder.var_map.insert("opt".to_string(), Local(1));

    let match_expr = expr(
        HirExprKind::Match {
            scrutinee: Box::new(expr(HirExprKind::Var("opt".to_string()), option_ty())),
            arms: vec![HirMatchArm {
                pattern: HirPattern::Enum(
                    "Option".to_string(),
                    "None".to_string(),
                    Some(HirVariantLocation {
                        owner: enum_id,
                        variant_id: none_id,
                        name: "None".to_string(),
                    }),
                    vec![],
                ),
                guard: None,
                body: HirBlock {
                    stmts: vec![HirStmt::Return(Some(expr(
                        HirExprKind::IntLiteral(0),
                        Type::I64,
                    )))],
                    ty: Type::Never,
                },
            }],
        },
        Type::I64,
    );

    builder.lower_expr(
        &match_expr,
        Place {
            local: Local(2),
            projection: vec![],
        },
    );

    let (discr_local, scrutinee_local) = builder
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|stmt| match &stmt.kind {
            StatementKind::Assign(
                Place { local: discr, .. },
                Rvalue::Discriminant(Place {
                    local: scrutinee, ..
                }),
            ) => Some((*discr, *scrutinee)),
            _ => None,
        })
        .expect("match should assign discriminant from scrutinee temp");

    let return_block = builder
        .blocks
        .iter()
        .find(|block| {
            matches!(
                block.terminator,
                Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })
            )
        })
        .expect("return terminator block");
    assert!(return_block.statements.iter().any(
        |stmt| matches!(&stmt.kind, StatementKind::StorageDead(local) if *local == discr_local)
    ));
    assert!(return_block.statements.iter().any(
        |stmt| matches!(&stmt.kind, StatementKind::StorageDead(local) if *local == scrutinee_local)
    ));

    assert_builder_agreement_clean(builder, Type::Unit);
}

#[test]
fn returning_enum_match_guard_cleans_match_temps_before_return() {
    let enum_id = DefId::new(CrateId(0), LocalDefId(40));
    let none_id = VariantId(0);
    let program = option_program(vec![(none_id, "None", HirVariantFields::Unit)]);
    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::I64),
        mutability: Mutability::Mut,
        name: Some("return_place".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&option_ty()),
        mutability: Mutability::Not,
        name: Some("opt".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::I64),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    builder.var_map.insert("opt".to_string(), Local(1));

    let match_expr = expr(
        HirExprKind::Match {
            scrutinee: Box::new(expr(HirExprKind::Var("opt".to_string()), option_ty())),
            arms: vec![HirMatchArm {
                pattern: HirPattern::Enum(
                    "Option".to_string(),
                    "None".to_string(),
                    Some(HirVariantLocation {
                        owner: enum_id,
                        variant_id: none_id,
                        name: "None".to_string(),
                    }),
                    vec![],
                ),
                guard: Some(expr(
                    HirExprKind::Block(HirBlock {
                        stmts: vec![HirStmt::Return(Some(expr(
                            HirExprKind::IntLiteral(0),
                            Type::I64,
                        )))],
                        ty: Type::Bool,
                    }),
                    Type::Bool,
                )),
                body: HirBlock {
                    stmts: vec![HirStmt::Expr(expr(HirExprKind::IntLiteral(1), Type::I64))],
                    ty: Type::I64,
                },
            }],
        },
        Type::I64,
    );

    builder.lower_expr(
        &match_expr,
        Place {
            local: Local(2),
            projection: vec![],
        },
    );

    let (discr_local, scrutinee_local) = builder
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|stmt| match &stmt.kind {
            StatementKind::Assign(
                Place { local: discr, .. },
                Rvalue::Discriminant(Place {
                    local: scrutinee, ..
                }),
            ) => Some((*discr, *scrutinee)),
            _ => None,
        })
        .expect("match should assign discriminant from scrutinee temp");

    let return_block = builder
        .blocks
        .iter()
        .find(|block| {
            matches!(
                block.terminator,
                Some(Terminator::Return | Terminator::ReturnWithOrigin { .. })
            )
        })
        .expect("return terminator block");
    assert!(return_block.statements.iter().any(
        |stmt| matches!(&stmt.kind, StatementKind::StorageDead(local) if *local == discr_local)
    ));
    assert!(return_block.statements.iter().any(
        |stmt| matches!(&stmt.kind, StatementKind::StorageDead(local) if *local == scrutinee_local)
    ));

    assert_builder_agreement_clean(builder, Type::Unit);
}

#[test]
fn generic_enum_payload_binding_moves_without_dropping_scrutinee() {
    let enum_id = DefId::new(CrateId(0), LocalDefId(40));
    let some_id = VariantId(1);
    let payload_ty = Type::Generic(crate::types::GenericParamId {
        owner: enum_id,
        index: 0,
    });
    let option_ty = Type::Enum {
        id: enum_id,
        args: vec![payload_ty.clone()],
    };
    let program = option_program(vec![(
        some_id,
        "Some",
        HirVariantFields::Positional(vec![payload_ty.clone()]),
    )]);

    let type_context = test_type_context(&program);
    let mut builder = builder_with_var(&program, &type_context, "opt", option_ty.clone());
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&payload_ty.clone()),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });

    let value_expr = expr(HirExprKind::Var("value".to_string()), payload_ty.clone());
    let match_expr = expr(
        HirExprKind::Match {
            scrutinee: Box::new(expr(HirExprKind::Var("opt".to_string()), option_ty)),
            arms: vec![match_arm(
                HirPattern::Enum(
                    "Option".to_string(),
                    "Some".to_string(),
                    Some(HirVariantLocation {
                        owner: enum_id,
                        variant_id: some_id,
                        name: "Some".to_string(),
                    }),
                    vec![HirPattern::Binding {
                        name: "value".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        mutable: false,
                    }],
                ),
                None,
                value_expr,
            )],
        },
        payload_ty,
    );

    builder.lower_expr(
        &match_expr,
        Place {
            local: Local(1),
            projection: vec![],
        },
    );

    let value_local = builder
        .locals
        .iter()
        .position(|decl| decl.name.as_deref() == Some("value"))
        .map(Local)
        .expect("value binding local");
    let moved_scrutinee = builder
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|stmt| match &stmt.kind {
            StatementKind::Assign(
                Place { local, .. },
                Rvalue::Use(Operand::Move(Place { local: source, .. })),
            ) if *local == value_local => Some(*source),
            _ => None,
        })
        .expect("generic payload binding should move from scrutinee temp");
    assert!(!builder.blocks.iter().any(|block| matches!(
        &block.terminator,
        Some(Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. })
            if place.local == moved_scrutinee
    )));

    let report = builder_agreement_report_with_contract(builder, Type::Unit, Default::default());
    assert!(report.invalid_type_ids > 0);
}

#[test]
fn function_generic_enum_payload_binding_moves_without_dropping_scrutinee() {
    let enum_id = DefId::new(CrateId(0), LocalDefId(40));
    let function_id = DefId::new(CrateId(0), LocalDefId(41));
    let some_id = VariantId(1);
    let payload_ty = Type::Generic(crate::types::GenericParamId {
        owner: function_id,
        index: 0,
    });
    let option_ty = Type::Enum {
        id: enum_id,
        args: vec![payload_ty.clone()],
    };
    let program = option_program(vec![(
        some_id,
        "Some",
        HirVariantFields::Positional(vec![payload_ty.clone()]),
    )]);

    let type_context = test_type_context(&program);
    let mut builder = builder_with_var(&program, &type_context, "opt", option_ty.clone());
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&payload_ty.clone()),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });

    let value_expr = expr(HirExprKind::Var("value".to_string()), payload_ty.clone());
    let match_expr = expr(
        HirExprKind::Match {
            scrutinee: Box::new(expr(HirExprKind::Var("opt".to_string()), option_ty)),
            arms: vec![match_arm(
                HirPattern::Enum(
                    "Option".to_string(),
                    "Some".to_string(),
                    Some(HirVariantLocation {
                        owner: enum_id,
                        variant_id: some_id,
                        name: "Some".to_string(),
                    }),
                    vec![HirPattern::Binding {
                        name: "value".to_string(),
                        local_id: crate::ids::HirLocalId(0),
                        mutable: false,
                    }],
                ),
                None,
                value_expr,
            )],
        },
        payload_ty,
    );

    builder.lower_expr(
        &match_expr,
        Place {
            local: Local(1),
            projection: vec![],
        },
    );

    let value_local = builder
        .locals
        .iter()
        .position(|decl| decl.name.as_deref() == Some("value"))
        .map(Local)
        .expect("value binding local");
    let moved_scrutinee = builder
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|stmt| match &stmt.kind {
            StatementKind::Assign(
                Place { local, .. },
                Rvalue::Use(Operand::Move(Place { local: source, .. })),
            ) if *local == value_local => Some(*source),
            _ => None,
        })
        .expect("function generic payload binding should move from scrutinee temp");
    assert!(!builder.blocks.iter().any(|block| matches!(
        &block.terminator,
        Some(Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. })
            if place.local == moved_scrutinee
    )));

    let report = builder_agreement_report_with_contract(builder, Type::Unit, Default::default());
    assert!(report.invalid_type_ids > 0);
}

use super::*;

#[test]
fn enum_match_lowers_to_discriminant_switch() {
    let enum_id = DefId::new(CrateId(0), LocalDefId(40));
    let none_id = VariantId(0);
    let some_id = VariantId(1);
    let program = option_program(vec![
        (none_id, "None", HirVariantFields::Unit),
        (some_id, "Some", HirVariantFields::Unit),
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
                        vec![],
                    ),
                    None,
                    expr(HirExprKind::IntLiteral(1), Type::I64),
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

    let switch_targets: Vec<i64> = builder
        .blocks
        .iter()
        .filter_map(|block| match &block.terminator {
            Some(
                Terminator::SwitchInt { targets, .. }
                | Terminator::SwitchIntWithOrigin { targets, .. },
            ) => Some(targets),
            _ => None,
        })
        .flatten()
        .map(|(value, _)| *value)
        .collect();
    assert!(switch_targets.contains(&(none_id.0 as i64)));
    assert!(switch_targets.contains(&(some_id.0 as i64)));
    assert!(builder
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .any(|stmt| {
            matches!(
                &stmt.kind,
                StatementKind::Assign(_, Rvalue::Discriminant(_))
            )
        }));

    assert_builder_agreement_clean(builder, Type::Unit);
}

#[test]
fn guarded_enum_arm_does_not_make_final_enum_arm_exhaustive() {
    let enum_id = DefId::new(CrateId(0), LocalDefId(40));
    let none_id = VariantId(0);
    let some_id = VariantId(1);
    let program = option_program(vec![
        (none_id, "None", HirVariantFields::Unit),
        (some_id, "Some", HirVariantFields::Unit),
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
                    Some(expr(HirExprKind::BoolLiteral(false), Type::Bool)),
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
                        vec![],
                    ),
                    None,
                    expr(HirExprKind::IntLiteral(1), Type::I64),
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

    let (matched_block, otherwise) = builder
        .blocks
        .iter()
        .find_map(|block| match &block.terminator {
            Some(
                Terminator::SwitchInt {
                    targets, otherwise, ..
                }
                | Terminator::SwitchIntWithOrigin {
                    targets, otherwise, ..
                },
            ) => targets
                .iter()
                .find(|(value, _)| *value == some_id.0 as i64)
                .map(|(_, matched)| (*matched, *otherwise)),
            _ => None,
        })
        .expect("final enum arm discriminant switch");
    assert_ne!(otherwise, matched_block);

    assert_builder_agreement_clean(builder, Type::Unit);
}

#[test]
fn enum_match_payload_binding_uses_downcast_field_projection() {
    let enum_id = DefId::new(CrateId(0), LocalDefId(40));
    let some_id = VariantId(1);
    let program = option_program(vec![(
        some_id,
        "Some",
        HirVariantFields::Positional(vec![Type::I64]),
    )]);
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

    let value_expr = expr(HirExprKind::Var("value".to_string()), Type::I64);
    let guard = expr(
        HirExprKind::BinOp(
            crate::hir::BinOp::Gt,
            Box::new(value_expr.clone()),
            Box::new(expr(HirExprKind::IntLiteral(0), Type::I64)),
        ),
        Type::Bool,
    );
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
                Some(guard),
                value_expr,
            )],
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

    let value_local = builder
        .locals
        .iter()
        .position(|decl| decl.name.as_deref() == Some("value"))
        .map(Local)
        .expect("value binding local");
    assert!(builder
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .any(|stmt| {
            matches!(
                &stmt.kind,
                StatementKind::Assign(
                    Place { local, .. },
                    Rvalue::Use(Operand::Copy(Place { projection, .. }))
                ) if *local == value_local
                    && projection.contains(&Projection::Downcast(some_id))
                    && projection.contains(&Projection::Field { index: 0, identity: None })
            )
        }));
    assert!(builder.blocks.iter().any(|block| matches!(
        &block.terminator,
        Some(
            Terminator::SwitchInt {
                discr: Operand::Copy(_),
                ..
            } | Terminator::SwitchIntWithOrigin {
                discr: Operand::Copy(_),
                ..
            },
        )
    )));

    let bool_id = builder.type_id_for(&Type::Bool);
    let guard_false_cleanup = builder
        .blocks
        .iter()
        .find_map(|block| match &block.terminator {
            Some(
                Terminator::SwitchInt {
                    discr: Operand::Copy(place),
                    targets,
                    otherwise,
                }
                | Terminator::SwitchIntWithOrigin {
                    discr: Operand::Copy(place),
                    targets,
                    otherwise,
                    ..
                },
            ) if builder.locals[place.local.0].ty == bool_id
                && targets.contains(&(1, *otherwise)) == false =>
            {
                Some(*otherwise)
            }
            _ => None,
        })
        .expect("guard false cleanup block");
    assert!(builder.blocks[guard_false_cleanup.0].statements.iter().any(
        |stmt| matches!(&stmt.kind, StatementKind::StorageDead(local) if *local == value_local)
    ));

    assert_builder_agreement_clean(builder, Type::Unit);
}

#[test]
fn literal_match_uses_value_comparison_not_discriminant() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let mut builder = builder_with_var(&program, &type_context, "n", Type::I64);
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
    let match_expr = expr(
        HirExprKind::Match {
            scrutinee: Box::new(expr(HirExprKind::Var("n".to_string()), Type::I64)),
            arms: vec![
                match_arm(
                    HirPattern::Literal(HirLiteralPattern::Int(0)),
                    None,
                    expr(HirExprKind::IntLiteral(1), Type::I64),
                ),
                match_arm(
                    HirPattern::Wildcard,
                    None,
                    expr(HirExprKind::IntLiteral(2), Type::I64),
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

    assert!(!builder
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .any(|stmt| {
            matches!(
                &stmt.kind,
                StatementKind::Assign(_, Rvalue::Discriminant(_))
            )
        }));
    assert!(builder
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .any(|stmt| {
            matches!(
                &stmt.kind,
                StatementKind::Assign(
                    _,
                    Rvalue::BinaryOp(
                        crate::mir::MirBinOp::Eq,
                        _,
                        Operand::Constant(Constant::Int(0))
                    )
                )
            )
        }));
}

#[test]
fn enum_match_literal_payload_pattern_lowers_payload_check_before_body() {
    let enum_id = DefId::new(CrateId(0), LocalDefId(40));
    let none_id = VariantId(0);
    let some_id = VariantId(1);
    let program = option_program(vec![
        (none_id, "None", HirVariantFields::Unit),
        (
            some_id,
            "Some",
            HirVariantFields::Positional(vec![Type::I64]),
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

    let match_expr = expr(
        HirExprKind::Match {
            scrutinee: Box::new(expr(HirExprKind::Var("opt".to_string()), option_ty())),
            arms: vec![
                match_arm(
                    HirPattern::Enum(
                        "Option".to_string(),
                        "Some".to_string(),
                        Some(HirVariantLocation {
                            owner: enum_id,
                            variant_id: some_id,
                            name: "Some".to_string(),
                        }),
                        vec![HirPattern::Literal(HirLiteralPattern::Int(0))],
                    ),
                    None,
                    expr(HirExprKind::IntLiteral(1), Type::I64),
                ),
                match_arm(
                    HirPattern::Wildcard,
                    None,
                    expr(HirExprKind::IntLiteral(2), Type::I64),
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

    let payload_check = builder
        .blocks
        .iter()
        .enumerate()
        .find_map(|(block_index, block)| {
            block.statements.iter().find_map(|stmt| match &stmt.kind {
                StatementKind::Assign(
                    bool_place,
                    Rvalue::BinaryOp(
                        crate::mir::MirBinOp::Eq,
                        Operand::Copy(Place { projection, .. }),
                        Operand::Constant(Constant::Int(0)),
                    ),
                ) if projection.contains(&Projection::Downcast(some_id))
                    && projection.contains(&Projection::Field {
                        index: 0,
                        identity: None,
                    }) =>
                {
                    Some((BasicBlockId(block_index), bool_place.clone()))
                }
                _ => None,
            })
        })
        .expect("payload literal comparison");
    let (payload_check_block, payload_bool_place) = payload_check;
    let discriminant_success = builder
        .blocks
        .iter()
        .find_map(|block| match &block.terminator {
            Some(
                Terminator::SwitchInt { targets, .. }
                | Terminator::SwitchIntWithOrigin { targets, .. },
            ) => targets
                .iter()
                .find_map(|(value, target)| (*value == some_id.0 as i64).then_some(*target)),
            _ => None,
        })
        .expect("discriminant Some target");
    assert_eq!(discriminant_success, payload_check_block);

    let body_entry = match &builder.blocks[payload_check_block.0].terminator {
        Some(
            Terminator::SwitchInt {
                discr: Operand::Copy(discr),
                targets,
                otherwise,
            }
            | Terminator::SwitchIntWithOrigin {
                discr: Operand::Copy(discr),
                targets,
                otherwise,
                ..
            },
        ) if *discr == payload_bool_place => (
            targets
                .iter()
                .find_map(|(value, target)| (*value == 1).then_some(*target))
                .expect("payload comparison success target"),
            *otherwise,
        ),
        terminator => panic!("expected payload bool SwitchInt, got {terminator:?}"),
    };
    let (payload_success_entry, payload_failure_entry) = body_entry;
    let body_block = match &builder.blocks[payload_success_entry.0].terminator {
        Some(Terminator::Goto(target) | Terminator::GotoWithOrigin { target, .. }) => *target,
        terminator => panic!("expected payload success to enter body, got {terminator:?}"),
    };
    let fallback_matched_block = match &builder.blocks[payload_failure_entry.0].terminator {
        Some(Terminator::Goto(target) | Terminator::GotoWithOrigin { target, .. }) => *target,
        terminator => panic!("expected payload failure to reach fallback, got {terminator:?}"),
    };
    let fallback_body_block = match &builder.blocks[fallback_matched_block.0].terminator {
        Some(Terminator::Goto(target) | Terminator::GotoWithOrigin { target, .. }) => *target,
        terminator => {
            panic!("expected fallback matched block to enter body, got {terminator:?}")
        }
    };
    assert!(builder.blocks[body_block.0].statements.iter().any(|stmt| {
        matches!(
            &stmt.kind,
            StatementKind::Assign(
                Place {
                    local: Local(1),
                    projection,
                },
                Rvalue::Use(Operand::Constant(Constant::Int(1)))
            ) if projection.is_empty()
        )
    }));
    assert!(builder.blocks[fallback_body_block.0]
        .statements
        .iter()
        .any(|stmt| {
            matches!(
                &stmt.kind,
                StatementKind::Assign(
                    Place {
                        local: Local(1),
                        projection,
                    },
                    Rvalue::Use(Operand::Constant(Constant::Int(2)))
                ) if projection.is_empty()
            )
        }));

    assert_builder_agreement_clean(builder, Type::Unit);
}

#[test]
#[should_panic(expected = "unsupported refutable enum payload pattern")]
fn enum_match_string_payload_pattern_panics_loudly() {
    let enum_id = DefId::new(CrateId(0), LocalDefId(40));
    let some_id = VariantId(1);
    let program = option_program(vec![(
        some_id,
        "Some",
        HirVariantFields::Positional(vec![Type::Str]),
    )]);
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

    let match_expr = expr(
        HirExprKind::Match {
            scrutinee: Box::new(expr(HirExprKind::Var("opt".to_string()), option_ty())),
            arms: vec![
                match_arm(
                    HirPattern::Enum(
                        "Option".to_string(),
                        "Some".to_string(),
                        Some(HirVariantLocation {
                            owner: enum_id,
                            variant_id: some_id,
                            name: "Some".to_string(),
                        }),
                        vec![HirPattern::Literal(HirLiteralPattern::String(
                            "x".to_string(),
                        ))],
                    ),
                    None,
                    expr(HirExprKind::IntLiteral(1), Type::I64),
                ),
                match_arm(
                    HirPattern::Wildcard,
                    None,
                    expr(HirExprKind::IntLiteral(2), Type::I64),
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
}

#[test]
#[should_panic(expected = "unsupported refutable enum payload pattern")]
fn enum_match_float_payload_pattern_panics_loudly() {
    let enum_id = DefId::new(CrateId(0), LocalDefId(40));
    let some_id = VariantId(1);
    let program = option_program(vec![(
        some_id,
        "Some",
        HirVariantFields::Positional(vec![Type::F32]),
    )]);
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

    let match_expr = expr(
        HirExprKind::Match {
            scrutinee: Box::new(expr(HirExprKind::Var("opt".to_string()), option_ty())),
            arms: vec![
                match_arm(
                    HirPattern::Enum(
                        "Option".to_string(),
                        "Some".to_string(),
                        Some(HirVariantLocation {
                            owner: enum_id,
                            variant_id: some_id,
                            name: "Some".to_string(),
                        }),
                        vec![HirPattern::Literal(HirLiteralPattern::Float(1.5))],
                    ),
                    None,
                    expr(HirExprKind::IntLiteral(1), Type::I64),
                ),
                match_arm(
                    HirPattern::Wildcard,
                    None,
                    expr(HirExprKind::IntLiteral(2), Type::I64),
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
}

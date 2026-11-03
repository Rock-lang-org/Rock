use super::*;

#[test]
fn test_lower_struct_literal_orders_operands_by_field_id() {
    let program = program_with_struct(vec!["a", "b"]);
    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&struct_ty(0)),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    let owner = program.test_struct_by_name("Foo").unwrap().0;
    let literal = expr(
        HirExprKind::StructLiteral(
            "Foo".to_string(),
            None,
            vec![
                HirStructLiteralField {
                    name: "b".to_string(),
                    value: expr(HirExprKind::IntLiteral(2), Type::I64),
                    field: Some(crate::hir::HirFieldLocation {
                        owner,
                        field_id: FieldId(1),
                        name: "b".to_string(),
                    }),
                },
                HirStructLiteralField {
                    name: "a".to_string(),
                    value: expr(HirExprKind::IntLiteral(1), Type::I64),
                    field: Some(crate::hir::HirFieldLocation {
                        owner,
                        field_id: FieldId(0),
                        name: "a".to_string(),
                    }),
                },
            ],
        ),
        struct_ty(0),
    );

    builder.lower_expr(
        &literal,
        Place {
            local: Local(0),
            projection: vec![],
        },
    );

    let Some(StatementData {
        kind:
            StatementKind::Assign(
                _,
                Rvalue::Aggregate(
                    AggregateKind::Struct {
                        id: struct_id,
                        display_name,
                    },
                    operands,
                ),
            ),
        ..
    }) = builder.blocks[0].statements.last()
    else {
        panic!("expected struct aggregate assignment");
    };

    assert_eq!(*struct_id, owner);
    assert_eq!(display_name, "Foo");
    assert!(matches!(
        operands[0],
        Operand::Copy(Place {
            local: Local(2),
            ..
        })
    ));
    assert!(matches!(
        operands[1],
        Operand::Copy(Place {
            local: Local(1),
            ..
        })
    ));
}

#[test]
fn enum_variant_lowers_to_canonical_aggregate_with_payloads() {
    let enum_id = DefId::new(CrateId(0), LocalDefId(30));
    let variant_id = crate::ids::VariantId(2);
    let mut enums = std::collections::HashMap::new();
    enums.insert(
        enum_id,
        crate::hir::HirEnum {
            id: enum_id,
            name: "Option".to_string(),
            generic_params: vec![],
            variants: vec![crate::hir::HirVariant {
                id: variant_id,
                name: "Some".to_string(),
                fields: crate::hir::HirVariantFields::Positional(vec![Type::I64]),
            }],
        },
    );
    let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        enums,
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        HirNameTables {
            enums_by_name: std::collections::HashMap::from([("Option".to_string(), enum_id)]),
            ..HirNameTables::default()
        },
        &std::collections::HashMap::from([(enum_id, "Option".to_string())]),
    );
    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::Enum {
            id: enum_id,
            args: vec![],
        }),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    let expr = expr(
        HirExprKind::EnumVariant(
            "Option".to_string(),
            "Some".to_string(),
            vec![expr(HirExprKind::IntLiteral(9), Type::I64)],
            Some(crate::hir::HirVariantLocation {
                owner: enum_id,
                variant_id,
                name: "Some".to_string(),
            }),
        ),
        Type::Enum {
            id: enum_id,
            args: vec![],
        },
    );

    builder.lower_expr(
        &expr,
        Place {
            local: Local(0),
            projection: vec![],
        },
    );

    assert!(builder.blocks[0].statements.iter().any(|stmt| matches!(
        &stmt.kind,
        StatementKind::Assign(
            _,
            Rvalue::Aggregate(
                AggregateKind::EnumVariant {
                    enum_id: found_enum,
                    variant_id: found_variant,
                    enum_name,
                    variant_name,
                },
                operands,
            ),
        ) if *found_enum == enum_id
            && *found_variant == variant_id
            && enum_name == "Option"
            && variant_name == "Some"
            && operands.len() == 1
    )));

    assert_builder_agreement_clean(builder, Type::Unit);
}

#[test]
fn struct_field_access_projection_carries_field_identity() {
    let struct_id = DefId::new(CrateId(0), LocalDefId(31));
    let field_id = crate::ids::FieldId(4);
    let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::new(),
        std::collections::HashMap::from([(
            struct_id,
            crate::hir::HirStruct {
                id: struct_id,
                name: "Point".to_string(),
                generic_params: vec![],
                fields: vec![crate::hir::HirField {
                    id: field_id,
                    name: "x".to_string(),
                    ty: Type::I64,
                    public: true,
                }],
            },
        )]),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        HirNameTables {
            structs_by_name: std::collections::HashMap::from([("Point".to_string(), struct_id)]),
            ..HirNameTables::default()
        },
        &std::collections::HashMap::from([(struct_id, "Point".to_string())]),
    );
    let type_context = test_type_context(&program);
    let mut builder = builder_with_var(
        &program,
        &type_context,
        "p",
        Type::Struct {
            id: struct_id,
            args: vec![],
        },
    );
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
    let field = expr(
        HirExprKind::FieldAccess(
            Box::new(expr(
                HirExprKind::Var("p".to_string()),
                Type::Struct {
                    id: struct_id,
                    args: vec![],
                },
            )),
            "x".to_string(),
            Some(crate::hir::HirFieldLocation {
                owner: struct_id,
                field_id,
                name: "x".to_string(),
            }),
        ),
        Type::I64,
    );

    builder.lower_expr(
        &field,
        Place {
            local: Local(1),
            projection: vec![],
        },
    );

    assert!(builder.blocks[0].statements.iter().any(|stmt| matches!(
        &stmt.kind,
        StatementKind::Assign(
            _,
            Rvalue::Use(Operand::Copy(Place { projection, .. }))
        ) if projection.iter().any(|projection| matches!(
            projection,
            Projection::Field {
                index: 0,
                identity: Some(identity),
            } if identity.owner == struct_id && identity.field_id == field_id
        ))
    )));
}

#[test]
fn non_place_struct_field_access_uses_canonical_id_and_field_identity() {
    let mut program = program_with_struct(vec!["a", "b"]);
    let struct_id = program.test_struct_by_name("Foo").unwrap().0;
    program.indexes.structs_by_id.remove(&struct_id);
    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.scope_locals.push(Vec::new());
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::I64),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    let field = expr(
        HirExprKind::FieldAccess(
            Box::new(expr(
                HirExprKind::StructLiteral(
                    "Foo".to_string(),
                    Some(struct_id),
                    vec![
                        HirStructLiteralField {
                            name: "a".to_string(),
                            value: expr(HirExprKind::IntLiteral(1), Type::I64),
                            field: Some(crate::hir::HirFieldLocation {
                                owner: struct_id,
                                field_id: FieldId(0),
                                name: "a".to_string(),
                            }),
                        },
                        HirStructLiteralField {
                            name: "b".to_string(),
                            value: expr(HirExprKind::IntLiteral(2), Type::I64),
                            field: Some(crate::hir::HirFieldLocation {
                                owner: struct_id,
                                field_id: FieldId(1),
                                name: "b".to_string(),
                            }),
                        },
                    ],
                ),
                struct_ty(0),
            )),
            "b".to_string(),
            Some(crate::hir::HirFieldLocation {
                owner: struct_id,
                field_id: FieldId(1),
                name: "b".to_string(),
            }),
        ),
        Type::I64,
    );

    builder.lower_expr(
        &field,
        Place {
            local: Local(0),
            projection: vec![],
        },
    );

    assert!(builder.blocks[0].statements.iter().any(|stmt| matches!(
        &stmt.kind,
        StatementKind::Assign(
            _,
            Rvalue::Use(Operand::Copy(Place { projection, .. }))
        ) if projection.iter().any(|projection| matches!(
            projection,
            Projection::Field {
                index: 1,
                identity: Some(identity),
            } if identity.owner == struct_id && identity.field_id == FieldId(1)
        ))
    )));
}

#[test]
#[should_panic(expected = "struct literal MIR lowering requires HIR field location")]
fn struct_literal_missing_field_sidecar_panics() {
    let program = program_with_struct(vec!["a", "b"]);
    let struct_id = program.test_struct_by_name("Foo").unwrap().0;
    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&struct_ty(0)),
        mutability: Mutability::Not,
        name: Some("dest".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    let literal = expr(
        HirExprKind::StructLiteral(
            "Foo".to_string(),
            Some(struct_id),
            vec![HirStructLiteralField {
                name: "a".to_string(),
                value: expr(HirExprKind::IntLiteral(1), Type::I64),
                field: None,
            }],
        ),
        struct_ty(0),
    );

    builder.lower_expr(
        &literal,
        Place {
            local: Local(0),
            projection: vec![],
        },
    );
}

#[test]
fn test_lower_expr_field_access_moves_non_copy_value() {
    let mut structs = std::collections::HashMap::new();
    let wrapper_id = DefId::new(CrateId(0), LocalDefId(0));
    structs.insert(
        wrapper_id,
        HirStruct {
            id: wrapper_id,
            name: "Wrapper".to_string(),
            generic_params: vec![],
            fields: vec![HirField {
                id: FieldId(0),
                name: "inner".to_string(),
                ty: struct_ty(1),
                public: true,
            }],
        },
    );
    let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        std::collections::HashMap::new(),
        structs,
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        HirNameTables {
            structs_by_name: std::collections::HashMap::from([("Wrapper".to_string(), wrapper_id)]),
            ..HirNameTables::default()
        },
        &std::collections::HashMap::from([(wrapper_id, "Wrapper".to_string())]),
    );
    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&struct_ty(0)),
        mutability: Mutability::Not,
        name: Some("w".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    builder.var_map.insert("w".to_string(), Local(0));
    let dest = Place {
        local: Local(0),
        projection: vec![],
    };
    let field_expr = expr(
        HirExprKind::FieldAccess(
            Box::new(expr(HirExprKind::Var("w".to_string()), struct_ty(0))),
            "inner".to_string(),
            Some(crate::hir::HirFieldLocation {
                owner: DefId::new(CrateId(0), LocalDefId(0)),
                field_id: FieldId(0),
                name: "inner".to_string(),
            }),
        ),
        struct_ty(1),
    );

    builder.lower_expr_with_context(&field_expr, dest, false);
    assert!(builder.blocks[0].statements.iter().any(|stmt| matches!(
        &stmt.kind,
        StatementKind::Assign(_, Rvalue::Use(Operand::Move(_)))
    )));
}

#[test]
fn test_lower_expr_field_access_from_non_place_base_avoids_unit_fallback() {
    let program = program_with_struct(vec!["a", "b"]);
    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.scope_locals.push(Vec::new());
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::I64),
        mutability: Mutability::Not,
        name: Some("dest_value".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    let dest = Place {
        local: Local(0),
        projection: vec![],
    };
    let owner = program.test_struct_by_name("Foo").unwrap().0;
    let field_expr = expr(
        HirExprKind::FieldAccess(
            Box::new(expr(
                HirExprKind::StructLiteral(
                    "Foo".to_string(),
                    None,
                    vec![
                        HirStructLiteralField {
                            name: "a".to_string(),
                            value: expr(HirExprKind::IntLiteral(1), Type::I64),
                            field: Some(crate::hir::HirFieldLocation {
                                owner,
                                field_id: FieldId(0),
                                name: "a".to_string(),
                            }),
                        },
                        HirStructLiteralField {
                            name: "b".to_string(),
                            value: expr(HirExprKind::IntLiteral(2), Type::I64),
                            field: Some(crate::hir::HirFieldLocation {
                                owner,
                                field_id: FieldId(1),
                                name: "b".to_string(),
                            }),
                        },
                    ],
                ),
                struct_ty(0),
            )),
            "b".to_string(),
            Some(crate::hir::HirFieldLocation {
                owner,
                field_id: FieldId(1),
                name: "b".to_string(),
            }),
        ),
        Type::I64,
    );

    builder.lower_expr_with_context(&field_expr, dest, false);
    assert!(builder.blocks[0].statements.iter().any(|stmt| matches!(
        &stmt.kind,
        StatementKind::Assign(_, Rvalue::Use(Operand::Copy(place)))
            if place.projection.contains(&Projection::Field {
                index: 1,
                identity: Some(crate::mir::MirFieldIdentity {
                    owner,
                    field_id: FieldId(1),
                }),
            })
    )));
    assert!(!builder.blocks[0].statements.iter().any(|stmt| matches!(
        &stmt.kind,
        StatementKind::Assign(
            _,
            Rvalue::Use(Operand::Constant(crate::mir::Constant::Unit))
        )
    )));
    assert_eq!(
        builder.scope_locals.last().map(|locals| locals.len()),
        Some(1)
    );
}

#[test]
fn test_lower_expr_tuple_index_from_non_place_base_avoids_unit_fallback() {
    let program = empty_program();
    let type_context = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &type_context);
    builder.blocks.push(BasicBlock {
        statements: vec![],
        terminator: None,
    });
    builder.current_block = Some(BasicBlockId(0));
    builder.scope_locals.push(Vec::new());
    builder.locals.push(LocalDecl {
        ty: builder.type_id_for(&Type::Bool),
        mutability: Mutability::Not,
        name: Some("dest_value".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    let dest = Place {
        local: Local(0),
        projection: vec![],
    };
    let tuple_expr = expr(
        HirExprKind::TupleIndex(
            Box::new(expr(
                HirExprKind::TupleLiteral(vec![
                    expr(HirExprKind::IntLiteral(1), Type::I64),
                    expr(HirExprKind::BoolLiteral(true), Type::Bool),
                ]),
                Type::Tuple(vec![Type::I64, Type::Bool]),
            )),
            1,
        ),
        Type::Bool,
    );

    builder.lower_expr_with_context(&tuple_expr, dest, false);
    assert!(builder.blocks[0].statements.iter().any(|stmt| matches!(
        &stmt.kind,
        StatementKind::Assign(_, Rvalue::Use(Operand::Copy(place)))
            if place.projection.contains(&Projection::Field {
                index: 1,
                identity: None,
            })
    )));
    assert!(!builder.blocks[0].statements.iter().any(|stmt| matches!(
        &stmt.kind,
        StatementKind::Assign(
            _,
            Rvalue::Use(Operand::Constant(crate::mir::Constant::Unit))
        )
    )));
    assert_eq!(
        builder.scope_locals.last().map(|locals| locals.len()),
        Some(1)
    );
}

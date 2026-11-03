use super::*;

#[test]
fn type_id_contains_reference_matches_structural_pointer_and_projection_behavior() {
    let trait_id = DefId::new(CrateId(0), LocalDefId(10));
    let reference = Type::Reference {
        mutable: false,
        inner: Box::new(Type::I64),
    };
    let cases = vec![
        Type::Pointer(Box::new(reference.clone())),
        Type::Projection {
            ty: Box::new(reference.clone()),
            trait_id,
            assoc_type: assoc_type(trait_id),
            trait_args: vec![],
        },
        Type::Projection {
            ty: Box::new(Type::I64),
            trait_id,
            assoc_type: assoc_type(trait_id),
            trait_args: vec![reference],
        },
    ];

    let mut context = TypeContext::new();
    for ty in cases {
        let id = context.intern_type(&ty);
        let view = crate::type_context::TypeView::new(&context);

        assert_eq!(
            type_id_contains_reference(view, id),
            TypeFacts::contains_reference(&ty)
        );
    }
}

fn test_function_with_reference_copy(type_context: &mut TypeContext) -> MirFunction {
    MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0))),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![
                StatementData::assign(
                    Place {
                        local: Local(1),
                        projection: vec![],
                    },
                    Rvalue::Ref(
                        Mutability::Not,
                        Place {
                            local: Local(3),
                            projection: vec![],
                        },
                    ),
                    None,
                ),
                StatementData::assign(
                    Place {
                        local: Local(2),
                        projection: vec![],
                    },
                    Rvalue::Use(Operand::Copy(Place {
                        local: Local(1),
                        projection: vec![],
                    })),
                    None,
                ),
                StatementData::assign(
                    Place {
                        local: Local(0),
                        projection: vec![],
                    },
                    Rvalue::Use(Operand::Copy(Place {
                        local: Local(2),
                        projection: vec![],
                    })),
                    None,
                ),
            ],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: type_id(
                    type_context,
                    Type::Reference {
                        mutable: false,
                        inner: Box::new(Type::I64),
                    },
                ),
                mutability: Mutability::Not,
                name: Some("ret".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: type_id(
                    type_context,
                    Type::Reference {
                        mutable: false,
                        inner: Box::new(Type::I64),
                    },
                ),
                mutability: Mutability::Not,
                name: Some("r1".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: type_id(
                    type_context,
                    Type::Reference {
                        mutable: false,
                        inner: Box::new(Type::I64),
                    },
                ),
                mutability: Mutability::Not,
                name: Some("r2".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: type_id(type_context, Type::I64),
                mutability: Mutability::Not,
                name: Some("x".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: type_id(
            type_context,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            },
        ),
        ownership: Default::default(),
    }
}

#[test]
fn reference_liveness_ignores_local_display_name_presence() {
    let mut type_context = TypeContext::new();
    let named_func = test_function(&mut type_context);
    let mut unnamed_func = named_func.clone();
    unnamed_func.local_decls[2].name = None;

    let named = compute_reference_liveness(&named_func, &type_context);
    let unnamed = compute_reference_liveness(&unnamed_func, &type_context);

    assert_eq!(named.entry_sets, unnamed.entry_sets);
    assert_eq!(named.exit_sets, unnamed.exit_sets);
    assert_eq!(named.before_statement_sets, unnamed.before_statement_sets);
    assert_eq!(named.after_statement_sets, unnamed.after_statement_sets);
}

#[test]
fn test_reference_liveness_keeps_pointer_owner_live_for_projected_write() {
    let mut type_context = TypeContext::new();
    let pointer_i64 = type_id(&mut type_context, Type::Pointer(Box::new(Type::I64)));
    let i64 = type_id(&mut type_context, Type::I64);
    let mut_ref_i64 = type_id(
        &mut type_context,
        Type::Reference {
            mutable: true,
            inner: Box::new(Type::I64),
        },
    );
    let unit = type_id(&mut type_context, Type::Unit);
    let func = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0))),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![
                StatementData::storage_live(Local(1), None),
                StatementData::assign(
                    Place {
                        local: Local(1),
                        projection: vec![crate::mir::Projection::Deref],
                    },
                    Rvalue::Use(crate::mir::Operand::Copy(Place {
                        local: Local(4),
                        projection: vec![],
                    })),
                    None,
                ),
                StatementData::assign(
                    Place {
                        local: Local(1),
                        projection: vec![crate::mir::Projection::Deref],
                    },
                    Rvalue::Use(crate::mir::Operand::Copy(Place {
                        local: Local(4),
                        projection: vec![],
                    })),
                    None,
                ),
                StatementData::assign(
                    Place {
                        local: Local(2),
                        projection: vec![],
                    },
                    Rvalue::Ref(
                        Mutability::Mut,
                        Place {
                            local: Local(1),
                            projection: vec![],
                        },
                    ),
                    None,
                ),
                StatementData::storage_live(Local(3), None),
                StatementData::assign(
                    Place {
                        local: Local(3),
                        projection: vec![],
                    },
                    Rvalue::Cast(
                        crate::mir::Operand::Move(Place {
                            local: Local(2),
                            projection: vec![],
                        }),
                        pointer_i64,
                    ),
                    None,
                ),
                StatementData::storage_live(Local(4), None),
                StatementData::assign(
                    Place {
                        local: Local(4),
                        projection: vec![],
                    },
                    Rvalue::Use(crate::mir::Operand::Copy(Place {
                        local: Local(1),
                        projection: vec![],
                    })),
                    None,
                ),
                StatementData::storage_live(Local(5), None),
                StatementData::assign(
                    Place {
                        local: Local(3),
                        projection: vec![crate::mir::Projection::Deref],
                    },
                    Rvalue::Use(crate::mir::Operand::Copy(Place {
                        local: Local(4),
                        projection: vec![],
                    })),
                    None,
                ),
                StatementData::storage_dead(Local(5), None),
                StatementData::storage_dead(Local(4), None),
                StatementData::storage_dead(Local(3), None),
                StatementData::storage_dead(Local(2), None),
                StatementData::storage_dead(Local(1), None),
            ],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: i64,
                mutability: Mutability::Mut,
                name: Some("ret".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: pointer_i64,
                mutability: Mutability::Mut,
                name: Some("ptr_owner".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: mut_ref_i64,
                mutability: Mutability::Not,
                name: Some("r".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: pointer_i64,
                mutability: Mutability::Not,
                name: Some("ptr".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64,
                mutability: Mutability::Not,
                name: Some("y".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: unit,
                mutability: Mutability::Not,
                name: Some("tmp".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: i64,
        ownership: Default::default(),
    };

    let liveness = compute_reference_liveness(&func, &type_context);

    assert!(liveness.before_statement_sets[0][1].contains(Local(1)));
    assert!(liveness.after_statement_sets[0][1].contains(Local(1)));
    assert!(liveness.before_statement_sets[0][7].contains(Local(3)));
}

#[test]
fn reference_liveness_uses_local_set_at_statement_boundaries() {
    let mut type_context = TypeContext::new();
    let func = test_function_with_reference_copy(&mut type_context);
    let liveness = compute_reference_liveness(&func, &type_context);

    assert!(liveness.entry_sets[0].is_empty());
    assert!(liveness.before_statement_sets[0]
        .iter()
        .any(|set| set.iter().any(|local| local == Local(1))));
}

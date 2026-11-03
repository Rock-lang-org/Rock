use super::*;

#[test]
fn borrowck_uses_indexed_loan_state_for_active_conflicts() {
    let mut type_context = TypeContext::new();
    let func = mut_borrow_then_shared_read_function(&mut type_context);
    let err = BorrowChecker::check_function(&func, &type_context).expect_err("borrow conflict");

    assert!(err
        .0
        .iter()
        .any(|diag| { diag.message.contains("Cannot borrow") || diag.message.contains("borrow") }));
}

#[test]
fn immutable_binding_mut_borrow_diagnostic_ignores_display_name_presence() {
    let mut type_context = TypeContext::new();
    let named = immutable_binding_mut_borrow_function(&mut type_context, Some("value"));
    let unnamed = immutable_binding_mut_borrow_function(&mut type_context, None);

    for function in [&named, &unnamed] {
        let err = BorrowChecker::check_function(function, &type_context)
            .expect_err("mutable borrow of immutable binding should be rejected");
        assert!(err.0.iter().any(|diag| diag
            .message
            .contains("Cannot take a mutable reference to immutable binding")));
    }
}

fn immutable_binding_mut_borrow_function(
    type_context: &mut TypeContext,
    binding_name: Option<&str>,
) -> MirFunction {
    let unit = test_type_id(type_context, Type::Unit);
    let i64_id = test_type_id(type_context, Type::I64);
    let mut_ref_i64 = test_type_id(
        type_context,
        Type::Reference {
            mutable: true,
            inner: Box::new(Type::I64),
        },
    );

    MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(120))),
        name: "mut_borrow_immutable".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(2),
                    projection: Vec::new(),
                },
                Rvalue::Ref(
                    Mutability::Mut,
                    Place {
                        local: Local(1),
                        projection: Vec::new(),
                    },
                ),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: unit,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Not,
                name: binding_name.map(str::to_string),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: mut_ref_i64,
                mutability: Mutability::Not,
                name: Some("borrow".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: unit,
        ownership: MirOwnershipMetadata::default(),
    }
}

fn mut_borrow_then_shared_read_function(type_context: &mut TypeContext) -> MirFunction {
    let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0)));
    let local_decls = vec![
        LocalDecl {
            ty: test_type_id(type_context, Type::Unit),
            mutability: Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        },
        LocalDecl {
            ty: test_type_id(type_context, Type::I64),
            mutability: Mutability::Mut,
            name: Some("x".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        },
        LocalDecl {
            ty: test_type_id(
                type_context,
                Type::Reference {
                    inner: Box::new(Type::I64),
                    mutable: true,
                },
            ),
            mutability: Mutability::Not,
            name: Some("r".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        },
        LocalDecl {
            ty: test_type_id(
                type_context,
                Type::Reference {
                    inner: Box::new(Type::I64),
                    mutable: false,
                },
            ),
            mutability: Mutability::Not,
            name: Some("s".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        },
        LocalDecl {
            ty: test_type_id(
                type_context,
                Type::Reference {
                    inner: Box::new(Type::I64),
                    mutable: true,
                },
            ),
            mutability: Mutability::Not,
            name: Some("keep_r_live".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        },
    ];

    MirFunction {
        id: function_id,
        name: "mut_borrow_then_shared_read".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![
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
                StatementData::assign(
                    Place {
                        local: Local(3),
                        projection: vec![],
                    },
                    Rvalue::Ref(
                        Mutability::Not,
                        Place {
                            local: Local(1),
                            projection: vec![],
                        },
                    ),
                    None,
                ),
                StatementData::assign(
                    Place {
                        local: Local(4),
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
        local_decls,
        closure_captures: vec![],
        arg_count: 1,
        ret_type: test_type_id(type_context, Type::Unit),
        ownership: Default::default(),
    }
}

#[test]
fn assert_operand_read_conflicts_with_active_mut_borrow() {
    let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0)));
    let mut type_context = TypeContext::new();
    let local_decls = vec![
        LocalDecl {
            ty: test_type_id(&mut type_context, Type::Unit),
            mutability: Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        },
        LocalDecl {
            ty: test_type_id(&mut type_context, Type::I64),
            mutability: Mutability::Mut,
            name: Some("x".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        },
        LocalDecl {
            ty: test_type_id(
                &mut type_context,
                Type::Reference {
                    inner: Box::new(Type::I64),
                    mutable: true,
                },
            ),
            mutability: Mutability::Not,
            name: Some("r".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        },
        LocalDecl {
            ty: test_type_id(
                &mut type_context,
                Type::Reference {
                    inner: Box::new(Type::I64),
                    mutable: true,
                },
            ),
            mutability: Mutability::Not,
            name: Some("keep_r_live".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        },
    ];
    let block = BasicBlock {
        statements: vec![
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
            StatementData::assert(
                MirAssert {
                    kind: MirAssertKind::BoundsCheck,
                    operands: vec![Operand::Copy(Place {
                        local: Local(1),
                        projection: vec![],
                    })],
                },
                None,
            ),
            StatementData::assign(
                Place {
                    local: Local(3),
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
    };
    let program = MirProgram {
        functions: BTreeMap::from([(
            function_id.clone(),
            MirFunction {
                id: function_id,
                name: "assert_read_conflict".to_string(),
                basic_blocks: vec![block],
                local_decls,
                closure_captures: vec![],
                arg_count: 1,
                ret_type: test_type_id(&mut type_context, Type::Unit),
                ownership: Default::default(),
            },
        )]),
        type_context,
        backend_contract: Default::default(),
    };

    let diagnostics = BorrowChecker::run(&program).expect_err("assert read should conflict");

    assert!(diagnostics
        .0
        .iter()
        .any(|diagnostic| diagnostic.message.contains("borrow")));
}

#[test]
fn assert_move_operand_read_does_not_conflict_with_active_shared_borrow() {
    let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0)));
    let mut type_context = TypeContext::new();
    let local_decls = vec![
        LocalDecl {
            ty: test_type_id(&mut type_context, Type::Unit),
            mutability: Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        },
        LocalDecl {
            ty: test_type_id(&mut type_context, Type::I64),
            mutability: Mutability::Not,
            name: Some("x".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        },
        LocalDecl {
            ty: test_type_id(
                &mut type_context,
                Type::Reference {
                    inner: Box::new(Type::I64),
                    mutable: false,
                },
            ),
            mutability: Mutability::Not,
            name: Some("r".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        },
        LocalDecl {
            ty: test_type_id(
                &mut type_context,
                Type::Reference {
                    inner: Box::new(Type::I64),
                    mutable: false,
                },
            ),
            mutability: Mutability::Not,
            name: Some("keep_r_live".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        },
    ];
    let block = BasicBlock {
        statements: vec![
            StatementData::assign(
                Place {
                    local: Local(2),
                    projection: vec![],
                },
                Rvalue::Ref(
                    Mutability::Not,
                    Place {
                        local: Local(1),
                        projection: vec![],
                    },
                ),
                None,
            ),
            StatementData::assert(
                MirAssert {
                    kind: MirAssertKind::BoundsCheck,
                    operands: vec![Operand::Move(Place {
                        local: Local(1),
                        projection: vec![],
                    })],
                },
                None,
            ),
            StatementData::assign(
                Place {
                    local: Local(3),
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
    };
    let program = MirProgram {
        functions: BTreeMap::from([(
            function_id.clone(),
            MirFunction {
                id: function_id,
                name: "assert_move_read_shared_borrow".to_string(),
                basic_blocks: vec![block],
                local_decls,
                closure_captures: vec![],
                arg_count: 1,
                ret_type: test_type_id(&mut type_context, Type::Unit),
                ownership: Default::default(),
            },
        )]),
        type_context,
        backend_contract: Default::default(),
    };

    BorrowChecker::run(&program).expect("assert move operand should be read-only");
}

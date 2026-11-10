use super::*;

#[test]
fn borrowck_rejects_spanless_non_cleanup_discriminant_read_after_move() {
    let mut type_context = TypeContext::new();
    let enum_id = DefId::new(CrateId(0), LocalDefId(700));
    let enum_ty = Type::Enum {
        id: enum_id,
        args: Vec::new(),
    };
    let enum_ty_id = test_type_id(&mut type_context, enum_ty);
    let i64_id = test_type_id(&mut type_context, Type::I64);
    let function = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(701))),
        name: "spanless_discriminant_after_move".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![
                StatementData::assign(
                    Place {
                        local: Local(2),
                        projection: Vec::new(),
                    },
                    Rvalue::Use(Operand::Move(Place {
                        local: Local(1),
                        projection: Vec::new(),
                    })),
                    None,
                ),
                StatementData::assign(
                    Place {
                        local: Local(3),
                        projection: Vec::new(),
                    },
                    Rvalue::Discriminant(Place {
                        local: Local(1),
                        projection: Vec::new(),
                    }),
                    None,
                ),
                StatementData::assign(
                    Place {
                        local: Local(0),
                        projection: Vec::new(),
                    },
                    Rvalue::Use(Operand::Copy(Place {
                        local: Local(3),
                        projection: Vec::new(),
                    })),
                    None,
                ),
            ],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: enum_ty_id,
                mutability: Mutability::Not,
                name: Some("value".to_string()),
                span: None,
                source: crate::mir::LocalSource::Argument,
            },
            LocalDecl {
                ty: enum_ty_id,
                mutability: Mutability::Not,
                name: Some("moved".to_string()),
                span: None,
                source: crate::mir::LocalSource::Temporary,
            },
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Not,
                name: Some("discriminant".to_string()),
                span: None,
                source: crate::mir::LocalSource::Temporary,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: i64_id,
        ownership: MirOwnershipMetadata::default(),
    };

    let err = BorrowChecker::check_function(&function, &type_context)
        .expect_err("spanless non-cleanup discriminant read after move should be rejected");
    assert!(err
        .0
        .iter()
        .any(|diag| diag.message.contains("borrow of moved value")));
    let diagnostic = err
        .0
        .iter()
        .find(|diag| diag.message.contains("borrow of moved value"))
        .expect("move diagnostic should be present");
    assert_eq!(diagnostic.code, Some(DiagnosticCode::Internal));
    assert_eq!(
        diagnostic.location,
        crate::diagnostic::DiagnosticLocation::Toolchain
    );
    assert!(diagnostic.primary.is_none());
    assert!(diagnostic.secondary.is_empty());
}

#[test]
fn borrowck_reports_parent_use_after_field_move() {
    let mut type_context = TypeContext::new();
    let i64_id = type_context.intern_type(&Type::I64);
    let function = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(99))),
        name: "field_move".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![
                StatementData::assign(
                    Place {
                        local: Local(2),
                        projection: Vec::new(),
                    },
                    Rvalue::Use(Operand::Move(Place {
                        local: Local(1),
                        projection: vec![Projection::Field {
                            index: 0,
                            identity: None,
                        }],
                    })),
                    None,
                ),
                StatementData::assign(
                    Place {
                        local: Local(0),
                        projection: Vec::new(),
                    },
                    Rvalue::Use(Operand::Copy(Place {
                        local: Local(1),
                        projection: Vec::new(),
                    })),
                    None,
                ),
            ],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Mut,
                name: Some("ret".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Mut,
                name: Some("value".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Mut,
                name: Some("tmp".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: i64_id,
        ownership: Default::default(),
    };

    let err = BorrowChecker::check_function(&function, &type_context)
        .expect_err("parent use after field move should be rejected");
    assert!(err.0.iter().any(|diag| diag.message.contains("moved")));
}

#[test]
fn borrowck_reports_drop_after_field_move() {
    let mut type_context = TypeContext::new();
    let i64_id = type_context.intern_type(&Type::I64);
    let function = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(101))),
        name: "drop_after_field_move".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: vec![StatementData::assign(
                    Place {
                        local: Local(2),
                        projection: Vec::new(),
                    },
                    Rvalue::Use(Operand::Move(Place {
                        local: Local(1),
                        projection: vec![Projection::Field {
                            index: 0,
                            identity: None,
                        }],
                    })),
                    None,
                )],
                terminator: Some(Terminator::Drop {
                    place: Place {
                        local: Local(1),
                        projection: Vec::new(),
                    },
                    target: crate::mir::BasicBlockId(1),
                }),
            },
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Mut,
                name: Some("ret".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Mut,
                name: Some("value".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Mut,
                name: Some("tmp".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: i64_id,
        ownership: Default::default(),
    };

    let err = BorrowChecker::check_function(&function, &type_context)
        .expect_err("drop after field move should be rejected");
    assert!(err.0.iter().any(|diag| diag.message.contains("moved")));
}

#[test]
fn borrowck_allows_sibling_use_after_field_move() {
    let mut type_context = TypeContext::new();
    let i64_id = type_context.intern_type(&Type::I64);
    let function = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(100))),
        name: "field_move_sibling".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![
                StatementData::assign(
                    Place {
                        local: Local(2),
                        projection: Vec::new(),
                    },
                    Rvalue::Use(Operand::Move(Place {
                        local: Local(1),
                        projection: vec![Projection::Field {
                            index: 0,
                            identity: None,
                        }],
                    })),
                    None,
                ),
                StatementData::assign(
                    Place {
                        local: Local(0),
                        projection: Vec::new(),
                    },
                    Rvalue::Use(Operand::Copy(Place {
                        local: Local(1),
                        projection: vec![Projection::Field {
                            index: 1,
                            identity: None,
                        }],
                    })),
                    None,
                ),
            ],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Mut,
                name: Some("ret".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Mut,
                name: Some("value".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Mut,
                name: Some("tmp".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: i64_id,
        ownership: Default::default(),
    };

    BorrowChecker::check_function(&function, &type_context)
        .expect("sibling field remains usable after moving another field");
}

#[test]
fn borrowck_rejects_non_copy_move_out_of_shared_reference() {
    let mut type_context = TypeContext::new();
    let string_def = DefId::new(CrateId(0), LocalDefId(130));
    let string_ty = Type::Struct {
        id: string_def,
        args: Vec::new(),
    };
    let string_id = type_context.intern_type(&string_ty);
    let ref_string_id = type_context.intern_type(&Type::Reference {
        mutable: false,
        inner: Box::new(string_ty),
    });
    let function = move_or_copy_out_of_reference_function(
        string_id,
        ref_string_id,
        Operand::Move(Place {
            local: Local(1),
            projection: vec![Projection::Deref],
        }),
    );
    let mut backend_contract = MirBackendContract::default();
    backend_contract.nominal_layouts.insert(
        string_def,
        MirNominalLayout::Struct {
            id: string_def,
            fields: Vec::new(),
            generic_params: Vec::new(),
        },
    );

    let err = BorrowChecker::check_function_with_contract(
        &function,
        &type_context,
        &backend_contract,
        &HashMap::new(),
        &Default::default(),
    )
    .expect_err("moving a non-copy value out of a shared reference should be rejected");

    assert!(err
        .0
        .iter()
        .any(|diag| diag.message.contains("Cannot move non-copy value of type")));
}

#[test]
fn borrowck_allows_copy_out_of_shared_reference() {
    let mut type_context = TypeContext::new();
    let i64_id = type_context.intern_type(&Type::I64);
    let ref_i64_id = type_context.intern_type(&Type::Reference {
        mutable: false,
        inner: Box::new(Type::I64),
    });
    let function = move_or_copy_out_of_reference_function(
        i64_id,
        ref_i64_id,
        Operand::Copy(Place {
            local: Local(1),
            projection: vec![Projection::Deref],
        }),
    );

    BorrowChecker::check_function(&function, &type_context)
        .expect("copying I64 out of a shared reference should be allowed");
}

#[test]
fn borrowck_rejects_non_copy_copy_out_of_shared_reference() {
    let mut type_context = TypeContext::new();
    let pair_def = DefId::new(CrateId(0), LocalDefId(132));
    let i64_id = type_context.intern_type(&Type::I64);
    let pair_ty = Type::Struct {
        id: pair_def,
        args: Vec::new(),
    };
    let pair_id = type_context.intern_type(&pair_ty);
    let ref_pair_id = type_context.intern_type(&Type::Reference {
        mutable: false,
        inner: Box::new(pair_ty),
    });
    let function = move_or_copy_out_of_reference_function(
        pair_id,
        ref_pair_id,
        Operand::Copy(Place {
            local: Local(1),
            projection: vec![Projection::Deref],
        }),
    );
    let mut backend_contract = MirBackendContract::default();
    backend_contract.nominal_layouts.insert(
        pair_def,
        MirNominalLayout::Struct {
            id: pair_def,
            fields: vec![("x".to_string(), i64_id)],
            generic_params: Vec::new(),
        },
    );

    let err = BorrowChecker::check_function_with_contract(
        &function,
        &type_context,
        &backend_contract,
        &HashMap::new(),
        &Default::default(),
    )
    .expect_err("copy operand of non-copy value out of shared reference should be rejected");

    assert!(err
        .0
        .iter()
        .any(|diag| diag.message.contains("Cannot move non-copy value of type")));
}

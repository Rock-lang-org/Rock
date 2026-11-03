use super::*;

#[test]
fn reference_return_summary_joins_branch_return_origins() {
    let mut type_context = TypeContext::new();
    let ref_i64 = test_type_id(
        &mut type_context,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        },
    );
    let bool_id = test_type_id(&mut type_context, Type::Bool);
    let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(122)));
    let function = MirFunction {
        id: function_id.clone(),
        name: "choose_ref".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::SwitchInt {
                    discr: Operand::Copy(Place {
                        local: Local(1),
                        projection: Vec::new(),
                    }),
                    targets: vec![(1, crate::mir::BasicBlockId(1))],
                    otherwise: crate::mir::BasicBlockId(2),
                }),
            },
            BasicBlock {
                statements: vec![StatementData::assign(
                    Place {
                        local: Local(0),
                        projection: Vec::new(),
                    },
                    Rvalue::Use(Operand::Copy(Place {
                        local: Local(2),
                        projection: Vec::new(),
                    })),
                    None,
                )],
                terminator: Some(Terminator::Goto(crate::mir::BasicBlockId(3))),
            },
            BasicBlock {
                statements: vec![StatementData::assign(
                    Place {
                        local: Local(0),
                        projection: Vec::new(),
                    },
                    Rvalue::Use(Operand::Copy(Place {
                        local: Local(3),
                        projection: Vec::new(),
                    })),
                    None,
                )],
                terminator: Some(Terminator::Goto(crate::mir::BasicBlockId(3))),
            },
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![
            LocalDecl {
                ty: ref_i64,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: bool_id,
                mutability: Mutability::Not,
                name: Some("flag".to_string()),
                span: None,
                source: crate::mir::LocalSource::Argument,
            },
            LocalDecl {
                ty: ref_i64,
                mutability: Mutability::Not,
                name: Some("left".to_string()),
                span: None,
                source: crate::mir::LocalSource::Argument,
            },
            LocalDecl {
                ty: ref_i64,
                mutability: Mutability::Not,
                name: Some("right".to_string()),
                span: None,
                source: crate::mir::LocalSource::Argument,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 3,
        ret_type: ref_i64,
        ownership: MirOwnershipMetadata::default(),
    };
    let program = MirProgram {
        functions: BTreeMap::from([(function_id.clone(), function)]),
        type_context,
        backend_contract: Default::default(),
    };

    let summaries = BorrowChecker::reference_return_summaries(&program);
    let summary = summaries
        .get(&function_id)
        .expect("choose_ref should have a reference return summary");

    assert!(summary.contains(&ReferenceOrigin::Param(Local(2))));
    assert!(summary.contains(&ReferenceOrigin::Param(Local(3))));
}

#[test]
fn instance_reference_return_summary_maps_receiver_origin() {
    let mut type_context = TypeContext::new();
    let ref_i64 = test_type_id(
        &mut type_context,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        },
    );
    let instance_id = InstanceId(902);
    let callee_id = MirFunctionId::Instance(instance_id);
    let caller_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(903)));
    let callee = MirFunction {
        id: callee_id.clone(),
        name: "instance_ref".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(0),
                    projection: Vec::new(),
                },
                Rvalue::Use(Operand::Copy(Place {
                    local: Local(1),
                    projection: Vec::new(),
                })),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: ref_i64,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: ref_i64,
                mutability: Mutability::Not,
                name: Some("receiver".to_string()),
                span: None,
                source: crate::mir::LocalSource::Argument,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: ref_i64,
        ownership: MirOwnershipMetadata::default(),
    };
    let caller = MirFunction {
        id: caller_id.clone(),
        name: "caller".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Call {
                    func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                        MirCallableKey::Instance(instance_id),
                    ))),
                    args: vec![Operand::Copy(Place {
                        local: Local(1),
                        projection: Vec::new(),
                    })],
                    destination: Place {
                        local: Local(0),
                        projection: Vec::new(),
                    },
                    target: crate::mir::BasicBlockId(1),
                    span: Some(crate::lexer::Span::test()),
                }),
            },
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![
            LocalDecl {
                ty: ref_i64,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: ref_i64,
                mutability: Mutability::Not,
                name: Some("receiver".to_string()),
                span: None,
                source: crate::mir::LocalSource::Argument,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: ref_i64,
        ownership: MirOwnershipMetadata::default(),
    };
    let program = MirProgram {
        functions: BTreeMap::from([(callee_id, callee), (caller_id.clone(), caller)]),
        type_context,
        backend_contract: Default::default(),
    };

    let summaries = BorrowChecker::reference_return_summaries(&program);
    assert_eq!(
        summaries.get(&caller_id),
        Some(&HashSet::from([ReferenceOrigin::Param(Local(1))]))
    );
}

#[test]
fn temporary_reference_return_call_destination_is_rejected() {
    let mut type_context = TypeContext::new();
    let unit = test_type_id(&mut type_context, Type::Unit);
    let i64_id = test_type_id(&mut type_context, Type::I64);
    let ref_i64 = test_type_id(
        &mut type_context,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        },
    );
    let callee_id = DefId::new(CrateId(0), LocalDefId(904));
    let caller_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(905)));
    let callee = MirFunction {
        id: MirFunctionId::Function(callee_id),
        name: "returns_temporary_reference".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(0),
                    projection: Vec::new(),
                },
                Rvalue::Ref(
                    Mutability::Not,
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
                ty: ref_i64,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Not,
                name: Some("temporary".to_string()),
                span: None,
                source: crate::mir::LocalSource::Temporary,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: ref_i64,
        ownership: MirOwnershipMetadata::default(),
    };
    let caller = MirFunction {
        id: caller_id.clone(),
        name: "stores_call_result".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Call {
                    func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                        MirCallableKey::Function(callee_id),
                    ))),
                    args: Vec::new(),
                    destination: Place {
                        local: Local(1),
                        projection: Vec::new(),
                    },
                    target: crate::mir::BasicBlockId(1),
                    span: Some(crate::lexer::Span::test()),
                }),
            },
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![
            LocalDecl {
                ty: unit,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: ref_i64,
                mutability: Mutability::Not,
                name: Some("stored".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: unit,
        ownership: MirOwnershipMetadata::default(),
    };
    let program = MirProgram {
        functions: BTreeMap::from([
            (MirFunctionId::Function(callee_id), callee),
            (caller_id.clone(), caller.clone()),
        ]),
        type_context,
        backend_contract: Default::default(),
    };
    let summaries = BorrowChecker::reference_return_summaries(&program);
    let mut diagnostics = Diagnostics::default();

    BorrowChecker::validate_reference_escapes(
        &caller,
        &program.type_context,
        &MirBackendContract::default(),
        &summaries,
        &mut diagnostics,
    );

    assert!(diagnostics
        .0
        .iter()
        .any(|diagnostic| diagnostic.message.contains("temporary")));
}

#[test]
fn reference_return_summary_preserves_param_origin_through_pointer_projection() {
    let mut type_context = TypeContext::new();
    let i64_id = type_context.intern_type(&Type::I64);
    let ptr_i64 = type_context.intern_ty(crate::type_context::Ty::Pointer(i64_id));
    let ref_i64 = type_context.intern_ty(crate::type_context::Ty::Reference {
        mutable: false,
        inner: i64_id,
    });
    let index_i64 = type_context.intern_type(&Type::I64);
    let function = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(127))),
        name: "project_pointer_ref".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![
                StatementData::assign(
                    Place {
                        local: Local(2),
                        projection: Vec::new(),
                    },
                    Rvalue::Use(Operand::Copy(Place {
                        local: Local(1),
                        projection: Vec::new(),
                    })),
                    None,
                ),
                StatementData::assign(
                    Place {
                        local: Local(0),
                        projection: Vec::new(),
                    },
                    Rvalue::Ref(
                        Mutability::Not,
                        Place {
                            local: Local(2),
                            projection: vec![Projection::Index(Local(3))],
                        },
                    ),
                    None,
                ),
            ],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: ref_i64,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: ptr_i64,
                mutability: Mutability::Not,
                name: Some("ptr_arg".to_string()),
                span: None,
                source: crate::mir::LocalSource::Argument,
            },
            LocalDecl {
                ty: ptr_i64,
                mutability: Mutability::Not,
                name: Some("ptr_temp".to_string()),
                span: None,
                source: crate::mir::LocalSource::Temporary,
            },
            LocalDecl {
                ty: index_i64,
                mutability: Mutability::Not,
                name: Some("idx".to_string()),
                span: None,
                source: crate::mir::LocalSource::Temporary,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: ref_i64,
        ownership: MirOwnershipMetadata::default(),
    };

    let origins = BorrowChecker::reference_return_origins(
        &function,
        &type_context,
        &MirBackendContract::default(),
        &HashMap::new(),
    );

    assert!(origins.contains(&ReferenceOrigin::Param(Local(1))));
    assert!(!origins.contains(&ReferenceOrigin::Param(Local(2))));
}

#[test]
fn opaque_reference_return_maps_reference_argument_origins() {
    let mut type_context = TypeContext::new();
    let ref_i64 = test_type_id(
        &mut type_context,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        },
    );
    let extern_id = DefId::new(CrateId(0), LocalDefId(123));
    let function = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(124))),
        name: "extern_passthrough".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Call {
                    func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                        MirCallableKey::Extern(extern_id),
                    ))),
                    args: vec![Operand::Copy(Place {
                        local: Local(1),
                        projection: Vec::new(),
                    })],
                    destination: Place {
                        local: Local(0),
                        projection: Vec::new(),
                    },
                    target: crate::mir::BasicBlockId(1),
                    span: Some(crate::lexer::Span::test()),
                }),
            },
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![
            LocalDecl {
                ty: ref_i64,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: ref_i64,
                mutability: Mutability::Not,
                name: Some("input".to_string()),
                span: None,
                source: crate::mir::LocalSource::Argument,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: ref_i64,
        ownership: MirOwnershipMetadata::default(),
    };
    let mut diagnostics = Diagnostics::default();

    BorrowChecker::validate_reference_escapes(
        &function,
        &type_context,
        &MirBackendContract::default(),
        &HashMap::new(),
        &mut diagnostics,
    );

    assert!(diagnostics.0.is_empty());
}

#[test]
fn reference_return_summary_defers_missing_known_call_summary() {
    let mut type_context = TypeContext::new();
    let ref_i64 = test_type_id(
        &mut type_context,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        },
    );
    let callee_id = DefId::new(CrateId(0), LocalDefId(125));
    let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(126)));
    let function = MirFunction {
        id: function_id.clone(),
        name: "calls_later_summarized".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Call {
                    func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                        MirCallableKey::Function(callee_id),
                    ))),
                    args: Vec::new(),
                    destination: Place {
                        local: Local(0),
                        projection: Vec::new(),
                    },
                    target: crate::mir::BasicBlockId(1),
                    span: Some(crate::lexer::Span::test()),
                }),
            },
            BasicBlock {
                statements: Vec::new(),
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![LocalDecl {
            ty: ref_i64,
            mutability: Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: ref_i64,
        ownership: MirOwnershipMetadata::default(),
    };
    let origins = BorrowChecker::reference_return_origins(
        &function,
        &type_context,
        &MirBackendContract::default(),
        &HashMap::new(),
    );

    assert!(origins.is_empty());
}

use super::*;

fn nominal_owner_transfer_function(
    type_context: &mut TypeContext,
    owner_ty: crate::ids::TypeId,
) -> MirFunction {
    let shared_ref_i64 = type_id(
        type_context,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        },
    );
    let i64 = type_id(type_context, Type::I64);
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
                    Rvalue::Use(Operand::Move(Place {
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
                ty: owner_ty,
                mutability: Mutability::Mut,
                name: Some("ret".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: shared_ref_i64,
                mutability: Mutability::Not,
                name: Some("r".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: owner_ty,
                mutability: Mutability::Not,
                name: Some("owner".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64,
                mutability: Mutability::Mut,
                name: Some("x".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: owner_ty,
        ownership: Default::default(),
    }
}

#[test]
fn active_loan_entries_preserve_owner_for_contract_struct_field_reference() {
    let mut type_context = TypeContext::new();
    let struct_id = DefId::new(CrateId(0), LocalDefId(60));
    let generic = GenericParamId {
        owner: struct_id,
        index: 0,
    };
    let field_ty = type_id(
        &mut type_context,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Generic(generic)),
        },
    );
    let owner_ty = type_id(
        &mut type_context,
        Type::Struct {
            id: struct_id,
            args: vec![Type::I64],
        },
    );
    let func = nominal_owner_transfer_function(&mut type_context, owner_ty);
    let mut backend_contract = MirBackendContract::default();
    backend_contract.nominal_layouts.insert(
        struct_id,
        MirNominalLayout::Struct {
            id: struct_id,
            fields: vec![("value".to_string(), field_ty)],
            generic_params: vec![generic],
        },
    );

    let states =
        active_states_after_each_statement_with_contract(&func, &type_context, &backend_contract);

    assert!(states[1].owner_contains(LoanId(0), Local(2)));
}

#[test]
fn active_loan_entries_preserve_owner_for_contract_enum_field_reference() {
    let mut type_context = TypeContext::new();
    let enum_id = DefId::new(CrateId(0), LocalDefId(61));
    let generic = GenericParamId {
        owner: enum_id,
        index: 0,
    };
    let field_ty = type_id(
        &mut type_context,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Generic(generic)),
        },
    );
    let owner_ty = type_id(
        &mut type_context,
        Type::Enum {
            id: enum_id,
            args: vec![Type::I64],
        },
    );
    let func = nominal_owner_transfer_function(&mut type_context, owner_ty);
    let mut backend_contract = MirBackendContract::default();
    backend_contract.nominal_layouts.insert(
        enum_id,
        MirNominalLayout::Enum {
            id: enum_id,
            variants: vec![crate::mir::MirEnumVariantLayout {
                name: "Some".to_string(),
                fields: MirVariantLayoutFields::Positional(vec![field_ty]),
            }],
            generic_params: vec![generic],
        },
    );

    let states =
        active_states_after_each_statement_with_contract(&func, &type_context, &backend_contract);

    assert!(states[1].owner_contains(LoanId(0), Local(2)));
}

#[test]
fn active_loan_entries_preserve_owner_for_projection_output_nominal_reference_field() {
    let mut type_context = TypeContext::new();
    let struct_id = DefId::new(CrateId(0), LocalDefId(62));
    let trait_id = DefId::new(CrateId(0), LocalDefId(63));
    let generic = GenericParamId {
        owner: struct_id,
        index: 0,
    };
    let base_ty = type_id(&mut type_context, Type::I64);
    let field_ty = type_id(
        &mut type_context,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Generic(generic)),
        },
    );
    let output_ty = type_id(
        &mut type_context,
        Type::Struct {
            id: struct_id,
            args: vec![Type::I64],
        },
    );
    let owner_ty = type_id(
        &mut type_context,
        Type::Projection {
            ty: Box::new(Type::I64),
            trait_id,
            assoc_type: assoc_type(trait_id),
            trait_args: vec![],
        },
    );
    let func = nominal_owner_transfer_function(&mut type_context, owner_ty);
    let mut backend_contract = MirBackendContract::default();
    backend_contract.nominal_layouts.insert(
        struct_id,
        MirNominalLayout::Struct {
            id: struct_id,
            fields: vec![("value".to_string(), field_ty)],
            generic_params: vec![generic],
        },
    );
    backend_contract.projection_outputs.insert(
        MirProjectionKey {
            base: base_ty,
            trait_id,
            assoc_type_id: AssocTypeId(0),
            trait_args: vec![],
        },
        output_ty,
    );

    let states =
        active_states_after_each_statement_with_contract(&func, &type_context, &backend_contract);

    assert!(states[1].owner_contains(LoanId(0), Local(2)));
}

#[test]
fn active_loan_entries_copy_receiver_loan_to_returned_reference() {
    let mut type_context = TypeContext::new();
    let i64 = type_id(&mut type_context, Type::I64);
    let shared_ref_i64 = type_id(
        &mut type_context,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        },
    );
    let callee_ty = type_id(
        &mut type_context,
        Type::function(
            vec![Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            }],
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            },
        ),
    );
    let func = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0))),
        name: "main".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: vec![StatementData::assign(
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
                )],
                terminator: Some(Terminator::Call {
                    func: Operand::Copy(Place {
                        local: Local(4),
                        projection: vec![],
                    }),
                    args: vec![Operand::Copy(Place {
                        local: Local(2),
                        projection: vec![],
                    })],
                    destination: Place {
                        local: Local(3),
                        projection: vec![],
                    },
                    target: BasicBlockId(1),
                    span: Some(crate::lexer::Span::test()),
                }),
            },
            BasicBlock {
                statements: vec![StatementData::assign(
                    Place {
                        local: Local(0),
                        projection: vec![],
                    },
                    Rvalue::Use(Operand::Copy(Place {
                        local: Local(3),
                        projection: vec![],
                    })),
                    None,
                )],
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![
            LocalDecl {
                ty: shared_ref_i64,
                mutability: Mutability::Not,
                name: Some("ret".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64,
                mutability: Mutability::Mut,
                name: Some("x".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: shared_ref_i64,
                mutability: Mutability::Not,
                name: Some("receiver_ref".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: shared_ref_i64,
                mutability: Mutability::Not,
                name: Some("returned_ref".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: callee_ty,
                mutability: Mutability::Not,
                name: Some("callee".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: shared_ref_i64,
        ownership: Default::default(),
    };
    let borrows = crate::mir::borrowck::borrows::collect_function_borrows(&func);
    let table = crate::mir::dataflow::analyses::LoanTable::from_borrows(&borrows);
    let by_location = LoanLocationIndex::for_function(&table, &func);
    let reference_liveness = compute_reference_liveness(&func, &type_context);

    let entries = compute_active_loan_entries(
        &func,
        &type_context,
        &table,
        &by_location,
        &reference_liveness,
    );
    let loan_id = table.iter().next().expect("loan").id;

    assert!(entries[1].owners(loan_id).contains(Local(3)));
}

#[test]
fn active_loan_entries_move_reference_owner_to_call_destination() {
    let mut type_context = TypeContext::new();
    let i64 = type_id(&mut type_context, Type::I64);
    let shared_ref_i64 = type_id(
        &mut type_context,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        },
    );
    let slice_ref_i64 = type_id(
        &mut type_context,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Slice(Box::new(Type::I64))),
        },
    );
    let function = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(141))),
        name: "move_reference_call_destination".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: vec![StatementData::assign(
                    Place {
                        local: Local(1),
                        projection: Vec::new(),
                    },
                    Rvalue::Ref(
                        Mutability::Not,
                        Place {
                            local: Local(2),
                            projection: Vec::new(),
                        },
                    ),
                    None,
                )],
                terminator: Some(Terminator::Call {
                    func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                        MirCallableKey::Intrinsic(MirIntrinsicId::ArrayRefToSlice),
                    ))),
                    args: vec![Operand::Move(Place {
                        local: Local(1),
                        projection: Vec::new(),
                    })],
                    destination: Place {
                        local: Local(3),
                        projection: Vec::new(),
                    },
                    target: BasicBlockId(1),
                    span: Some(crate::lexer::Span::test()),
                }),
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
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![
            LocalDecl {
                ty: slice_ref_i64,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: shared_ref_i64,
                mutability: Mutability::Not,
                name: Some("source".to_string()),
                span: None,
                source: crate::mir::LocalSource::Temporary,
            },
            LocalDecl {
                ty: i64,
                mutability: Mutability::Mut,
                name: Some("value".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: slice_ref_i64,
                mutability: Mutability::Not,
                name: Some("destination".to_string()),
                span: None,
                source: crate::mir::LocalSource::Temporary,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: slice_ref_i64,
        ownership: Default::default(),
    };
    let borrows = crate::mir::borrowck::borrows::collect_function_borrows(&function);
    let table = crate::mir::dataflow::analyses::LoanTable::from_borrows(&borrows);
    let by_location = LoanLocationIndex::for_function(&table, &function);
    let reference_liveness = compute_reference_liveness(&function, &type_context);
    let entries = compute_active_loan_entries(
        &function,
        &type_context,
        &table,
        &by_location,
        &reference_liveness,
    );
    let loan_id = table.iter().next().expect("loan").id;

    assert!(entries[1].owners(loan_id).contains(Local(3)));
    assert!(!entries[1].owners(loan_id).contains(Local(1)));
}

#[test]
fn active_loan_transfer_preserves_aggregate_reference_owner() {
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let i64 = type_id(&mut type_context, Type::I64);
    let mut_ref = type_id(
        &mut type_context,
        Type::Reference {
            mutable: true,
            inner: Box::new(Type::I64),
        },
    );
    let holder = type_id(
        &mut type_context,
        Type::Tuple(vec![Type::Reference {
            mutable: true,
            inner: Box::new(Type::I64),
        }]),
    );
    let function = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(140))),
        name: "aggregate_reference".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(3),
                    projection: Vec::new(),
                },
                Rvalue::Aggregate(
                    AggregateKind::Tuple,
                    vec![Operand::Move(Place {
                        local: Local(2),
                        projection: Vec::new(),
                    })],
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
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: i64,
                mutability: Mutability::Mut,
                name: Some("value".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: mut_ref,
                mutability: Mutability::Not,
                name: Some("reference".to_string()),
                span: None,
                source: crate::mir::LocalSource::Temporary,
            },
            LocalDecl {
                ty: holder,
                mutability: Mutability::Not,
                name: Some("holder".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let statement = &function.basic_blocks[0].statements[0];
    let mut state = LoanState::new(1);
    state.add_owner(LoanId(0), Local(2));

    apply_active_loan_statement_transfer(
        &mut state,
        &function,
        &type_context,
        &MirBackendContract::default(),
        statement,
    );

    assert!(!state.owner_contains(LoanId(0), Local(2)));
    assert!(state.owner_contains(LoanId(0), Local(3)));
}

#[test]
fn active_loan_transfer_ignores_mut_reference_cast_to_non_pointer() {
    let mut type_context = TypeContext::new();
    let unit = type_context.intern_type(&Type::Unit);
    let i64 = type_context.intern_type(&Type::I64);
    let mut_i64_ref = type_context.intern_type(&Type::Reference {
        mutable: true,
        inner: Box::new(Type::I64),
    });
    let func = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0))),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: Vec::new(),
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: unit,
                mutability: Mutability::Mut,
                name: Some("ret".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: mut_i64_ref,
                mutability: Mutability::Not,
                name: Some("r".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64,
                mutability: Mutability::Not,
                name: Some("bits".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let mut state = LoanState::new(1);
    state.add_owner(crate::ids::LoanId(0), Local(1));
    let stmt = StatementData::assign(
        Place {
            local: Local(2),
            projection: vec![],
        },
        Rvalue::Cast(
            Operand::Copy(Place {
                local: Local(1),
                projection: vec![],
            }),
            i64,
        ),
        None,
    );

    apply_active_loan_statement_transfer(
        &mut state,
        &func,
        &type_context,
        &MirBackendContract::default(),
        &stmt,
    );

    let owners = state.owners(crate::ids::LoanId(0));
    assert!(owners.contains(Local(1)));
    assert!(!owners.contains(Local(2)));
}

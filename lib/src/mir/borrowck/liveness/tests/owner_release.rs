use super::*;

fn active_states_after_each_statement(
    func: &MirFunction,
    type_context: &TypeContext,
) -> Vec<LoanState> {
    let backend_contract = MirBackendContract::default();
    active_states_after_each_statement_with_contract(func, type_context, &backend_contract)
}

#[test]
fn active_loan_entries_release_owner_after_storage_dead() {
    let mut type_context = TypeContext::new();
    let func = test_function(&mut type_context);
    let reference_liveness = compute_reference_liveness(&func, &type_context);
    let borrows = vec![BorrowData {
        owner: Local(2),
        reborrow_source: None,
        place: Place {
            local: Local(1),
            projection: vec![],
        },
        kind: AccessKind::BorrowShared,
        created_at: Location::new(BasicBlockId(0), StatementIndex(1)),
        origin_span: None,
    }];
    let table = LoanTable::from_borrows(&borrows);
    let grouped = LoanLocationIndex::for_function(&table, &func);
    let entries =
        compute_active_loan_entries(&func, &type_context, &table, &grouped, &reference_liveness);

    let block = &func.basic_blocks[0];
    let mut state = entries[0].clone();
    for (stmt_idx, stmt) in block.statements.iter().enumerate() {
        apply_active_loan_statement_transfer(
            &mut state,
            &func,
            &type_context,
            &MirBackendContract::default(),
            stmt,
        );

        let location = Location::new(BasicBlockId(0), StatementIndex(stmt_idx));
        for loan_id in grouped.loan_ids_at(location) {
            state.activate(loan_id, &table);
        }

        if let Some(live_locals) = reference_liveness
            .after_statement_sets
            .get(0)
            .and_then(|stmt_sets| stmt_sets.get(stmt_idx))
        {
            state.retain_owners(|owner| live_locals.contains(owner));
        }

        if let StatementKind::StorageDead(local) = &stmt.kind {
            state.release_owner(*local);
        }

        if stmt_idx == 1 {
            assert!(state.is_active(LoanId(0)));
            assert!(state.owner_contains(LoanId(0), Local(2)));
        }
        if stmt_idx == 2 {
            assert!(!state.is_active(LoanId(0)));
        }
    }
}

#[test]
fn active_loan_statement_precision_releases_drop_in_same_block() {
    let mut type_context = TypeContext::new();
    let func = test_function(&mut type_context);
    let states = active_states_after_each_statement(&func, &type_context);

    assert!(!states[0].is_active(LoanId(0)));
    assert!(states[1].is_active(LoanId(0)));
    assert!(states[1].owner_contains(LoanId(0), Local(2)));
    assert!(!states[2].is_active(LoanId(0)));
}

#[test]
fn active_loan_statement_precision_moves_owner_before_next_statement() {
    let mut type_context = TypeContext::new();
    let shared_ref_i64 = type_id(
        &mut type_context,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        },
    );
    let i64 = type_id(&mut type_context, Type::I64);
    let func = MirFunction {
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
                        local: Local(4),
                        projection: vec![],
                    },
                    Rvalue::Use(Operand::Copy(Place {
                        local: Local(2),
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
                        local: Local(4),
                        projection: vec![],
                    })),
                    None,
                ),
                StatementData::storage_dead(Local(4), None),
            ],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: shared_ref_i64,
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
                ty: shared_ref_i64,
                mutability: Mutability::Not,
                name: Some("moved".to_string()),
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
                name: Some("used".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: shared_ref_i64,
        ownership: Default::default(),
    };
    let states = active_states_after_each_statement(&func, &type_context);

    assert!(states[0].owner_contains(LoanId(0), Local(1)));
    assert!(!states[1].owner_contains(LoanId(0), Local(1)));
    assert!(states[1].owner_contains(LoanId(0), Local(2)));
    assert!(states[2].owner_contains(LoanId(0), Local(4)));
}

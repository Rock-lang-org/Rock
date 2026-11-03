use super::*;

#[test]
fn active_loan_worklist_matches_sweep_for_branches_and_backedges() {
    use crate::mir::dataflow::Analysis;

    let mut type_context = TypeContext::new();
    let simple = test_function(&mut type_context);
    let (branch, _, _, _) = branch_merge_borrow_function(&mut type_context);
    let mut cyclic = branch.clone();
    cyclic.basic_blocks.last_mut().unwrap().terminator = Some(Terminator::Goto(BasicBlockId(0)));
    for func in [simple, branch, cyclic] {
        let borrows = crate::mir::borrowck::borrows::collect_function_borrows(&func);
        let table = LoanTable::from_borrows(&borrows);
        let by_location = LoanLocationIndex::for_function(&table, &func);
        let liveness = compute_reference_liveness(&func, &type_context);
        let contract = MirBackendContract::default();
        let summaries = std::collections::HashMap::new();
        let analysis = ActiveLoanAnalysis::new(
            &type_context,
            &contract,
            &summaries,
            &table,
            &by_location,
            &liveness,
        );
        let successors = super::compute_successors(&func);
        let mut predecessors = vec![Vec::new(); func.basic_blocks.len()];
        for (block, targets) in successors.iter().enumerate() {
            for &target in targets {
                predecessors[target].push(block);
            }
        }
        // Independent whole-CFG reference solver for the original equations.
        let mut entries = vec![LoanState::new(table.len()); func.basic_blocks.len()];
        let mut exits = entries.clone();
        let mut converged = false;
        for _ in 0..100 {
            let mut changed = false;
            for block in 0..func.basic_blocks.len() {
                let mut entry = LoanState::new(table.len());
                for &predecessor in &predecessors[block] {
                    entry.join(&exits[predecessor]);
                }
                let mut exit = entry.clone();
                analysis.apply_block(&mut exit, &func, BasicBlockId(block));
                changed |= entries[block] != entry || exits[block] != exit;
                entries[block] = entry;
                exits[block] = exit;
            }
            if !changed {
                converged = true;
                break;
            }
        }
        assert!(converged);
        assert_eq!(
            crate::mir::dataflow::run_fixpoint(&analysis, &func).entry_sets,
            entries
        );
    }
}

#[test]
fn active_loan_worklist_does_not_propagate_from_unreachable_predecessors() {
    let mut type_context = TypeContext::new();
    let mut func = test_function(&mut type_context);
    let mut unreachable = func.basic_blocks[0].clone();
    unreachable.statements.truncate(2);
    unreachable.terminator = Some(Terminator::Goto(BasicBlockId(2)));
    func.basic_blocks = vec![
        BasicBlock {
            statements: Vec::new(),
            terminator: Some(Terminator::Goto(BasicBlockId(2))),
        },
        unreachable,
        BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(1),
                    projection: vec![],
                },
                Rvalue::Use(Operand::Copy(Place {
                    local: Local(2),
                    projection: vec![crate::mir::Projection::Deref],
                })),
                None,
            )],
            terminator: Some(Terminator::Return),
        },
    ];
    let table = LoanTable::from_borrows(&[BorrowData {
        owner: Local(2),
        reborrow_source: None,
        place: Place {
            local: Local(1),
            projection: vec![],
        },
        kind: AccessKind::BorrowShared,
        created_at: Location::new(BasicBlockId(1), StatementIndex(1)),
        origin_span: None,
    }]);
    assert_eq!(table.len(), 1);
    let index = LoanLocationIndex::for_function(&table, &func);
    let liveness = compute_reference_liveness(&func, &type_context);
    let entries = compute_active_loan_entries(&func, &type_context, &table, &index, &liveness);
    assert!(
        !entries[2].is_active(LoanId(0)),
        "unreachable loans must not reach a live join"
    );
}

fn branch_merge_borrow_function(
    type_context: &mut TypeContext,
) -> (MirFunction, BasicBlockId, Local, Local) {
    let owner_a = Local(2);
    let owner_b = Local(3);
    let merge_block = BasicBlockId(3);

    let func = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0))),
        name: "main".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: vec![StatementData::assign(
                    Place {
                        local: Local(1),
                        projection: vec![],
                    },
                    Rvalue::Ref(
                        Mutability::Not,
                        Place {
                            local: Local(4),
                            projection: vec![],
                        },
                    ),
                    None,
                )],
                terminator: Some(Terminator::SwitchInt {
                    discr: Operand::Copy(Place {
                        local: Local(5),
                        projection: vec![],
                    }),
                    targets: vec![(0, BasicBlockId(1))],
                    otherwise: BasicBlockId(2),
                }),
            },
            BasicBlock {
                statements: vec![StatementData::assign(
                    Place {
                        local: owner_a,
                        projection: vec![],
                    },
                    Rvalue::Use(Operand::Copy(Place {
                        local: Local(1),
                        projection: vec![],
                    })),
                    None,
                )],
                terminator: Some(Terminator::Goto(merge_block)),
            },
            BasicBlock {
                statements: vec![StatementData::assign(
                    Place {
                        local: owner_b,
                        projection: vec![],
                    },
                    Rvalue::Use(Operand::Copy(Place {
                        local: Local(1),
                        projection: vec![],
                    })),
                    None,
                )],
                terminator: Some(Terminator::Goto(merge_block)),
            },
            BasicBlock {
                statements: vec![
                    StatementData::assign(
                        Place {
                            local: Local(0),
                            projection: vec![],
                        },
                        Rvalue::Use(Operand::Copy(Place {
                            local: owner_a,
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
                            local: owner_b,
                            projection: vec![],
                        })),
                        None,
                    ),
                ],
                terminator: Some(Terminator::Return),
            },
        ],
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
                name: Some("r".to_string()),
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
                name: Some("owner_a".to_string()),
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
                name: Some("owner_b".to_string()),
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
            LocalDecl {
                ty: type_id(type_context, Type::I64),
                mutability: Mutability::Not,
                name: Some("cond".to_string()),
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
    };

    (func, merge_block, owner_a, owner_b)
}

#[test]
fn active_loan_loop_backedge_preserves_statement_owner_copies() {
    let mut type_context = TypeContext::new();
    let shared_ref_i64 = type_id(
        &mut type_context,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        },
    );
    let i64 = type_id(&mut type_context, Type::I64);
    let bool_ty = type_id(&mut type_context, Type::Bool);
    let func = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0))),
        name: "main".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: vec![StatementData::assign(
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
                )],
                terminator: Some(Terminator::Goto(BasicBlockId(1))),
            },
            BasicBlock {
                statements: vec![
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
                            local: Local(5),
                            projection: vec![],
                        },
                        Rvalue::Use(Operand::Copy(Place {
                            local: Local(2),
                            projection: vec![],
                        })),
                        None,
                    ),
                ],
                terminator: Some(Terminator::SwitchInt {
                    discr: Operand::Copy(Place {
                        local: Local(4),
                        projection: vec![],
                    }),
                    targets: vec![(0, BasicBlockId(1))],
                    otherwise: BasicBlockId(2),
                }),
            },
            BasicBlock {
                statements: vec![StatementData::assign(
                    Place {
                        local: Local(0),
                        projection: vec![],
                    },
                    Rvalue::Use(Operand::Copy(Place {
                        local: Local(5),
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
                name: Some("copy".to_string()),
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
                ty: bool_ty,
                mutability: Mutability::Not,
                name: Some("again".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: shared_ref_i64,
                mutability: Mutability::Not,
                name: Some("loop_carried".to_string()),
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
    let table = LoanTable::from_borrows(&borrows);
    let by_location = LoanLocationIndex::for_function(&table, &func);
    let reference_liveness = compute_reference_liveness(&func, &type_context);
    let entries = compute_active_loan_entries(
        &func,
        &type_context,
        &table,
        &by_location,
        &reference_liveness,
    );

    assert!(entries[1].owner_contains(LoanId(0), Local(1)));
    assert!(entries[1].owner_contains(LoanId(0), Local(5)));
    assert!(entries[2].owner_contains(LoanId(0), Local(5)));
}

#[test]
fn active_loan_dataflow_releases_dead_owner_before_successor_entry() {
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let i64 = type_id(&mut type_context, Type::I64);
    let shared_ref_i64 = type_id(
        &mut type_context,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        },
    );
    let func = MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0))),
        name: "main".to_string(),
        basic_blocks: vec![
            BasicBlock {
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
                    StatementData::storage_dead(Local(2), None),
                ],
                terminator: Some(Terminator::Goto(BasicBlockId(1))),
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
                name: Some("ret".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
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
                name: Some("r".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let borrows = crate::mir::borrowck::borrows::collect_function_borrows(&func);
    let table = LoanTable::from_borrows(&borrows);
    let grouped = LoanLocationIndex::for_function(&table, &func);
    let reference_liveness = compute_reference_liveness(&func, &type_context);
    let backend_contract = MirBackendContract::default();
    let summaries = std::collections::HashMap::new();
    let analysis = ActiveLoanAnalysis::new(
        &type_context,
        &backend_contract,
        &summaries,
        &table,
        &grouped,
        &reference_liveness,
    );

    let results = crate::mir::dataflow::run_fixpoint(&analysis, &func);

    assert!(!results.entry_sets[1].is_active(LoanId(0)));
}

#[test]
fn active_loan_entries_use_indexed_state_and_merge_owners() {
    let mut type_context = TypeContext::new();
    let (func, merge_block, owner_a, owner_b) = branch_merge_borrow_function(&mut type_context);
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
    let merged_owners = entries[merge_block.0].owners(loan_id);

    assert!(merged_owners.contains(owner_a));
    assert!(merged_owners.contains(owner_b));
}

#[test]
fn loan_location_index_uses_dense_slots_and_deduplicates_loan_ids() {
    let mut table = LoanTable::new();
    let created_at = Location::new(BasicBlockId(0), StatementIndex(1));
    let first = table.push(crate::mir::dataflow::analyses::LoanData {
        id: LoanId(99),
        reborrow_source: None,
        diagnostic_place: Place {
            local: Local(1),
            projection: vec![],
        },
        place_path: crate::ids::PlacePathId(0),
        initial_owner: Local(2),
        kind: crate::mir::dataflow::analyses::LoanKind::Shared,
        origin_span: None,
        created_at,
    });
    let second = table.push(crate::mir::dataflow::analyses::LoanData {
        id: LoanId(99),
        reborrow_source: None,
        diagnostic_place: Place {
            local: Local(3),
            projection: vec![],
        },
        place_path: crate::ids::PlacePathId(0),
        initial_owner: Local(4),
        kind: crate::mir::dataflow::analyses::LoanKind::Mut,
        origin_span: None,
        created_at,
    });

    let index = super::LoanLocationIndex::from_table(&table, 1, &[2]);

    assert_eq!(
        index.loan_ids_at(created_at).collect::<Vec<_>>(),
        vec![first, second]
    );
    assert_eq!(
        index.loan_ids_at(created_at).collect::<Vec<_>>(),
        vec![first, second]
    );
    assert!(index
        .loan_ids_at(Location::new(BasicBlockId(0), StatementIndex(0)))
        .next()
        .is_none());
}

use super::{
    apply_active_loan_statement_transfer, compute_active_loan_entries,
    compute_active_loan_entries_with_contract, compute_reference_liveness,
    compute_reference_liveness_with_contract, type_id_contains_reference, ActiveLoanAnalysis,
    LoanLocationIndex,
};
use crate::ids::{AssocTypeId, CrateId, DefId, LoanId, LocalDefId};
use crate::mir::borrowck::location::{Location, StatementIndex};
use crate::mir::dataflow::analyses::{LoanState, LoanTable};
use crate::mir::{
    AggregateKind, BasicBlock, BasicBlockId, Constant, Local, LocalDecl, MirBackendContract,
    MirCallable, MirCallableKey, MirFunction, MirFunctionId, MirIntrinsicId, MirNominalLayout,
    MirProjectionKey, MirVariantLayoutFields, Mutability, Operand, Place, Rvalue, StatementData,
    StatementKind, Terminator,
};
use crate::type_context::TypeContext;
use crate::type_services::facts::TypeFacts;
use crate::types::{AssociatedTypeKey, GenericParamId, Type};

use super::super::accesses::AccessKind;
use super::super::borrows::BorrowData;

fn type_id(type_context: &mut TypeContext, ty: Type) -> crate::ids::TypeId {
    type_context.intern_type(&ty)
}

fn assoc_type(owner: DefId) -> AssociatedTypeKey {
    AssociatedTypeKey {
        owner,
        assoc_type_id: AssocTypeId(0),
    }
}

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

fn test_function(type_context: &mut TypeContext) -> MirFunction {
    MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(0))),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![
                StatementData::storage_live(Local(1), None),
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
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: type_id(type_context, Type::Unit),
                mutability: Mutability::Mut,
                name: Some("ret".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: type_id(type_context, Type::I64),
                mutability: Mutability::Mut,
                name: Some("x".to_string()),
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
        ],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: type_id(type_context, Type::Unit),
        ownership: Default::default(),
    }
}

fn active_states_after_each_statement(
    func: &MirFunction,
    type_context: &TypeContext,
) -> Vec<LoanState> {
    let backend_contract = MirBackendContract::default();
    active_states_after_each_statement_with_contract(func, type_context, &backend_contract)
}

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

fn active_states_after_each_statement_with_contract(
    func: &MirFunction,
    type_context: &TypeContext,
    backend_contract: &MirBackendContract,
) -> Vec<LoanState> {
    let borrows = crate::mir::borrowck::borrows::collect_function_borrows(func);
    let table = LoanTable::from_borrows(&borrows);
    let by_location = LoanLocationIndex::for_function(&table, func);
    let reference_liveness =
        compute_reference_liveness_with_contract(func, type_context, backend_contract);
    let entries = compute_active_loan_entries_with_contract(
        func,
        type_context,
        backend_contract,
        &std::collections::HashMap::new(),
        &table,
        &by_location,
        &reference_liveness,
    );
    let block = &func.basic_blocks[0];
    let mut state = entries[0].clone();
    let mut states = Vec::with_capacity(block.statements.len());

    for (stmt_idx, stmt) in block.statements.iter().enumerate() {
        apply_active_loan_statement_transfer(
            &mut state,
            func,
            type_context,
            backend_contract,
            stmt,
        );
        let location = Location::new(BasicBlockId(0), StatementIndex(stmt_idx));
        for loan_id in by_location.loan_ids_at(location) {
            state.activate(loan_id, &table);
        }
        if let Some(live_locals) = reference_liveness
            .after_statement_sets
            .first()
            .and_then(|stmt_sets| stmt_sets.get(stmt_idx))
        {
            state.retain_owners(|owner| live_locals.contains(owner));
        }
        if let StatementKind::StorageDead(local) = &stmt.kind {
            state.release_owner(*local);
        }
        states.push(state.clone());
    }

    states
}

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

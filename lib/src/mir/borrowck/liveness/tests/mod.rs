use super::super::accesses::AccessKind;
use super::super::borrows::BorrowData;
use super::{
    apply_active_loan_statement_transfer, compute_active_loan_entries,
    compute_active_loan_entries_with_contract, compute_reference_liveness,
    compute_reference_liveness_with_contract, compute_successors, type_id_contains_reference,
    ActiveLoanAnalysis, LoanLocationIndex,
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

fn type_id(type_context: &mut TypeContext, ty: Type) -> crate::ids::TypeId {
    type_context.intern_type(&ty)
}

fn assoc_type(owner: DefId) -> AssociatedTypeKey {
    AssociatedTypeKey {
        owner,
        assoc_type_id: AssocTypeId(0),
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

mod dataflow;
mod owner_release;
mod owner_transfer;
mod reference_types;

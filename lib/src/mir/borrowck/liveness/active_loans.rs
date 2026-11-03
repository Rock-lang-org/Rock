use std::collections::HashMap;

use crate::mir::{
    BasicBlockId, Local, MirBackendContract, MirFunction, MirFunctionId, Operand, Rvalue,
    StatementData, StatementKind, Terminator,
};

use crate::mir::dataflow::analyses::{LoanState, LoanTable};
use crate::mir::dataflow::Analysis;
use crate::type_context::{TypeContext, TypeView};

use super::*;
use crate::mir::borrowck::{MissingSummaryMode, OriginSet, ReferenceEscapeAnalysis};

pub(in crate::mir::borrowck) struct ActiveLoanAnalysis<'a> {
    type_context: &'a TypeContext,
    backend_contract: &'a MirBackendContract,
    return_summaries: &'a HashMap<MirFunctionId, OriginSet>,
    table: &'a LoanTable,
    loan_at_location: &'a LoanLocationIndex,
    reference_liveness: &'a ReferenceLiveness,
}

impl<'a> ActiveLoanAnalysis<'a> {
    pub fn new(
        type_context: &'a TypeContext,
        backend_contract: &'a MirBackendContract,
        return_summaries: &'a HashMap<MirFunctionId, OriginSet>,
        table: &'a LoanTable,
        loan_at_location: &'a LoanLocationIndex,
        reference_liveness: &'a ReferenceLiveness,
    ) -> Self {
        Self {
            type_context,
            backend_contract,
            return_summaries,
            table,
            loan_at_location,
            reference_liveness,
        }
    }
}

impl Analysis for ActiveLoanAnalysis<'_> {
    type Domain = LoanState;

    fn initial_state(&self, _func: &MirFunction) -> Self::Domain {
        LoanState::new(self.table.len())
    }

    fn apply_statement(&self, _state: &mut Self::Domain, _stmt: &StatementData) {
        // Active-loan precision needs the function and statement index; apply_block owns it.
    }

    fn apply_terminator(&self, _state: &mut Self::Domain, _term: &Terminator) {
        // Active-loan precision needs the function and block index; apply_block owns it.
    }

    fn apply_block(&self, state: &mut Self::Domain, func: &MirFunction, block_id: BasicBlockId) {
        let block = &func.basic_blocks[block_id.0];

        for (stmt_idx, stmt) in block.statements.iter().enumerate() {
            apply_active_loan_statement_transfer(
                state,
                func,
                self.type_context,
                self.backend_contract,
                stmt,
            );

            let location = crate::mir::borrowck::location::Location::new(
                block_id,
                crate::mir::borrowck::location::StatementIndex(stmt_idx),
            );
            for loan_id in self.loan_at_location.loan_ids_at(location) {
                state.activate(loan_id, self.table);
            }

            if let Some(live_locals) = self
                .reference_liveness
                .after_statement_sets
                .get(block_id.0)
                .and_then(|stmt_sets| stmt_sets.get(stmt_idx))
            {
                state.retain_owners(|owner| live_locals.contains(owner));
            }

            if let StatementKind::StorageDead(local) = &stmt.kind {
                state.release_owner(*local);
            }
        }

        let terminator_location = crate::mir::borrowck::location::Location::new(
            block_id,
            crate::mir::borrowck::location::StatementIndex(block.statements.len()),
        );
        if let Some(term) = &block.terminator {
            apply_active_loan_terminator_transfer(
                state,
                func,
                self.type_context,
                self.backend_contract,
                self.return_summaries,
                term,
            );
        }
        for loan_id in self.loan_at_location.loan_ids_at(terminator_location) {
            state.activate(loan_id, self.table);
        }

        if let Some(live_locals) = self.reference_liveness.exit_sets.get(block_id.0) {
            state.retain_owners(|owner| live_locals.contains(owner));
        }
    }
}

pub(in crate::mir::borrowck) fn apply_active_loan_statement_transfer(
    state: &mut LoanState,
    func: &MirFunction,
    type_context: &TypeContext,
    backend_contract: &MirBackendContract,
    stmt: &StatementData,
) {
    match &stmt.kind {
        StatementKind::Assign(dest, Rvalue::Use(Operand::Move(src))) => {
            state.transfer_owner(src.local, dest.local);
        }
        StatementKind::Assign(dest, Rvalue::Use(Operand::Copy(src))) => {
            if tracks_local_reference_liveness(func, type_context, backend_contract, src.local) {
                state.copy_owner(src.local, dest.local);
            }
        }
        StatementKind::Assign(dest, Rvalue::Ref(_, place))
            if place.projection.contains(&crate::mir::Projection::Deref)
                && tracks_local_reference_liveness(
                    func,
                    type_context,
                    backend_contract,
                    place.local,
                ) =>
        {
            state.copy_owner(place.local, dest.local);
        }
        StatementKind::Assign(dest, Rvalue::Aggregate(_, operands)) => {
            for operand in operands {
                let (Operand::Copy(place) | Operand::Move(place)) = operand else {
                    continue;
                };
                if !tracks_local_reference_liveness(
                    func,
                    type_context,
                    backend_contract,
                    place.local,
                ) {
                    continue;
                }
                match operand {
                    Operand::Copy(_) => {
                        state.copy_owner(place.local, dest.local);
                    }
                    Operand::Move(_) => {
                        state.transfer_owner(place.local, dest.local);
                    }
                    Operand::Constant(_) => unreachable!(),
                }
            }
        }
        StatementKind::Assign(dest, Rvalue::Cast(op, target)) => {
            if let Operand::Copy(place) | Operand::Move(place) = op {
                if local_is_mutable_reference(func, type_context, place.local)
                    && target_is_pointer(type_context, *target)
                {
                    state.transfer_owner(place.local, dest.local);
                }
            }
        }
        _ => {}
    }
}

fn apply_active_loan_terminator_transfer(
    state: &mut LoanState,
    func: &MirFunction,
    type_context: &TypeContext,
    backend_contract: &MirBackendContract,
    return_summaries: &HashMap<MirFunctionId, OriginSet>,
    term: &Terminator,
) {
    let Terminator::Call {
        func: callee,
        args,
        destination,
        ..
    } = term
    else {
        return;
    };

    let Some(destination_ty) = backend_contract.place_type_id(type_context, func, destination)
    else {
        return;
    };
    let view = TypeView::new(type_context);
    let raw_result =
        crate::mir::borrowck::type_id_contains_pointer(view, backend_contract, destination_ty);
    let reference_result =
        type_id_contains_reference_with_contract(view, backend_contract, destination_ty);
    if !raw_result && !reference_result {
        return;
    }
    let origins = ReferenceEscapeAnalysis::new(
        func,
        type_context,
        backend_contract,
        return_summaries,
        MissingSummaryMode::Conservative,
    );
    for index in origins.call_return_argument_indices(callee, args) {
        let Some(Operand::Copy(source) | Operand::Move(source)) = args.get(index) else {
            continue;
        };
        let Some(source_ty) = backend_contract.place_type_id(type_context, func, source) else {
            continue;
        };
        // Raw forwarding preserves pointer authority, not a temporary borrow of
        // an accessor's receiver. References formed from raw storage instead
        // inherit through their recovered, checked reborrow.
        let forwards_pointer = raw_result
            && crate::mir::borrowck::type_id_contains_pointer(view, backend_contract, source_ty);
        let forwards_reference = reference_result
            && type_id_contains_reference_with_contract(view, backend_contract, source_ty);
        if !forwards_pointer && !forwards_reference {
            continue;
        }
        if tracks_local_reference_liveness(func, type_context, backend_contract, source.local) {
            state.copy_owner(source.local, destination.local);
        }
    }
}

fn local_is_mutable_reference(
    func: &MirFunction,
    type_context: &TypeContext,
    local: Local,
) -> bool {
    let type_view = TypeView::new(type_context);
    func.local_decls
        .get(local.0)
        .is_some_and(|decl| crate::mir::borrowck::type_id_is_mut_reference(type_view, decl.ty))
}

fn target_is_pointer(type_context: &TypeContext, target: crate::ids::TypeId) -> bool {
    crate::mir::borrowck::type_id_is_pointer(TypeView::new(type_context), target)
}

use std::collections::HashMap;

use crate::diagnostic::Diagnostics;
use crate::mir::borrowck::borrows::collect_function_borrows_with_contract;
use crate::mir::borrowck::liveness::{
    apply_active_loan_statement_transfer, compute_active_loan_entries_with_contract,
    compute_reference_liveness_with_contract, LoanLocationIndex,
};
use crate::mir::dataflow::analyses::{InitializationAnalysis, LoanTable};
use crate::mir::dataflow::{run_fixpoint, Analysis};
use crate::mir::{
    DropObligationKind, LocalSource, MirBackendContract, MirFunction, MirFunctionId, StatementKind,
};

use super::*;

impl BorrowChecker {
    pub(super) fn validate_moves_and_initialization(
        func: &MirFunction,
        type_context: &crate::type_context::TypeContext,
        backend_contract: &MirBackendContract,
        return_summaries: &HashMap<MirFunctionId, OriginSet>,
        type_names: &crate::type_services::display::TypeDisplayContext,
    ) -> Diagnostics {
        let mut diagnostics = Diagnostics::default();
        let mut move_validation =
            MoveValidationContext::new(type_context, backend_contract, type_names);

        let init_analysis = InitializationAnalysis::new(func, type_context);
        let init_results = run_fixpoint(&init_analysis, func);

        let borrows = collect_function_borrows_with_contract(
            func,
            type_context,
            backend_contract,
            return_summaries,
        );
        let place_paths = crate::mir::borrowck::paths::PlacePathTable::for_function(func);
        let loan_table = LoanTable::from_borrows_with_paths(&borrows, place_paths);
        let loans_by_location = LoanLocationIndex::for_function(&loan_table, func);
        let reference_liveness =
            compute_reference_liveness_with_contract(func, type_context, backend_contract);
        let active_loan_entries = compute_active_loan_entries_with_contract(
            func,
            type_context,
            backend_contract,
            return_summaries,
            &loan_table,
            &loans_by_location,
            &reference_liveness,
        );

        for (block_idx, block) in func.basic_blocks.iter().enumerate() {
            if !init_results.reachable[block_idx] {
                continue;
            }
            let mut state = init_results.entry_sets[block_idx].clone();
            let mut active_loans = active_loan_entries[block_idx].clone();

            for (stmt_idx, stmt) in block.statements.iter().enumerate() {
                let mut live_loans = active_loans.clone();
                if let Some(live_locals) = reference_liveness
                    .before_statement_sets
                    .get(block_idx)
                    .and_then(|stmt_sets| stmt_sets.get(stmt_idx))
                {
                    live_loans.retain_owners(|owner| live_locals.contains(owner));
                }

                let location = crate::mir::borrowck::location::Location::new(
                    crate::mir::BasicBlockId(block_idx),
                    crate::mir::borrowck::location::StatementIndex(stmt_idx),
                );
                Self::check_created_reborrows(
                    location,
                    &loans_by_location,
                    &loan_table,
                    &live_loans,
                    &mut diagnostics,
                    stmt.span.clone(),
                );
                Self::check_statement_loans(
                    stmt,
                    type_context,
                    &loan_table,
                    &live_loans,
                    func,
                    &mut diagnostics,
                );
                Self::check_statement(
                    stmt,
                    &init_analysis,
                    &state,
                    func,
                    &mut move_validation,
                    &mut diagnostics,
                );

                init_analysis.apply_statement(&mut state, stmt);

                apply_active_loan_statement_transfer(
                    &mut active_loans,
                    func,
                    type_context,
                    backend_contract,
                    stmt,
                );

                let location = crate::mir::borrowck::location::Location::new(
                    crate::mir::BasicBlockId(block_idx),
                    crate::mir::borrowck::location::StatementIndex(stmt_idx),
                );
                for loan_id in loans_by_location.loan_ids_at(location) {
                    active_loans.activate(loan_id, &loan_table);
                }

                if let Some(live_locals) = reference_liveness
                    .after_statement_sets
                    .get(block_idx)
                    .and_then(|stmt_sets| stmt_sets.get(stmt_idx))
                {
                    active_loans.retain_owners(|owner| live_locals.contains(owner));
                }

                if let StatementKind::StorageDead(local) = &stmt.kind {
                    active_loans.release_owner(*local);
                }
            }

            if let Some(term) = &block.terminator {
                let location = crate::mir::borrowck::location::Location::new(
                    crate::mir::BasicBlockId(block_idx),
                    crate::mir::borrowck::location::StatementIndex(block.statements.len()),
                );
                Self::check_created_reborrows(
                    location,
                    &loans_by_location,
                    &loan_table,
                    &active_loans,
                    &mut diagnostics,
                    func.terminator_origin(term).source_span().cloned(),
                );
                Self::check_terminator(
                    term,
                    type_context,
                    &init_analysis,
                    &state,
                    &loan_table,
                    &active_loans,
                    func,
                    &mut diagnostics,
                );
            }
        }

        diagnostics
    }

    pub(super) fn validate_drop_obligations(
        func: &MirFunction,
        backend_contract: &MirBackendContract,
        diagnostics: &mut Diagnostics,
    ) {
        for obligation in &func.ownership.drop_obligations {
            if obligation.kind != DropObligationKind::Direct {
                continue;
            }
            if func
                .local_decls
                .get(obligation.place.local.0)
                .is_some_and(|local| {
                    obligation.place.projection.is_empty()
                        && local.source == LocalSource::ReturnPlace
                })
            {
                continue;
            }
            if backend_contract.drop_glue.contains_key(&obligation.ty) {
                continue;
            }

            diagnostics.push(mir_diagnostic(
                format!(
                    "Missing drop glue for a MIR value in function '{}'",
                    func.name
                ),
                func.local_decls
                    .get(obligation.place.local.0)
                    .and_then(|local| local.span.clone()),
            ));
        }
    }
}

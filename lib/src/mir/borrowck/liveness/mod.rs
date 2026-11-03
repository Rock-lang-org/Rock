use std::collections::HashMap;

use crate::mir::{MirBackendContract, MirFunction, MirFunctionId};

use crate::mir::dataflow::analyses::{LoanState, LoanTable};
use crate::mir::dataflow::LocalSet;
use crate::type_context::TypeContext;

use super::OriginSet;

mod active_loans;
mod location_index;
mod reference_liveness;
mod type_contains_reference;

pub(super) use active_loans::{apply_active_loan_statement_transfer, ActiveLoanAnalysis};
pub use location_index::LoanLocationIndex;
use reference_liveness::*;
use type_contains_reference::*;

#[derive(Clone, Debug)]
pub struct ReferenceLiveness {
    pub entry_sets: Vec<LocalSet>,
    pub exit_sets: Vec<LocalSet>,
    pub before_statement_sets: Vec<Vec<LocalSet>>,
    pub after_statement_sets: Vec<Vec<LocalSet>>,
}

pub fn compute_reference_liveness(
    func: &MirFunction,
    type_context: &TypeContext,
) -> ReferenceLiveness {
    let backend_contract = MirBackendContract::default();
    compute_reference_liveness_with_contract(func, type_context, &backend_contract)
}

pub fn compute_reference_liveness_with_contract(
    func: &MirFunction,
    type_context: &TypeContext,
    backend_contract: &MirBackendContract,
) -> ReferenceLiveness {
    let reference_locals = reference_locals(func, type_context, backend_contract);
    compute_reference_liveness_for(func, &reference_locals)
}

pub fn compute_active_loan_entries(
    func: &MirFunction,
    type_context: &TypeContext,
    table: &LoanTable,
    loan_at_location: &LoanLocationIndex,
    reference_liveness: &ReferenceLiveness,
) -> Vec<LoanState> {
    let backend_contract = MirBackendContract::default();
    compute_active_loan_entries_with_contract(
        func,
        type_context,
        &backend_contract,
        &HashMap::new(),
        table,
        loan_at_location,
        reference_liveness,
    )
}

pub fn compute_active_loan_entries_with_contract(
    func: &MirFunction,
    type_context: &TypeContext,
    backend_contract: &MirBackendContract,
    return_summaries: &HashMap<MirFunctionId, OriginSet>,
    table: &LoanTable,
    loan_at_location: &LoanLocationIndex,
    reference_liveness: &ReferenceLiveness,
) -> Vec<LoanState> {
    let analysis = ActiveLoanAnalysis::new(
        type_context,
        backend_contract,
        return_summaries,
        table,
        loan_at_location,
        reference_liveness,
    );
    crate::mir::dataflow::run_fixpoint(&analysis, func).entry_sets
}

#[cfg(test)]
mod tests;

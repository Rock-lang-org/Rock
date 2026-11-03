use std::collections::HashSet;

use crate::ids::PlacePathId;
use crate::mir::dataflow::analyses::{LoanState, LoanTable};
use crate::mir::{Place, Projection};

pub fn borrowed_root(place: &Place) -> Place {
    let mut root = place.clone();

    while matches!(root.projection.first(), Some(Projection::Deref)) {
        root.projection.remove(0);
    }

    root
}

pub fn resolve_place(place: &Place, table: &LoanTable, active_loans: &LoanState) -> Place {
    let mut resolved = place.clone();
    let mut seen = HashSet::new();

    loop {
        let Some(Projection::Deref) = resolved.projection.first() else {
            break;
        };

        if !seen.insert(resolved.local) {
            break;
        }

        let Some(loan_id) = active_loans
            .active_ids()
            .find(|loan_id| active_loans.owner_contains(*loan_id, resolved.local))
        else {
            break;
        };

        let Some(loan) = table.get(loan_id) else {
            break;
        };

        let Some(indexed_place) = table.place_paths().place(loan.place_path) else {
            break;
        };

        let mut next = indexed_place.clone();
        next.projection
            .extend(resolved.projection.iter().skip(1).cloned());
        resolved = next;
    }

    resolved
}

pub fn resolve_place_path(
    place: &Place,
    table: &LoanTable,
    active_loans: &LoanState,
) -> Option<PlacePathId> {
    let mut resolved = place.clone();
    let mut seen = HashSet::new();

    loop {
        let Some(Projection::Deref) = resolved.projection.first() else {
            break;
        };

        if !seen.insert(resolved.local) {
            break;
        }

        let Some(loan_id) = active_loans
            .active_ids()
            .find(|loan_id| active_loans.owner_contains(*loan_id, resolved.local))
        else {
            break;
        };

        let Some(loan) = table.get(loan_id) else {
            break;
        };

        let mut next = table.place_paths().place(loan.place_path)?.clone();
        next.projection
            .extend(resolved.projection.iter().skip(1).cloned());
        resolved = next;
    }

    table.place_paths().path_id(&resolved)
}

#[cfg(test)]
mod tests;

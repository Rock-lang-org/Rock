use std::collections::HashSet;

use crate::ids::PlacePathId;
use crate::mir::dataflow::analyses::{LoanState, LoanTable};
use crate::mir::{Local, Place, Projection};

pub fn borrowed_root(place: &Place) -> Place {
    let mut root = place.clone();

    while matches!(root.projection.first(), Some(Projection::Deref)) {
        root.projection.remove(0);
    }

    root
}

pub fn resolve_place(place: &Place, table: &LoanTable, active_loans: &LoanState) -> Place {
    resolve_with_seen(place, table, active_loans, &mut HashSet::new())
}

pub fn resolve_place_path(
    place: &Place,
    table: &LoanTable,
    active_loans: &LoanState,
) -> Option<PlacePathId> {
    table
        .place_paths()
        .path_id(&resolve_place(place, table, active_loans))
}

/// Replaces leading dereferences of reference locals by the places their
/// active loans borrow, until the place no longer goes through a known loan.
fn resolve_with_seen(
    place: &Place,
    table: &LoanTable,
    active_loans: &LoanState,
    seen: &mut HashSet<Local>,
) -> Place {
    let mut resolved = place.clone();
    while let Some(Projection::Deref) = resolved.projection.first() {
        if !seen.insert(resolved.local) {
            break;
        }
        let Some(borrowed) = borrowed_place_of(resolved.local, table, active_loans, seen) else {
            break;
        };
        let mut next = borrowed;
        next.projection
            .extend(resolved.projection.iter().skip(1).cloned());
        resolved = next;
    }
    resolved
}

/// The place `local` points to, from the active loans it owns.
///
/// A reborrow also owns the loans of the reference it was derived from, so
/// `a = &mut (*r).0` owns both the loan of `r`'s target and the field loan.
/// The most specific one (lying inside every other) is what `a` points to;
/// resolving to the wider loan would make disjoint field borrows conflict.
fn borrowed_place_of(
    local: Local,
    table: &LoanTable,
    active_loans: &LoanState,
    seen: &HashSet<Local>,
) -> Option<Place> {
    let candidates: Vec<Place> = active_loans
        .active_ids()
        .filter(|loan_id| active_loans.owner_contains(*loan_id, local))
        .filter_map(|loan_id| table.get(loan_id))
        .filter_map(|loan| table.place_paths().place(loan.place_path).cloned())
        .collect();
    let first = candidates.first()?.clone();
    if candidates.len() == 1 {
        return Some(first);
    }
    let resolved: Vec<Place> = candidates
        .iter()
        .map(|place| resolve_with_seen(place, table, active_loans, &mut seen.clone()))
        .collect();
    let within = |inner: &Place, outer: &Place| {
        inner.local == outer.local && inner.projection.starts_with(&outer.projection)
    };
    resolved
        .iter()
        .position(|candidate| resolved.iter().all(|other| within(candidate, other)))
        .map_or(Some(first), |index| Some(resolved[index].clone()))
}

#[cfg(test)]
mod tests;

use std::collections::HashSet;

use crate::ids::{LoanId, PlacePathId};
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

/// An interface origin union is a set of possible targets, not an arbitrary
/// first target. Inspect every carrier's semantic path when checking aliases.
pub fn resolve_places(place: &Place, table: &LoanTable, active_loans: &LoanState) -> Vec<Place> {
    let mut pending = vec![(place.clone(), HashSet::new())];
    let mut resolved = HashSet::new();
    while let Some((place, mut seen)) = pending.pop() {
        if !matches!(place.projection.first(), Some(Projection::Deref))
            || !seen.insert(place.local)
        {
            resolved.insert(place);
            continue;
        }
        let mut found = false;
        for loan_id in active_loans.active_ids() {
            if !active_loans.owner_contains(loan_id, place.local) { continue; }
            let Some(source) = table.get(loan_id)
                .and_then(|loan| table.place_paths().place(loan.place_path)) else { continue; };
            let mut next = source.clone();
            if table.is_returned_view(loan_id) {
                // Result slots are temporary. The current carrier retains the
                // same checked view and its full origin envelope after a move.
                next.local = place.local;
            }
            next.projection.extend(place.projection.iter().skip(1).cloned());
            pending.push((next, seen.clone()));
            found = true;
        }
        if !found { resolved.insert(place); }
    }
    resolved.into_iter().collect()
}

pub fn resolve_loan_places(id: LoanId, table: &LoanTable, active_loans: &LoanState) -> Vec<Place> {
    let Some(place) = table.get(id).and_then(|loan| table.place_paths().place(loan.place_path)) else {
        return Vec::new();
    };
    if !table.is_returned_view(id) { return resolve_places(place, table, active_loans); }
    let mut resolved = HashSet::new();
    for owner in active_loans.owners(id).iter() {
        let mut view = place.clone();
        view.local = owner;
        resolved.extend(resolve_places(&view, table, active_loans));
    }
    resolved.into_iter().collect()
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
mod tests {
    use super::{borrowed_root, resolve_place, resolve_place_path};
    use crate::ids::LoanId;
    use crate::mir::borrowck::location::{Location, StatementIndex};
    use crate::mir::borrowck::paths::PlacePathTable;
    use crate::mir::dataflow::analyses::{LoanData, LoanKind, LoanState, LoanTable};
    use crate::mir::{BasicBlockId, Local, Place, Projection};

    #[test]
    fn test_borrowed_root_peels_leading_deref() {
        let place = Place {
            local: Local(2),
            projection: vec![
                Projection::Deref,
                Projection::Field {
                    index: 0,
                    identity: None,
                },
            ],
        };

        assert_eq!(
            borrowed_root(&place),
            Place {
                local: Local(2),
                projection: vec![Projection::Field {
                    index: 0,
                    identity: None,
                }],
            }
        );
    }

    #[test]
    fn test_resolve_place_reborrow_uses_owner_loan() {
        let mut table = LoanTable::new();
        let id = table.push(LoanData {
            id: LoanId(99),
            diagnostic_place: Place {
                local: Local(1),
                projection: vec![],
            },
            place_path: crate::ids::PlacePathId(0),
            initial_owner: Local(2),
            kind: LoanKind::Shared,
            origin_span: None,
            created_at: Location::new(BasicBlockId(0), StatementIndex(0)),
        });
        let mut active = LoanState::new(table.len());
        active.activate(id, &table);

        let place = Place {
            local: Local(2),
            projection: vec![Projection::Deref],
        };

        assert_eq!(
            resolve_place(&place, &table, &active),
            Place {
                local: Local(1),
                projection: vec![],
            }
        );
    }

    #[test]
    fn resolve_place_path_uses_indexed_loan_path_not_diagnostic_place() {
        let semantic_place = Place {
            local: Local(1),
            projection: vec![Projection::Field {
                index: 0,
                identity: None,
            }],
        };
        let diagnostic_place = Place {
            local: Local(1),
            projection: vec![Projection::Field {
                index: 1,
                identity: None,
            }],
        };
        let mut paths = PlacePathTable::new();
        let semantic_path = paths.intern(semantic_place);
        paths.intern(diagnostic_place.clone());

        let mut table = LoanTable::new();
        table.replace_place_paths_for_test(paths);
        let id = table.push(LoanData {
            id: LoanId(99),
            diagnostic_place,
            place_path: semantic_path,
            initial_owner: Local(2),
            kind: LoanKind::Shared,
            origin_span: None,
            created_at: Location::new(BasicBlockId(0), StatementIndex(0)),
        });
        let mut active = LoanState::new(table.len());
        active.activate(id, &table);

        let reborrow = Place {
            local: Local(2),
            projection: vec![Projection::Deref],
        };

        assert_eq!(
            resolve_place_path(&reborrow, &table, &active),
            Some(semantic_path)
        );
    }
}

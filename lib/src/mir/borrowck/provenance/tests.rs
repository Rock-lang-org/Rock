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
        reborrow_source: None,
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
        reborrow_source: None,
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

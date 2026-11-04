use super::*;

use crate::ids::LoanId;
use crate::mir::borrowck::location::{Location, StatementIndex};
use crate::mir::{BasicBlockId, Local, Place};

fn loan_data(id: LoanId, owner: Local) -> LoanData {
    LoanData {
        id,
        reborrow_source: None,
        diagnostic_place: Place {
            local: Local(1),
            projection: vec![],
        },
        place_path: crate::ids::PlacePathId(0),
        initial_owner: owner,
        kind: LoanKind::Shared,
        origin_span: None,
        created_at: Location::new(BasicBlockId(0), StatementIndex(0)),
    }
}

#[test]
fn loan_table_allocates_crate_loan_ids() {
    let mut table = LoanTable::new();

    let id = table.push(loan_data(LoanId(999), Local(2)));

    assert_eq!(id, LoanId(0));
    assert_eq!(table.get(id).expect("loan data").id, id);
    assert_eq!(table.len(), 1);
}

#[test]
fn loan_table_builds_from_collected_borrows_by_location() {
    let stmt = crate::mir::StatementData::assign(
        crate::mir::Place {
            local: crate::mir::Local(2),
            projection: vec![],
        },
        crate::mir::Rvalue::Ref(
            crate::mir::Mutability::Not,
            crate::mir::Place {
                local: crate::mir::Local(1),
                projection: vec![],
            },
        ),
        None,
    );
    let borrow = crate::mir::borrowck::borrows::collect_statement_borrows(
        &stmt,
        crate::mir::borrowck::location::Location::new(
            crate::mir::BasicBlockId(0),
            crate::mir::borrowck::location::StatementIndex(0),
        ),
    )
    .pop()
    .expect("borrow");

    let borrows = vec![borrow];
    let table = LoanTable::from_borrows(&borrows);

    let loan = table.get(crate::ids::LoanId(0)).expect("loan data");
    assert_eq!(loan.initial_owner, crate::mir::Local(2));
    assert_eq!(loan.diagnostic_place.local, crate::mir::Local(1));
    assert_eq!(loan.created_at.block, crate::mir::BasicBlockId(0));
}

#[test]
fn loan_table_records_indexed_place_paths() {
    use crate::mir::borrowck::borrows::BorrowData;
    use crate::mir::borrowck::paths::PlacePathTable;

    let place = Place {
        local: Local(1),
        projection: Vec::new(),
    };
    let mut paths = PlacePathTable::new();
    let place_path = paths.intern(place.clone());
    let borrows = vec![BorrowData {
        owner: Local(2),
        reborrow_source: None,
        place,
        kind: crate::mir::borrowck::accesses::AccessKind::BorrowShared,
        created_at: Location::new(BasicBlockId(0), StatementIndex(0)),
        origin_span: None,
    }];

    let table = LoanTable::from_borrows_with_paths(&borrows, paths);

    assert_eq!(
        table.get(crate::ids::LoanId(0)).unwrap().place_path,
        place_path
    );
}

#[test]
fn loan_table_resolves_reborrow_place_path_from_prior_owner() {
    use crate::mir::borrowck::borrows::BorrowData;
    use crate::mir::borrowck::paths::PlacePathTable;
    use crate::mir::Projection;

    let root_place = Place {
        local: Local(1),
        projection: Vec::new(),
    };
    let reborrow_place = Place {
        local: Local(2),
        projection: vec![Projection::Deref],
    };
    let mut paths = PlacePathTable::new();
    let root_path = paths.intern(root_place.clone());
    paths.intern(reborrow_place.clone());
    let borrows = vec![
        BorrowData {
            owner: Local(2),
            reborrow_source: None,
            place: root_place,
            kind: crate::mir::borrowck::accesses::AccessKind::BorrowMut,
            created_at: Location::new(BasicBlockId(0), StatementIndex(0)),
            origin_span: None,
        },
        BorrowData {
            owner: Local(3),
            reborrow_source: None,
            place: reborrow_place,
            kind: crate::mir::borrowck::accesses::AccessKind::BorrowMut,
            created_at: Location::new(BasicBlockId(0), StatementIndex(1)),
            origin_span: None,
        },
    ];

    let table = LoanTable::from_borrows_with_paths(&borrows, paths);

    assert_eq!(table.get(LoanId(0)).unwrap().place_path, root_path);
    assert_eq!(table.get(LoanId(1)).unwrap().place_path, root_path);
}

#[test]
fn loan_table_push_preserves_existing_supplied_place_path() {
    use crate::mir::borrowck::paths::PlacePathTable;
    use crate::mir::Projection;

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

    assert_eq!(table.get(id).expect("loan data").place_path, semantic_path);
}

#[test]
fn loan_state_activates_transfers_releases_and_joins_owners() {
    let mut table = LoanTable::new();
    let first = table.push(loan_data(LoanId(99), Local(2)));
    let second = table.push(loan_data(LoanId(99), Local(4)));
    let mut left = LoanState::new(table.len());
    let mut right = LoanState::new(table.len());

    left.activate(first, &table);
    right.activate(first, &table);
    right.activate(second, &table);
    right.transfer_owner(Local(2), Local(3));

    assert!(left.join(&right));
    assert!(left.is_active(first));
    assert!(left.is_active(second));
    assert!(left.owners(first).contains(Local(2)));
    assert!(left.owners(first).contains(Local(3)));
    assert!(left.release_owner(Local(2)));
    assert!(!left.owners(first).contains(Local(2)));
}

#[test]
fn check_aliasing_uses_indexed_loan_state() {
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
        kind: LoanKind::Mut,
        origin_span: None,
        created_at: Location::new(BasicBlockId(0), StatementIndex(0)),
    });
    let mut active = LoanState::new(table.len());
    active.activate(id, &table);

    let place = Place {
        local: Local(1),
        projection: vec![],
    };

    assert_eq!(
        LoanAnalysis::check_aliasing(&place, LoanKind::Mut, &table, &active),
        Err(id)
    );
    assert_eq!(
        LoanAnalysis::check_aliasing(&place, LoanKind::Shared, &table, &active),
        Err(id)
    );
    let through_owner = Place {
        local: Local(2),
        projection: vec![Projection::Deref],
    };
    assert_eq!(
        LoanAnalysis::check_aliasing(&through_owner, LoanKind::Shared, &table, &active),
        Ok(())
    );
    assert_eq!(
        LoanAnalysis::check_aliasing(&through_owner, LoanKind::Mut, &table, &active),
        Ok(())
    );
}

#[test]
fn check_aliasing_uses_indexed_loan_path_when_diagnostic_place_differs() {
    use crate::mir::borrowck::paths::PlacePathTable;
    use crate::mir::Projection;

    let loan_semantic_place = Place {
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
    let mut place_paths = PlacePathTable::new();
    let loan_path = place_paths.intern(loan_semantic_place);
    place_paths.intern(diagnostic_place.clone());

    let table = LoanTable {
        loans: vec![LoanData {
            id: LoanId(0),
            diagnostic_place: diagnostic_place.clone(),
            reborrow_source: None,
            place_path: loan_path,
            initial_owner: Local(2),
            kind: LoanKind::Mut,
            origin_span: None,
            created_at: Location::new(BasicBlockId(0), StatementIndex(0)),
        }],
        place_paths,
    };
    let mut active = LoanState::new(table.len());
    active.activate(LoanId(0), &table);

    assert!(
        LoanAnalysis::check_aliasing(&diagnostic_place, LoanKind::Mut, &table, &active).is_ok()
    );
}

#[test]
fn check_aliasing_resolves_indexed_path_from_original_reborrow() {
    use crate::mir::borrowck::paths::PlacePathTable;
    use crate::mir::Projection;

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
    let diagnostic_path = paths.intern(diagnostic_place.clone());

    let mut table = LoanTable::new();
    table.replace_place_paths_for_test(paths);
    let owner_id = table.push(LoanData {
        id: LoanId(99),
        reborrow_source: None,
        diagnostic_place: diagnostic_place.clone(),
        place_path: semantic_path,
        initial_owner: Local(2),
        kind: LoanKind::Mut,
        origin_span: None,
        created_at: Location::new(BasicBlockId(0), StatementIndex(0)),
    });
    let diagnostic_id = table.push(LoanData {
        id: LoanId(99),
        reborrow_source: None,
        diagnostic_place,
        place_path: diagnostic_path,
        initial_owner: Local(3),
        kind: LoanKind::Mut,
        origin_span: None,
        created_at: Location::new(BasicBlockId(0), StatementIndex(0)),
    });
    let mut active = LoanState::new(table.len());
    active.activate(owner_id, &table);
    active.activate(diagnostic_id, &table);

    let reborrow_access = Place {
        local: Local(2),
        projection: vec![Projection::Deref],
    };

    assert!(LoanAnalysis::check_aliasing(&reborrow_access, LoanKind::Mut, &table, &active).is_ok());
}

#[test]
fn check_aliasing_ignores_diagnostic_place_when_indexed_loan_path_is_stale() {
    use crate::mir::borrowck::paths::PlacePathTable;

    let place = Place {
        local: Local(1),
        projection: vec![],
    };
    let mut place_paths = PlacePathTable::new();
    place_paths.intern(place.clone());
    let table = LoanTable {
        loans: vec![LoanData {
            id: LoanId(0),
            diagnostic_place: place.clone(),
            reborrow_source: None,
            place_path: crate::ids::PlacePathId(999),
            initial_owner: Local(2),
            kind: LoanKind::Shared,
            origin_span: None,
            created_at: Location::new(BasicBlockId(0), StatementIndex(0)),
        }],
        place_paths,
    };
    let mut active = LoanState::new(table.len());
    active.activate(LoanId(0), &table);

    assert!(LoanAnalysis::check_aliasing(&place, LoanKind::Mut, &table, &active).is_ok());
}

impl LoanTable {
    pub(crate) fn replace_place_paths_for_test(&mut self, place_paths: PlacePathTable) {
        self.place_paths = place_paths;
    }
}

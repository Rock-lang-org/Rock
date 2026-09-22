//! Loan (borrow) tracking analysis.
//!
//! Tracks active loans (borrows) and checks for aliasing violations.

use crate::ids::{Idx, LoanId, PlacePathId};
use crate::mir::borrowck::borrows::collect_function_borrows;
use crate::mir::borrowck::location::Location;
use crate::mir::borrowck::paths::PlacePathTable;
use crate::mir::dataflow::{BitSet, Lattice, LocalSet};
use crate::mir::{Local, MirFunction, Mutability, Place, Projection};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanData {
    pub id: LoanId,
    /// Original structural place retained for diagnostics; semantic checks use `place_path`.
    pub diagnostic_place: Place,
    pub place_path: PlacePathId,
    pub initial_owner: Local,
    pub kind: LoanKind,
    pub origin_span: Option<crate::lexer::Span>,
    pub created_at: Location,
}

#[derive(Debug, Default)]
pub struct LoanTable {
    loans: Vec<LoanData>,
    place_paths: PlacePathTable,
}

impl LoanTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_borrows(borrows: &[crate::mir::borrowck::borrows::BorrowData]) -> Self {
        Self::from_borrows_with_paths(borrows, PlacePathTable::new())
    }

    pub fn from_borrows_with_paths(
        borrows: &[crate::mir::borrowck::borrows::BorrowData],
        place_paths: PlacePathTable,
    ) -> Self {
        let mut table = Self::new();
        table.place_paths = place_paths;
        let mut owner_paths: Vec<Option<PlacePathId>> = Vec::new();
        for borrow in borrows {
            let kind = match borrow.kind {
                crate::mir::borrowck::accesses::AccessKind::BorrowShared => LoanKind::Shared,
                crate::mir::borrowck::accesses::AccessKind::BorrowMut => LoanKind::Mut,
                _ => continue,
            };
            let place_path = table.resolve_borrow_place_path(&borrow.place, &owner_paths);
            table.push(LoanData {
                id: crate::ids::LoanId(0),
                diagnostic_place: borrow.place.clone(),
                place_path,
                initial_owner: borrow.owner,
                kind,
                origin_span: borrow.origin_span.clone(),
                created_at: borrow.created_at,
            });
            let owner_index = borrow.owner.0;
            if owner_paths.len() <= owner_index {
                owner_paths.resize(owner_index + 1, None);
            }
            owner_paths[owner_index] = Some(place_path);
        }
        table
    }

    fn resolve_borrow_place_path(
        &mut self,
        place: &Place,
        owner_paths: &[Option<PlacePathId>],
    ) -> PlacePathId {
        let mut resolved = place.clone();
        while matches!(resolved.projection.first(), Some(Projection::Deref)) {
            let Some(Some(owner_path)) = owner_paths.get(resolved.local.0) else {
                break;
            };
            let Some(owner_place) = self.place_paths.place(*owner_path) else {
                break;
            };

            let mut next = owner_place.clone();
            next.projection
                .extend(resolved.projection.iter().skip(1).cloned());
            if next == resolved {
                break;
            }
            resolved = next;
        }
        self.place_paths.intern(resolved)
    }

    pub fn push(&mut self, mut loan: LoanData) -> LoanId {
        let id = LoanId(self.loans.len() as u32);
        loan.id = id;
        if self.place_paths.place(loan.place_path).is_none() {
            loan.place_path = self.place_paths.intern(loan.diagnostic_place.clone());
        }
        self.loans.push(loan);
        id
    }

    pub fn get(&self, id: LoanId) -> Option<&LoanData> {
        self.loans.get(id.index())
    }

    pub fn iter(&self) -> impl Iterator<Item = &LoanData> {
        self.loans.iter()
    }

    pub fn place_paths(&self) -> &PlacePathTable {
        &self.place_paths
    }

    #[cfg(test)]
    pub(crate) fn replace_place_paths_for_test(&mut self, place_paths: PlacePathTable) {
        self.place_paths = place_paths;
    }

    pub fn len(&self) -> usize {
        self.loans.len()
    }

    pub fn is_empty(&self) -> bool {
        self.loans.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanState {
    active: BitSet<LoanId>,
    owners: Vec<LocalSet>,
}

impl LoanState {
    pub fn new(loan_count: usize) -> Self {
        Self {
            active: BitSet::with_capacity(loan_count),
            owners: (0..loan_count).map(|_| LocalSet::new()).collect(),
        }
    }

    pub fn is_active(&self, id: LoanId) -> bool {
        self.active.contains(id)
            && self
                .owners
                .get(id.index())
                .is_some_and(|owners| !owners.is_empty())
    }

    pub fn activate(&mut self, id: LoanId, table: &LoanTable) -> bool {
        let Some(loan) = table.get(id) else {
            return false;
        };
        self.ensure_len(id.index() + 1);
        let active_changed = self.active.insert(id);
        let owner_changed = self.owners[id.index()].insert(loan.initial_owner);
        active_changed || owner_changed
    }

    pub fn add_owner(&mut self, id: LoanId, owner: Local) -> bool {
        self.ensure_len(id.index() + 1);
        let active_changed = self.active.insert(id);
        let owner_changed = self.owners[id.index()].insert(owner);
        active_changed || owner_changed
    }

    pub fn owners(&self, id: LoanId) -> &LocalSet {
        &self.owners[id.index()]
    }

    pub fn active_ids(&self) -> impl Iterator<Item = LoanId> + '_ {
        self.active.iter().filter(|id| self.is_active(*id))
    }

    pub fn owner_contains(&self, id: LoanId, local: Local) -> bool {
        self.owners
            .get(id.index())
            .is_some_and(|owners| owners.contains(local))
    }

    pub fn transfer_owner(&mut self, from: Local, to: Local) -> bool {
        let mut changed = false;
        for owners in &mut self.owners {
            if owners.remove(from) {
                changed = true;
                changed |= owners.insert(to);
            }
        }
        changed
    }

    pub fn copy_owner(&mut self, from: Local, to: Local) -> bool {
        let mut changed = false;
        for owners in &mut self.owners {
            if owners.contains(from) {
                changed |= owners.insert(to);
            }
        }
        changed
    }

    pub fn release_owner(&mut self, local: Local) -> bool {
        let mut changed = false;
        for (index, owners) in self.owners.iter_mut().enumerate() {
            if owners.remove(local) {
                changed = true;
            }
            if owners.is_empty() {
                changed |= self.active.remove(LoanId(index as u32));
            }
        }
        changed
    }

    pub fn retain_owners<F>(&mut self, mut keep: F) -> bool
    where
        F: FnMut(Local) -> bool,
    {
        let mut changed = false;
        for (index, owners) in self.owners.iter_mut().enumerate() {
            let current: Vec<_> = owners.iter().collect();
            for owner in current {
                if !keep(owner) {
                    changed |= owners.remove(owner);
                }
            }
            if owners.is_empty() {
                changed |= self.active.remove(LoanId(index as u32));
            }
        }
        changed
    }

    pub fn join(&mut self, other: &Self) -> bool {
        let mut changed = self.active.join(&other.active);
        if self.owners.len() < other.owners.len() {
            self.owners.resize_with(other.owners.len(), LocalSet::new);
            changed = true;
        }
        for (index, owners) in other.owners.iter().enumerate() {
            changed |= self.owners[index].join(owners);
        }
        changed
    }

    fn ensure_len(&mut self, len: usize) {
        if self.owners.len() < len {
            self.owners.resize_with(len, LocalSet::new);
        }
    }
}

impl Lattice for LoanState {
    fn join(&mut self, other: &Self) -> bool {
        LoanState::join(self, other)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanKind {
    Shared,
    Mut,
}

impl From<Mutability> for LoanKind {
    fn from(m: Mutability) -> Self {
        match m {
            Mutability::Not => LoanKind::Shared,
            Mutability::Mut => LoanKind::Mut,
        }
    }
}

pub struct LoanAnalysis;

impl LoanAnalysis {
    /// Collect all loans created in a function.
    pub fn collect_loans(func: &MirFunction) -> LoanTable {
        let borrows = collect_function_borrows(func);
        LoanTable::from_borrows(&borrows)
    }

    /// Check for aliasing violations at a given state.
    pub fn check_aliasing(
        place: &Place,
        kind: LoanKind,
        table: &LoanTable,
        active_loans: &LoanState,
    ) -> Result<(), LoanId> {
        let original_place = place.clone();
        let place = crate::mir::borrowck::provenance::resolve_place(place, table, active_loans);
        let access_path = crate::mir::borrowck::provenance::resolve_place_path(
            &original_place,
            table,
            active_loans,
        );

        for loan_id in active_loans.active_ids() {
            let Some(loan) = table.get(loan_id) else {
                continue;
            };
            if Self::access_through_mutable_loan_owner(&original_place, loan_id, loan, active_loans)
            {
                continue;
            }

            // Check if loans conflict
            let places_conflict = if let (Some(access_path), Some(loan_path)) = (
                access_path,
                Self::resolve_loan_path(loan, table, active_loans),
            ) {
                table.place_paths().paths_conflict(access_path, loan_path)
            } else if let Some(loan_place) = table.place_paths().place(loan.place_path) {
                let resolved_loan_place = crate::mir::borrowck::provenance::resolve_place(
                    loan_place,
                    table,
                    active_loans,
                );
                crate::mir::borrowck::conflicts::places_conflict(&place, &resolved_loan_place)
            } else {
                false
            };

            if places_conflict {
                match (kind, loan.kind) {
                    (LoanKind::Mut, _) | (_, LoanKind::Mut) => {
                        return Err(loan_id);
                    }
                    (LoanKind::Shared, LoanKind::Shared) => {
                        // Multiple shared loans are OK
                    }
                }
            }
        }
        Ok(())
    }

    fn access_through_mutable_loan_owner(
        place: &Place,
        loan_id: LoanId,
        loan: &LoanData,
        active_loans: &LoanState,
    ) -> bool {
        // The owner of an exclusive loan may both read and write through it.
        loan.kind == LoanKind::Mut
            && active_loans.owner_contains(loan_id, place.local)
            && matches!(place.projection.first(), Some(Projection::Deref))
    }

    fn resolve_loan_path(
        loan: &LoanData,
        table: &LoanTable,
        active_loans: &LoanState,
    ) -> Option<PlacePathId> {
        let Some(loan_place) = table.place_paths().place(loan.place_path) else {
            return None;
        };
        let resolved_loan_path =
            crate::mir::borrowck::provenance::resolve_place_path(loan_place, table, active_loans);
        let loan_path = resolved_loan_path.unwrap_or(loan.place_path);

        table.place_paths().place(loan_path).map(|_| loan_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ids::LoanId;
    use crate::mir::borrowck::location::{Location, StatementIndex};
    use crate::mir::{BasicBlockId, Local, Place};

    fn loan_data(id: LoanId, owner: Local) -> LoanData {
        LoanData {
            id,
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
                place: root_place,
                kind: crate::mir::borrowck::accesses::AccessKind::BorrowMut,
                created_at: Location::new(BasicBlockId(0), StatementIndex(0)),
                origin_span: None,
            },
            BorrowData {
                owner: Local(3),
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
            diagnostic_place: diagnostic_place.clone(),
            place_path: semantic_path,
            initial_owner: Local(2),
            kind: LoanKind::Mut,
            origin_span: None,
            created_at: Location::new(BasicBlockId(0), StatementIndex(0)),
        });
        let diagnostic_id = table.push(LoanData {
            id: LoanId(99),
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

        assert!(
            LoanAnalysis::check_aliasing(&reborrow_access, LoanKind::Mut, &table, &active).is_ok()
        );
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
}

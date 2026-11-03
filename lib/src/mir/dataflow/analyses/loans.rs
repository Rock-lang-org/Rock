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
    pub reborrow_source: Option<Place>,
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
                reborrow_source: borrow.reborrow_source.clone(),
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
        let inherited = loan
            .reborrow_source
            .as_ref()
            .is_some_and(|source| self.copy_owner(source.local, loan.initial_owner));
        let active_changed = self.active.insert(id);
        let owner_changed = self.owners[id.index()].insert(loan.initial_owner);
        inherited || active_changed || owner_changed
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
        Self::check_aliasing_through(place, kind, place, table, active_loans)
    }

    /// Check a recovered storage root while retaining the reborrow authority of
    /// the pointer/reference used to reach it; sibling loans still conflict.
    pub fn check_aliasing_through(
        place: &Place,
        kind: LoanKind,
        through: &Place,
        table: &LoanTable,
        active_loans: &LoanState,
    ) -> Result<(), LoanId> {
        Self::check_aliasing_with_owner(
            place,
            kind,
            through
                .projection
                .contains(&Projection::Deref)
                .then_some(through.local),
            table,
            active_loans,
        )
    }

    /// A selected call can reborrow an aggregate's carried storage without
    /// dereferencing the aggregate. Authority belongs to that input's owner.
    pub fn check_reborrow(
        place: &Place,
        kind: LoanKind,
        source: Local,
        table: &LoanTable,
        active_loans: &LoanState,
    ) -> Result<(), LoanId> {
        Self::check_aliasing_with_owner(place, kind, Some(source), table, active_loans)
    }

    fn check_aliasing_with_owner(
        place: &Place,
        kind: LoanKind,
        owner: Option<Local>,
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
            if loan.kind == LoanKind::Mut
                && owner.is_some_and(|owner| active_loans.owner_contains(loan_id, owner))
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
mod tests;

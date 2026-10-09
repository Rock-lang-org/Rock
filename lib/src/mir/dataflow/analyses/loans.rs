//! Loan (borrow) tracking analysis.
//!
//! Tracks active loans (borrows) and checks for aliasing violations.

use std::collections::HashSet;

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
    returned_views: HashSet<LoanId>,
}

impl LoanTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_borrows(borrows: &[crate::mir::borrowck::borrows::BorrowData]) -> Self {
        Self::from_borrows_with_paths(borrows, PlacePathTable::new())
    }

    /// Reserve conservative root identities for a descriptor-backed body.
    /// Reservation activates no loan: checked body transfers supply its owners.
    /// Field disjointness is unavailable until the descriptor proves it.
    pub(crate) fn root_envelopes(local_count: usize) -> Option<Self> {
        u32::try_from(local_count.checked_mul(2)?).ok()?;
        let mut table = Self::new();
        for index in 0..local_count {
            let place = Place {
                local: Local(index),
                projection: vec![],
            };
            let place_path = table.place_paths.intern(place.clone());
            for kind in [LoanKind::Shared, LoanKind::Mut] {
                table.push(LoanData {
                    id: LoanId(0),
                    diagnostic_place: place.clone(),
                    place_path,
                    initial_owner: Local(index),
                    kind,
                    origin_span: None,
                    created_at: Location::new(
                        crate::mir::BasicBlockId(0),
                        crate::mir::borrowck::location::StatementIndex(0),
                    ),
                });
            }
        }
        Some(table)
    }

    pub(crate) fn root_envelope_id(&self, root: Local, kind: LoanKind) -> Option<LoanId> {
        let index = root
            .0
            .checked_mul(2)?
            .checked_add(usize::from(kind == LoanKind::Mut))?;
        let id = LoanId(u32::try_from(index).ok()?);
        let loan = self.get(id)?;
        (loan.kind == kind
            && loan.diagnostic_place.local == root
            && loan.diagnostic_place.projection.is_empty())
        .then_some(id)
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
                crate::mir::borrowck::accesses::AccessKind::BorrowShared
                | crate::mir::borrowck::accesses::AccessKind::ReturnedShared => LoanKind::Shared,
                crate::mir::borrowck::accesses::AccessKind::BorrowMut
                | crate::mir::borrowck::accesses::AccessKind::ReturnedMut => LoanKind::Mut,
                _ => continue,
            };
            let returned_view = matches!(
                borrow.kind,
                crate::mir::borrowck::accesses::AccessKind::ReturnedShared
                    | crate::mir::borrowck::accesses::AccessKind::ReturnedMut
            );
            let place_path = if returned_view {
                // Resolve this result-relative view through its actual dataflow
                // envelopes, never through the previous value of a reused slot.
                table.place_paths.intern(borrow.place.clone())
            } else {
                table.resolve_borrow_place_path(&borrow.place, &owner_paths)
            };
            let id = table.push(LoanData {
                id: crate::ids::LoanId(0),
                diagnostic_place: borrow.place.clone(),
                place_path,
                initial_owner: borrow.owner,
                kind,
                origin_span: borrow.origin_span.clone(),
                created_at: borrow.created_at,
            });
            if returned_view {
                table.returned_views.insert(id);
                continue;
            }
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

    pub(crate) fn is_returned_view(&self, id: LoanId) -> bool {
        self.returned_views.contains(&id)
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
    owners: Vec<LoanOwners>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LoanOwners {
    storage: LocalSet,
    contents: LocalSet,
    /// Provenance/lifetime carriers, not safe-reference access permissions.
    raw: LocalSet,
    /// A raw address of this storage, distinct from raw handles in its value.
    /// Reading the pointee transports contents, not the address's storage lease.
    raw_storage: LocalSet,
    /// A reference reconstructed from a raw carrier retains its lifetime lease;
    /// the separately collected borrow supplies its shared/exclusive access kind.
    borrowed_raw: LocalSet,
    /// Conservative erased-return lifetime envelopes. The checked result's
    /// separately collected view loan supplies access permissions.
    returned: LocalSet,
}

impl LoanOwners {
    fn all(&self) -> LocalSet {
        let mut owners = self.storage.clone();
        owners.join(&self.contents);
        owners.join(&self.raw);
        owners.join(&self.raw_storage);
        owners.join(&self.borrowed_raw);
        owners.join(&self.returned);
        owners
    }

    fn is_empty(&self) -> bool {
        self.storage.is_empty()
            && self.contents.is_empty()
            && self.raw.is_empty()
            && self.raw_storage.is_empty()
            && self.borrowed_raw.is_empty()
            && self.returned.is_empty()
    }

    fn contains(&self, local: Local) -> bool {
        self.storage.contains(local)
            || self.contents.contains(local)
            || self.raw.contains(local)
            || self.raw_storage.contains(local)
            || self.borrowed_raw.contains(local)
            || self.returned.contains(local)
    }

    fn remove(&mut self, local: Local) -> bool {
        self.storage.remove(local)
            | self.contents.remove(local)
            | self.raw.remove(local)
            | self.raw_storage.remove(local)
            | self.borrowed_raw.remove(local)
            | self.returned.remove(local)
    }

    fn join(&mut self, other: &Self) -> bool {
        self.storage.join(&other.storage)
            | self.contents.join(&other.contents)
            | self.raw.join(&other.raw)
            | self.raw_storage.join(&other.raw_storage)
            | self.borrowed_raw.join(&other.borrowed_raw)
            | self.returned.join(&other.returned)
    }
}

impl LoanState {
    pub fn new(loan_count: usize) -> Self {
        Self {
            active: BitSet::with_capacity(loan_count),
            owners: (0..loan_count).map(|_| LoanOwners::default()).collect(),
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
        let owner_changed = self.owners[id.index()].storage.insert(loan.initial_owner);
        active_changed || owner_changed
    }

    pub fn add_owner(&mut self, id: LoanId, owner: Local) -> bool {
        self.ensure_len(id.index() + 1);
        let active_changed = self.active.insert(id);
        let owner_changed = self.owners[id.index()].storage.insert(owner);
        active_changed || owner_changed
    }

    pub fn owners(&self, id: LoanId) -> LocalSet {
        self.owners[id.index()].all()
    }

    pub fn active_ids(&self) -> impl Iterator<Item = LoanId> + '_ {
        self.active.iter().filter(|id| self.is_active(*id))
    }

    pub(crate) fn roots_carried_by<'a>(
        &'a self,
        owner: Local,
        table: &'a LoanTable,
    ) -> impl Iterator<Item = Local> + 'a {
        self.active_ids()
            .filter(move |id| self.owner_contains(*id, owner))
            .filter_map(|id| table.get(id).map(|loan| loan.diagnostic_place.local))
    }

    pub fn owner_contains(&self, id: LoanId, local: Local) -> bool {
        self.owners
            .get(id.index())
            .is_some_and(|owners| owners.contains(local))
    }

    fn safe_owner_contains(&self, id: LoanId, local: Local) -> bool {
        self.owners
            .get(id.index())
            .is_some_and(|owners| owners.storage.contains(local) || owners.contents.contains(local))
    }

    pub(crate) fn storage_owner_contains(&self, id: LoanId, local: Local) -> bool {
        self.owners
            .get(id.index())
            .is_some_and(|owners| owners.storage.contains(local))
    }

    /// The alias resolver must not interpret payload/lease carriers as addresses.
    pub(crate) fn storage_alias_state(&self) -> Self {
        let mut aliases = self.clone();
        for owners in &mut aliases.owners {
            *owners = LoanOwners {
                storage: owners.storage.clone(),
                ..LoanOwners::default()
            };
        }
        aliases
    }

    /// Embed only selected source loans, retaining lease roles and excluding the
    /// call's storage reborrows by identity, not by loan kind or source name.
    pub(crate) fn copy_selected_as_contents(
        &mut self,
        from: Local,
        to: Local,
        selected: &HashSet<LoanId>,
        contents_only: bool,
    ) -> bool {
        let mut changed = false;
        for id in selected {
            let Some(owners) = self.owners.get_mut(id.index()) else {
                continue;
            };
            if owners.contents.contains(from) || (!contents_only && owners.storage.contains(from)) {
                changed |= owners.contents.insert(to);
            }
            if owners.raw.contains(from) || (!contents_only && owners.raw_storage.contains(from)) {
                changed |= owners.raw.insert(to);
            }
            if owners.borrowed_raw.contains(from) {
                changed |= owners.borrowed_raw.insert(to);
            }
            if owners.returned.contains(from) {
                changed |= owners.returned.insert(to);
            }
        }
        changed
    }

    pub fn transfer_owner(&mut self, from: Local, to: Local) -> bool {
        let mut changed = false;
        for owners in &mut self.owners {
            for group in [
                &mut owners.storage,
                &mut owners.contents,
                &mut owners.raw,
                &mut owners.raw_storage,
                &mut owners.borrowed_raw,
                &mut owners.returned,
            ] {
                if group.remove(from) {
                    changed = true;
                    changed |= group.insert(to);
                }
            }
        }
        changed
    }

    pub fn copy_owner(&mut self, from: Local, to: Local) -> bool {
        let mut changed = false;
        for owners in &mut self.owners {
            for group in [
                &mut owners.storage,
                &mut owners.contents,
                &mut owners.raw,
                &mut owners.raw_storage,
                &mut owners.borrowed_raw,
                &mut owners.returned,
            ] {
                if group.contains(from) {
                    changed |= group.insert(to);
                }
            }
        }
        changed
    }

    /// Embed an input value's loans in an aggregate or borrowed container view.
    pub fn copy_owner_as_contents(&mut self, from: Local, to: Local) -> bool {
        let mut changed = false;
        for owners in &mut self.owners {
            if owners.storage.contains(from) || owners.contents.contains(from) {
                changed |= owners.contents.insert(to);
            }
            if owners.raw.contains(from) || owners.raw_storage.contains(from) {
                changed |= owners.raw.insert(to);
            }
            if owners.borrowed_raw.contains(from) {
                changed |= owners.borrowed_raw.insert(to);
            }
            if owners.returned.contains(from) {
                changed |= owners.returned.insert(to);
            }
        }
        changed
    }

    /// A proven returned-content effect does not retain a receiver-storage loan.
    pub fn copy_contents_owner(&mut self, from: Local, to: Local) -> bool {
        let mut changed = false;
        for owners in &mut self.owners {
            if owners.contents.contains(from) {
                changed |= owners.contents.insert(to);
            }
            if owners.raw.contains(from) {
                changed |= owners.raw.insert(to);
            }
            if owners.borrowed_raw.contains(from) {
                changed |= owners.borrowed_raw.insert(to);
            }
            if owners.returned.contains(from) {
                changed |= owners.returned.insert(to);
            }
        }
        changed
    }

    /// Loading a raw field transports its contained loans, not container storage.
    pub fn copy_contents_owner_as_raw(&mut self, from: Local, to: Local) -> bool {
        let mut changed = false;
        for owners in &mut self.owners {
            if owners.contents.contains(from)
                || owners.raw.contains(from)
                || owners.borrowed_raw.contains(from)
                || owners.returned.contains(from)
            {
                changed |= owners.raw.insert(to);
            }
        }
        changed
    }

    /// Preserve actual provenance without granting a raw carrier safe loan rights.
    pub fn copy_owner_as_raw(&mut self, from: Local, to: Local) -> bool {
        let mut changed = false;
        for owners in &mut self.owners {
            if owners.storage.contains(from) || owners.raw_storage.contains(from) {
                changed |= owners.raw_storage.insert(to);
            }
            if owners.contents.contains(from) {
                // Addressing an aggregate does not erase its live safe fields.
                changed |= owners.contents.insert(to);
            }
            if owners.raw.contains(from)
                || owners.borrowed_raw.contains(from)
                || owners.returned.contains(from)
            {
                changed |= owners.raw.insert(to);
            }
        }
        changed
    }

    /// Retain an erased interface's origin envelope without inheriting the
    /// source loan's shared/exclusive permission as the returned view's kind.
    pub fn copy_owner_as_returned(&mut self, from: Local, to: Local, contents_only: bool) -> bool {
        let mut changed = false;
        for owners in &mut self.owners {
            let carries = if contents_only {
                owners.contents.contains(from)
                    || owners.raw.contains(from)
                    || owners.borrowed_raw.contains(from)
                    || owners.returned.contains(from)
            } else {
                owners.contains(from)
            };
            if carries {
                changed |= owners.returned.insert(to);
            }
        }
        changed
    }

    pub fn copy_owner_for_borrow(&mut self, from: Local, to: Local) -> bool {
        let mut changed = false;
        for owners in &mut self.owners {
            if owners.storage.contains(from) {
                changed |= owners.storage.insert(to);
            }
            if owners.contents.contains(from) {
                changed |= owners.contents.insert(to);
            }
            if owners.raw.contains(from)
                || owners.raw_storage.contains(from)
                || owners.borrowed_raw.contains(from)
            {
                changed |= owners.borrowed_raw.insert(to);
            }
            if owners.returned.contains(from) {
                changed |= owners.returned.insert(to);
            }
        }
        changed
    }

    /// Transport one certified slice-data effect. Only this result's supplying
    /// slot-storage role is excluded; every LoanId and other carrier stays live.
    pub(crate) fn copy_borrowed_slice_data(
        &mut self,
        from: Local,
        to: Local,
        supplying_slots: &HashSet<LoanId>,
    ) -> bool {
        let mut changed = false;
        for (index, owners) in self.owners.iter_mut().enumerate() {
            if !supplying_slots.contains(&LoanId(index as u32)) && owners.storage.contains(from) {
                changed |= owners.storage.insert(to);
            }
            if owners.contents.contains(from) {
                changed |= owners.contents.insert(to);
            }
            if owners.raw.contains(from)
                || owners.raw_storage.contains(from)
                || owners.borrowed_raw.contains(from)
            {
                changed |= owners.borrowed_raw.insert(to);
            }
            if owners.returned.contains(from) {
                changed |= owners.returned.insert(to);
            }
        }
        changed
    }

    fn access_through_raw_lease_owner(&self, id: LoanId, place: &Place) -> bool {
        self.owners.get(id.index()).is_some_and(|owners| {
            matches!(place.projection.first(), Some(Projection::Deref))
                && (owners.borrowed_raw.contains(place.local)
                    || ((owners.raw.contains(place.local)
                        || owners.raw_storage.contains(place.local))
                        && owners.borrowed_raw.is_empty()))
                && owners.storage.is_empty()
                && owners.contents.is_empty()
        })
    }

    fn access_through_returned_lease_owner(
        &self,
        id: LoanId,
        place: &Place,
        kind: LoanKind,
        table: &LoanTable,
    ) -> bool {
        self.owners.get(id.index()).is_some_and(|owners| {
            matches!(place.projection.first(), Some(Projection::Deref))
                && owners.returned.contains(place.local)
                // A live safe alias is not just a lifetime envelope.
                && owners.storage.is_empty() && owners.contents.is_empty()
                && table.iter().any(|view| {
                    table.is_returned_view(view.id) && self.safe_owner_contains(view.id, place.local)
                        && (kind == LoanKind::Shared || view.kind == LoanKind::Mut)
                })
        })
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
            let current: Vec<_> = owners.all().iter().collect();
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
            self.owners
                .resize_with(other.owners.len(), LoanOwners::default);
            changed = true;
        }
        for (index, owners) in other.owners.iter().enumerate() {
            changed |= self.owners[index].join(owners);
        }
        changed
    }

    fn ensure_len(&mut self, len: usize) {
        if self.owners.len() < len {
            self.owners.resize_with(len, LoanOwners::default);
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

impl LoanKind {
    /// Compatibility of overlapping accesses, independent of storage layout.
    pub(crate) fn conflicts_with(self, loan: Self) -> bool {
        self == Self::Mut || loan == Self::Mut
    }
}

impl From<crate::types::ReceiverMode> for LoanKind {
    fn from(mode: crate::types::ReceiverMode) -> Self {
        match mode {
            crate::types::ReceiverMode::Shared => Self::Shared,
            crate::types::ReceiverMode::Mut | crate::types::ReceiverMode::Move => Self::Mut,
        }
    }
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
        let places = crate::mir::borrowck::provenance::resolve_places(place, table, active_loans);

        for loan_id in active_loans.active_ids() {
            let Some(loan) = table.get(loan_id) else {
                continue;
            };
            if active_loans.access_through_raw_lease_owner(loan_id, &original_place)
                || active_loans.access_through_returned_lease_owner(
                    loan_id,
                    &original_place,
                    kind,
                    table,
                )
                || Self::access_through_mutable_loan_owner(
                    &original_place,
                    loan_id,
                    loan,
                    active_loans,
                )
            {
                continue;
            }

            // Check if loans conflict
            let places_conflict = if table.place_paths().place(loan.place_path).is_some() {
                let loan_places = crate::mir::borrowck::provenance::resolve_loan_places(
                    loan_id,
                    table,
                    active_loans,
                );
                places.iter().any(|place| {
                    loan_places.iter().any(|loan_place| {
                        match (
                            table.place_paths().path_id(place),
                            table.place_paths().path_id(loan_place),
                        ) {
                            (Some(access_path), Some(loan_path)) => {
                                table.place_paths().paths_conflict(access_path, loan_path)
                            }
                            _ => {
                                crate::mir::borrowck::conflicts::places_conflict(place, loan_place)
                            }
                        }
                    })
                })
            } else {
                false
            };

            if places_conflict && kind.conflicts_with(loan.kind) {
                return Err(loan_id);
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
            && active_loans.safe_owner_contains(loan_id, place.local)
            && matches!(place.projection.first(), Some(Projection::Deref))
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
    fn reserved_root_envelopes_share_native_ownership_and_access_rules() {
        let table = LoanTable::root_envelopes(4).unwrap();
        let root = Place {
            local: Local(0),
            projection: vec![],
        };
        let exclusive = table.root_envelope_id(root.local, LoanKind::Mut).unwrap();
        let shared = table
            .root_envelope_id(root.local, LoanKind::Shared)
            .unwrap();
        let mut state = LoanState::new(table.len());
        assert_eq!(
            state.active_ids().count(),
            0,
            "identities are not live loans"
        );
        assert!(LoanAnalysis::check_aliasing(&root, LoanKind::Mut, &table, &state).is_ok());
        state.add_owner(exclusive, Local(1));
        assert_eq!(
            LoanAnalysis::check_aliasing(&root, LoanKind::Shared, &table, &state),
            Err(exclusive)
        );
        assert_eq!(
            LoanAnalysis::check_aliasing(&root, LoanKind::Mut, &table, &state),
            Err(exclusive)
        );
        state.copy_owner(Local(1), Local(2));
        state.release_owner(Local(1));
        assert!(
            state.is_active(exclusive),
            "a copied carrier keeps the loan alive"
        );
        state.transfer_owner(Local(2), Local(3));
        assert!(!state.owner_contains(exclusive, Local(2)));
        assert_eq!(
            state.roots_carried_by(Local(3), &table).collect::<Vec<_>>(),
            vec![root.local]
        );
        state.release_owner(Local(3));
        assert!(!state.is_active(exclusive));
        state.add_owner(shared, Local(1));
        assert!(LoanAnalysis::check_aliasing(&root, LoanKind::Shared, &table, &state).is_ok());
        assert_eq!(
            LoanAnalysis::check_aliasing(&root, LoanKind::Mut, &table, &state),
            Err(shared)
        );
        assert!(table.root_envelope_id(Local(4), LoanKind::Shared).is_none());
    }

    #[test]
    fn root_carrier_joins_and_liveness_keep_each_path_obligation() {
        let table = LoanTable::root_envelopes(4).unwrap();
        let first = table.root_envelope_id(Local(0), LoanKind::Shared).unwrap();
        let second = table.root_envelope_id(Local(1), LoanKind::Mut).unwrap();
        let mut left = LoanState::new(table.len());
        left.add_owner(first, Local(2));
        let mut right = LoanState::new(table.len());
        right.add_owner(second, Local(3));
        let mut joined = left.clone();
        assert!(joined.join(&right));
        assert!(!joined.join(&right));
        right.join(&left);
        assert_eq!(joined, right);
        joined.retain_owners(|local| local == Local(2));
        assert!(joined.is_active(first));
        assert!(!joined.is_active(second));
        assert!(
            !joined.owner_contains(second, Local(2)),
            "a CFG join cannot change owner identity"
        );
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
    fn returned_contents_keep_payload_loan_without_receiver_storage_loan() {
        let mut table = LoanTable::new();
        let payload = table.push(loan_data(LoanId(0), Local(1)));
        let receiver_storage = table.push(loan_data(LoanId(1), Local(3)));
        let mut state = LoanState::new(table.len());
        state.activate(payload, &table);
        state.copy_owner_as_contents(Local(1), Local(2));
        state.release_owner(Local(1));
        state.copy_owner_as_contents(Local(2), Local(3));
        state.activate(receiver_storage, &table);
        let mut opaque = state.clone();
        opaque.copy_owner(Local(3), Local(4));
        state.copy_contents_owner(Local(3), Local(4));
        for owner in [Local(2), Local(3)] {
            state.release_owner(owner);
            opaque.release_owner(owner);
        }
        assert!(state.is_active(payload));
        assert!(state.owner_contains(payload, Local(4)));
        assert!(!state.is_active(receiver_storage));
        assert!(
            opaque.is_active(receiver_storage),
            "missing evidence must retain storage conservatively",
        );
        state.release_owner(Local(4));
        assert!(!state.is_active(payload));
    }

    #[test]
    fn selected_mutation_contents_preserve_roles_and_exclude_receiver_loan() {
        let mut table = LoanTable::new();
        let payload = table.push(loan_data(LoanId(0), Local(1)));
        let receiver = table.push(loan_data(LoanId(1), Local(7)));
        let mut state = LoanState::new(table.len());
        state.activate(payload, &table);
        state.copy_owner(Local(1), Local(7));
        state.copy_owner_as_contents(Local(1), Local(7));
        state.copy_owner_as_raw(Local(1), Local(7));
        state.copy_contents_owner_as_raw(Local(7), Local(7));
        state.copy_owner_for_borrow(Local(7), Local(7));
        state.copy_owner_as_returned(Local(7), Local(7), false);
        state.activate(receiver, &table);
        let selected = HashSet::from([payload]);
        assert!(state.copy_selected_as_contents(Local(7), Local(8), &selected, false));
        let roles = &state.owners[payload.index()];
        assert!(roles.contents.contains(Local(8)));
        assert!(roles.raw.contains(Local(8)));
        assert!(roles.borrowed_raw.contains(Local(8)));
        assert!(roles.returned.contains(Local(8)));
        assert!(!roles.storage.contains(Local(8)));
        assert!(!roles.raw_storage.contains(Local(8)));
        assert!(
            roles.storage.contains(Local(7)),
            "copy must not erase the source"
        );
        assert!(
            !state.owner_contains(receiver, Local(8)),
            "exact excluded storage loan"
        );
        state.copy_owner_as_raw(Local(7), Local(10));
        assert!(
            !state.copy_selected_as_contents(
                Local(10),
                Local(11),
                &HashSet::from([receiver]),
                true
            ),
            "Contents must not retain raw storage-address leases"
        );
        assert!(state.copy_selected_as_contents(
            Local(10),
            Local(12),
            &HashSet::from([receiver]),
            false
        ));
        assert!(state.owners[receiver.index()].raw.contains(Local(12)));
        let aliases = state.storage_alias_state();
        assert!(
            !aliases.owner_contains(payload, Local(8)),
            "contents are not address aliases"
        );
        assert!(aliases.storage_owner_contains(receiver, Local(7)));
        state.transfer_owner(Local(8), Local(9));
        assert!(state.owner_contains(payload, Local(9)));
        for local in 1..=12 {
            state.release_owner(Local(local));
        }
        assert!(!state.is_active(payload));
        assert!(!state.is_active(receiver));
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
            returned_views: HashSet::new(),
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
            returned_views: HashSet::new(),
        };
        let mut active = LoanState::new(table.len());
        active.activate(LoanId(0), &table);

        assert!(LoanAnalysis::check_aliasing(&place, LoanKind::Mut, &table, &active).is_ok());
    }
}

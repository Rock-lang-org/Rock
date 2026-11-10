use std::collections::{HashMap, HashSet};

use crate::mir::{Local, Place, Projection};
use crate::types::Type;

/// A hidden array temporary that a by-value `for` loop consumes element by
/// element. Elements `counter + 1..len` are the unvisited tail that scope-exit
/// cleanup must still drop; the element at `counter` already moved into the
/// loop binding.
#[derive(Clone)]
pub(super) struct LoopOwnedArray {
    pub(super) counter: Local,
    /// The loop binding that owns the element moved out on the current pass.
    pub(super) item_local: Local,
    pub(super) elem_ty: Type,
    pub(super) len: usize,
}

/// Per-function move and drop-flag bookkeeping used to decide which scope-exit
/// drops are unconditional, flag-guarded, or skipped.
///
/// The builder owns MIR local creation, so flag locals are allocated by the
/// caller and registered here once they exist.
#[derive(Default)]
pub(super) struct DropState {
    moved_places: HashSet<Place>,
    statically_moved_cleanup_places: HashSet<Place>,
    drop_flags: HashMap<Local, Local>,
    projection_drop_flags: HashMap<Place, Local>,
    loop_owned_arrays: HashMap<Local, LoopOwnedArray>,
    consumed_loop_arrays: HashSet<Local>,
    cleanup_mode: bool,
}

impl DropState {
    /// Forgets all per-function move and flag facts. The cleanup-mode flag is
    /// scoped by `enter_cleanup`/`leave_cleanup` and is left alone.
    pub(super) fn reset_for_function(&mut self) {
        self.drop_flags.clear();
        self.projection_drop_flags.clear();
        self.moved_places.clear();
        self.statically_moved_cleanup_places.clear();
        self.loop_owned_arrays.clear();
        self.consumed_loop_arrays.clear();
    }

    pub(super) fn register_loop_owned_array(&mut self, temp: Local, owned: LoopOwnedArray) {
        self.loop_owned_arrays.insert(temp, owned);
    }

    pub(super) fn loop_owned_array(&self, temp: Local) -> Option<LoopOwnedArray> {
        self.loop_owned_arrays.get(&temp).cloned()
    }

    /// The loop finished: every element was moved out or dropped, so later
    /// scope exits only end the temporary's storage.
    pub(super) fn consume_loop_owned_array(&mut self, temp: Local) {
        if self.loop_owned_arrays.remove(&temp).is_some() {
            self.consumed_loop_arrays.insert(temp);
        }
    }

    pub(super) fn is_consumed_loop_array(&self, temp: Local) -> bool {
        self.consumed_loop_arrays.contains(&temp)
    }

    /// Switches to cleanup mode and returns the previous mode to hand back to
    /// `leave_cleanup`.
    pub(super) fn enter_cleanup(&mut self) -> bool {
        std::mem::replace(&mut self.cleanup_mode, true)
    }

    pub(super) fn leave_cleanup(&mut self, previous: bool) {
        self.cleanup_mode = previous;
    }

    pub(super) fn in_cleanup(&self) -> bool {
        self.cleanup_mode
    }

    pub(super) fn register_drop_flag(&mut self, local: Local, flag: Local) {
        self.drop_flags.insert(local, flag);
    }

    pub(super) fn register_projection_drop_flag(&mut self, place: Place, flag: Local) {
        self.projection_drop_flags.insert(place, flag);
    }

    pub(super) fn has_projection_drop_flag(&self, place: &Place) -> bool {
        self.projection_drop_flags.contains_key(place)
    }

    pub(super) fn drop_flag_for(&self, local: Local) -> Option<Local> {
        self.drop_flags.get(&local).copied()
    }

    pub(super) fn drop_flag_for_place(&self, place: &Place) -> Option<Local> {
        if place.projection.is_empty() {
            self.drop_flag_for(place.local)
        } else {
            self.projection_drop_flags
                .iter()
                .find_map(|(candidate, flag)| {
                    (candidate.local == place.local
                        && candidate.projection.len() == place.projection.len()
                        && projection_is_prefix(&candidate.projection, &place.projection))
                    .then_some(*flag)
                })
        }
    }

    /// Projection flags of `local`, ordered by flag local for stable output.
    pub(super) fn projection_drop_flags_for_local(&self, local: Local) -> Vec<Local> {
        let mut flags = self
            .projection_drop_flags
            .iter()
            .filter_map(|(place, flag)| (place.local == local).then_some(*flag))
            .collect::<Vec<_>>();
        flags.sort_by_key(|flag| flag.0);
        flags
    }

    /// Projection flags of every tracked place at or below `place`, ordered by
    /// flag local.
    pub(super) fn projection_drop_flags_under(&self, place: &Place) -> Vec<Local> {
        let mut flags = self
            .projection_drop_flags
            .iter()
            .filter_map(|(candidate, flag)| place_is_prefix(place, candidate).then_some(*flag))
            .collect::<Vec<_>>();
        flags.sort_by_key(|flag| flag.0);
        flags
    }

    /// A moved whole local forgets earlier moves of its parts.
    pub(super) fn record_local_moved(&mut self, local: Local) {
        self.moved_places.retain(|moved| moved.local != local);
    }

    pub(super) fn record_projection_moved(&mut self, place: Place) {
        self.moved_places.insert(place);
    }

    /// Re-initialisation of `place` clears move facts at or below it.
    pub(super) fn reinitialize(&mut self, place: &Place) {
        if place.projection.is_empty() {
            self.moved_places.retain(|moved| moved.local != place.local);
            self.statically_moved_cleanup_places
                .retain(|moved| moved.local != place.local);
        } else {
            self.moved_places
                .retain(|moved| !place_is_prefix(place, moved));
            self.statically_moved_cleanup_places
                .retain(|moved| !place_is_prefix(place, moved));
        }
    }

    /// Records that `place` is moved for cleanup purposes and is known at
    /// build time to be moved, superseding finer-grained earlier moves.
    pub(super) fn record_statically_moved_for_cleanup(&mut self, place: Place) {
        if place.projection.is_empty() {
            self.moved_places.retain(|moved| moved.local != place.local);
        } else {
            self.moved_places
                .retain(|moved| !place_is_prefix(&place, moved));
        }
        self.moved_places.insert(place.clone());
        self.statically_moved_cleanup_places.insert(place);
    }

    pub(super) fn may_be_moved(&self, place: &Place) -> bool {
        self.moved_places
            .iter()
            .any(|moved| place_is_prefix(moved, place))
    }

    pub(super) fn statically_moved_for_cleanup(&self, place: &Place) -> bool {
        self.statically_moved_cleanup_places
            .iter()
            .any(|moved| place_is_prefix(moved, place))
    }

    pub(super) fn has_moved_descendant(&self, place: &Place) -> bool {
        self.moved_places
            .iter()
            .any(|moved| place_is_strict_prefix(place, moved))
    }
}

fn place_is_prefix(prefix: &Place, place: &Place) -> bool {
    prefix.local == place.local && projection_is_prefix(&prefix.projection, &place.projection)
}

fn place_is_strict_prefix(prefix: &Place, place: &Place) -> bool {
    prefix.local == place.local
        && prefix.projection.len() < place.projection.len()
        && projection_is_prefix(&prefix.projection, &place.projection)
}

fn projection_is_prefix(prefix: &[Projection], projection: &[Projection]) -> bool {
    prefix.len() <= projection.len()
        && prefix
            .iter()
            .zip(projection.iter())
            .all(|(lhs, rhs)| projection_component_matches(lhs, rhs))
}

fn projection_component_matches(lhs: &Projection, rhs: &Projection) -> bool {
    match (lhs, rhs) {
        (Projection::Deref, Projection::Deref) => true,
        (Projection::Downcast(lhs), Projection::Downcast(rhs)) => lhs == rhs,
        (Projection::Index(lhs), Projection::Index(rhs)) => lhs == rhs,
        (Projection::Field { index: lhs, .. }, Projection::Field { index: rhs, .. }) => {
            lhs == rhs
        }
        _ => false,
    }
}

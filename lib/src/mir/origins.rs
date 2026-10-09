//! Layout-independent origin roles and their may-flow join.

use std::collections::{HashMap, HashSet};

use super::dataflow::Lattice;
use super::{Local, Place, ReferenceOrigin};

pub(crate) type OriginSet = HashSet<ReferenceOrigin>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ReferenceOriginState {
    pub(crate) origins: HashMap<Place, OriginSet>,
}

impl Lattice for ReferenceOriginState {
    fn join(&mut self, other: &Self) -> bool {
        let mut changed = false;
        for (place, origins) in &other.origins {
            changed |= self.origins.entry(place.clone()).or_default().join(origins);
        }
        changed
    }
}

impl ReferenceOrigin {
    /// Loading a value transports contained loans, not the referent's storage.
    pub(crate) fn loaded_contents(self) -> Self {
        match self {
            Self::Param(local) => Self::ParamContents(local),
            origin => origin,
        }
    }

    /// Addressing contents retains storage; raw reconstruction cannot certify
    /// that the resulting reference is an independent copy of contained loans.
    pub(crate) fn borrowed_contents(self, raw_memory: bool, raw_indexed: bool) -> Self {
        match self {
            Self::ParamContents(local) if raw_memory || raw_indexed => {
                Self::ParamBorrowedContents(local)
            }
            Self::Param(local) if raw_memory => Self::ParamBorrowedContents(local),
            Self::ParamContents(local) => Self::Param(local),
            origin => origin,
        }
    }

    pub(crate) fn local_storage(self) -> Option<Local> {
        match self {
            Self::Local(local) | Self::Temporary(local) => Some(local),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_and_addressing_preserve_distinct_origin_roles() {
        let local = Local(3);
        assert_eq!(
            ReferenceOrigin::Param(local).loaded_contents(),
            ReferenceOrigin::ParamContents(local),
        );
        assert_eq!(
            ReferenceOrigin::ParamContents(local).borrowed_contents(false, false),
            ReferenceOrigin::Param(local),
        );
        for origin in [
            ReferenceOrigin::Param(local),
            ReferenceOrigin::ParamContents(local),
        ] {
            assert_eq!(
                origin.borrowed_contents(true, false),
                ReferenceOrigin::ParamBorrowedContents(local),
            );
        }
        assert_eq!(
            ReferenceOrigin::Param(local).borrowed_contents(false, true),
            ReferenceOrigin::Param(local),
            "indexing alone cannot invent a raw reconstruction of the incoming origin",
        );
        assert_eq!(
            ReferenceOrigin::ParamContents(local).borrowed_contents(false, true),
            ReferenceOrigin::ParamBorrowedContents(local),
        );
        for origin in [
            ReferenceOrigin::ParamBorrowedContents(local),
            ReferenceOrigin::Local(local),
            ReferenceOrigin::Temporary(local),
            ReferenceOrigin::Static,
            ReferenceOrigin::UnknownExternal,
        ] {
            assert_eq!(origin.loaded_contents(), origin);
            assert_eq!(origin.borrowed_contents(false, false), origin);
            assert_eq!(origin.borrowed_contents(true, false), origin);
            assert_eq!(origin.borrowed_contents(false, true), origin);
        }
    }

    #[test]
    fn origin_joins_retain_every_branch_without_manufacturing_roles() {
        let place = Place {
            local: Local(1),
            projection: vec![],
        };
        let states = [
            ReferenceOrigin::Param(Local(2)),
            ReferenceOrigin::ParamContents(Local(2)),
            ReferenceOrigin::Local(Local(3)),
        ]
        .map(|origin| ReferenceOriginState {
            origins: HashMap::from([(place.clone(), HashSet::from([origin]))]),
        });
        let mut forward = states[0].clone();
        assert!(!forward.join(&states[0]));
        assert!(forward.join(&states[1]));
        assert!(forward.join(&states[2]));
        let mut reverse = states[2].clone();
        reverse.join(&states[1]);
        reverse.join(&states[0]);
        assert_eq!(forward, reverse);
        let mut tail = states[1].clone();
        tail.join(&states[2]);
        let mut associated = states[0].clone();
        associated.join(&tail);
        assert_eq!(forward, associated);
        assert_eq!(forward.origins[&place].len(), 3);
        assert!(!forward.origins[&place].contains(&ReferenceOrigin::Static));
        assert!(
            !forward.origins[&place].contains(&ReferenceOrigin::ParamBorrowedContents(Local(2)))
        );
    }
}

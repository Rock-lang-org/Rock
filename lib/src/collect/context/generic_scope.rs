//! The trait and generic-parameter scope in force while a header is lowered.
//! Entering a scope returns a frame holding what it replaced, and leaving
//! restores that frame, so nested declarations cannot leak parameters into
//! their enclosing one.

use crate::ids::DefId;
use crate::type_services::kind::Kind;

#[derive(Default)]
pub(crate) struct GenericScope {
    trait_name: Option<String>,
    owner: Option<DefId>,
    params: Vec<String>,
    kinds: Vec<Kind>,
    /// Set while lowering a nominal's fields: only the declared parameter list is in scope.
    closed: bool,
}

/// The generic owner and parameters replaced by `GenericScope::enter`.
pub(crate) struct GenericFrame {
    owner: Option<DefId>,
    params: Vec<String>,
    kinds: Vec<Kind>,
}

/// The parameters of the current owner at the time of the snapshot, for a
/// declaration that extends its enclosing owner instead of replacing it.
pub(crate) struct ParamsFrame {
    params: Vec<String>,
    kinds: Vec<Kind>,
}

/// The trait context replaced by `GenericScope::enter_trait`.
pub(crate) struct TraitFrame {
    name: Option<String>,
}

impl GenericScope {
    /// Makes `owner` the generic owner with exactly `params`.
    pub(crate) fn enter(
        &mut self,
        owner: DefId,
        params: Vec<String>,
        kinds: Vec<Kind>,
    ) -> GenericFrame {
        GenericFrame {
            owner: self.owner.replace(owner),
            params: std::mem::replace(&mut self.params, params),
            kinds: std::mem::replace(&mut self.kinds, kinds),
        }
    }

    pub(crate) fn leave(&mut self, frame: GenericFrame) {
        self.owner = frame.owner;
        self.params = frame.params;
        self.kinds = frame.kinds;
    }

    pub(crate) fn enter_trait(&mut self, name: String) -> TraitFrame {
        TraitFrame {
            name: self.trait_name.replace(name),
        }
    }

    pub(crate) fn leave_trait(&mut self, frame: TraitFrame) {
        self.trait_name = frame.name;
    }

    pub(crate) fn trait_name(&self) -> Option<&str> {
        self.trait_name.as_deref()
    }

    pub(crate) fn owner(&self) -> Option<DefId> {
        self.owner
    }

    pub(crate) fn params(&self) -> &[String] {
        &self.params
    }

    pub(crate) fn kinds(&self) -> &[Kind] {
        &self.kinds
    }

    pub(crate) fn position(&self, name: &str) -> Option<usize> {
        self.params.iter().position(|param| param == name)
    }

    pub(crate) fn kind_at(&self, index: usize) -> Option<&Kind> {
        self.kinds.get(index)
    }

    /// Closes or reopens the scope to implicit declarations, returning the previous state.
    pub(crate) fn set_closed(&mut self, closed: bool) -> bool {
        std::mem::replace(&mut self.closed, closed)
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed
    }

    /// Appends a parameter to the current owner and returns its index.
    pub(crate) fn declare(&mut self, name: String, kind: Kind) -> usize {
        self.params.push(name);
        self.kinds.push(kind);
        self.params.len() - 1
    }

    pub(crate) fn snapshot_params(&self) -> ParamsFrame {
        ParamsFrame {
            params: self.params.clone(),
            kinds: self.kinds.clone(),
        }
    }

    /// Drops the parameters declared since `snapshot_params`.
    pub(crate) fn restore_params(&mut self, frame: ParamsFrame) {
        self.params = frame.params;
        self.kinds = frame.kinds;
    }
}

#[cfg(test)]
mod tests;

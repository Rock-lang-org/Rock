//! Phase 4 constraint solver: discharge accumulated constraints after lowering.
//!
//! This runs between generalization and finalization. It:
//!   - Verifies trait bounds: if a type variable resolved to a concrete type, checks
//!     that the type has an impl for the required trait.
//!   - Checks integer/float literal constraints: if a TypeVar from a literal still
//!     has no concrete type, emits an ambiguity warning.
//!
//! Trait bound violations on concrete types are hard errors.

mod auto_traits;
mod constructor_obligations;
mod impl_lookup;
mod structural;
mod trait_bounds;
mod type_predicates;
mod worklist;

use std::collections::{HashMap, HashSet};

use crate::hir::{HirEnum, HirImpl, HirStruct};
use crate::ids::DefId;
use crate::ids::TypeVarId;
use crate::infer::constraints::{ConstraintOwner, ConstraintStore};
use crate::infer::{InferenceEngine, UnifyError};
use crate::language_items::LanguageItems;
use crate::types::{TraitBound, Type};
use worklist::solve_constraints_in_place_mode;

#[derive(Debug, Clone)]
pub enum SolveErrorKind {
    Unify {
        context: String,
        error: UnifyError,
    },
    TraitNotImplemented {
        ty: Type,
        trait_name: String,
        context: String,
    },
    TryNonCarrier {
        ty: Type,
    },
    IntegerLiteral {
        found: Type,
    },
    Message(String),
}

#[derive(Debug, Clone)]
pub struct SolveError {
    pub kind: SolveErrorKind,
    pub span: crate::lexer::Span,
}

impl SolveError {
    pub fn contains(&self, pattern: &str) -> bool {
        self.message().contains(pattern)
    }

    pub fn message(&self) -> String {
        self.render_with(|ty| ty.to_string())
    }

    pub fn render(&self, engine: &InferenceEngine) -> String {
        self.render_with(|ty| engine.display_type(ty))
    }

    fn render_with(&self, display: impl Fn(&Type) -> String) -> String {
        match &self.kind {
            SolveErrorKind::Unify { context, error } => {
                format!("{context}: {}", error.render_with(&display))
            }
            SolveErrorKind::TraitNotImplemented {
                ty,
                trait_name,
                context,
            } => format!(
                "type `{}` does not implement trait `{}` (required by {})",
                display(ty),
                trait_name,
                context
            ),
            SolveErrorKind::TryNonCarrier { ty } => {
                format!("Cannot use '?' on non-carrier type {}", display(ty))
            }
            SolveErrorKind::IntegerLiteral { found } => format!(
                "integer literal resolved to non-integer type `{}`",
                display(found)
            ),
            SolveErrorKind::Message(message) => message.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct BuiltinTraitIds {
    sized: Option<DefId>,
    fn_once: Option<DefId>,
    fn_mut: Option<DefId>,
    fn_trait: Option<DefId>,
    send: Option<DefId>,
    sync: Option<DefId>,
}

impl BuiltinTraitIds {
    fn from_language_items(items: &LanguageItems<DefId>) -> Self {
        Self {
            sized: items.sized.as_ref().map(|items| items.trait_id),
            fn_once: items.fn_once.as_ref().map(|items| items.trait_id),
            fn_mut: items.fn_mut.as_ref().map(|items| items.trait_id),
            fn_trait: items.fn_trait.as_ref().map(|items| items.trait_id),
            send: items.send.as_ref().map(|items| items.trait_id),
            sync: items.sync.as_ref().map(|items| items.trait_id),
        }
    }
}

/// Result of constraint solving.
pub struct SolveResult {
    /// Warnings (non-fatal: ambiguous literals that were defaulted).
    pub warnings: Vec<String>,
    /// Hard errors: concrete type violates a required trait bound.
    pub errors: Vec<SolveError>,
    /// TypeVar ids that remain free but have at least one trait bound attached.
    pub generic_bounds: HashMap<TypeVarId, Vec<TraitBound>>,
    /// Representatives changed while this worklist invocation was running.
    pub changed_type_vars: Vec<TypeVarId>,
}

/// Solve all constraints in `store`.
///
/// `impls` is the list of all known trait implementations.
/// Trait violations on concrete types are always hard errors.
pub fn solve_constraints(
    engine: &mut InferenceEngine,
    store: &ConstraintStore,
    impls: &HashMap<DefId, HirImpl>,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    traits: &HashMap<DefId, crate::hir::HirTrait>,
    language_items: &LanguageItems<DefId>,
) -> SolveResult {
    let mut store = store.clone();
    solve_constraints_in_place(
        engine,
        &mut store,
        impls,
        structs,
        enums,
        traits,
        language_items,
    )
}

pub fn solve_constraints_in_place(
    engine: &mut InferenceEngine,
    store: &mut ConstraintStore,
    impls: &HashMap<DefId, HirImpl>,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    traits: &HashMap<DefId, crate::hir::HirTrait>,
    language_items: &LanguageItems<DefId>,
) -> SolveResult {
    solve_constraints_in_place_mode(
        engine,
        store,
        impls,
        structs,
        enums,
        traits,
        language_items,
        true,
        None,
        &crate::hir::HirGenericBounds::new(),
    )
}

pub(crate) fn solve_constraints_in_place_for_owners(
    engine: &mut InferenceEngine,
    store: &mut ConstraintStore,
    impls: &HashMap<DefId, HirImpl>,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    traits: &HashMap<DefId, crate::hir::HirTrait>,
    language_items: &LanguageItems<DefId>,
    owners: &HashSet<ConstraintOwner>,
    finalize_pending: bool,
    assumptions: &crate::hir::HirGenericBounds,
) -> SolveResult {
    solve_constraints_in_place_mode(
        engine,
        store,
        impls,
        structs,
        enums,
        traits,
        language_items,
        finalize_pending,
        Some(owners),
        assumptions,
    )
}

#[cfg(test)]
use crate::hir::HirImplReceiverPattern;
#[cfg(test)]
use crate::infer::constraints::ObligationState;
#[cfg(test)]
use crate::types::GenericParamId;
#[cfg(test)]
use impl_lookup::impl_exists_for;
#[cfg(test)]
mod tests;

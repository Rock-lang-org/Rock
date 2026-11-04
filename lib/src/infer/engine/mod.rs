//! Type inference engine implementation

mod binding;
mod errors;
mod folders;
mod fresh_vars;
mod invariant;
mod lifecycle;
mod strict;
mod type_var_state;
mod unify;

pub use errors::{InferenceError, UnifyError};

use std::collections::HashMap;

use crate::ids::{IdGen, TypeVarId};
use crate::lexer::Span;
use crate::type_services::kind::Kind;
use crate::type_services::normalize::TypeNormalizationEnv;
use crate::types::{TraitBound, Type};

/// Type inference engine using unification
pub struct InferenceEngine {
    next_var: IdGen<TypeVarId>,
    substitutions: HashMap<TypeVarId, Type>,
    substitution_generation: u64,
    rigid_substitution_generation: u64,
    changed_type_vars: Vec<TypeVarId>,
    /// Trait bounds for type variables (var_id -> list of bounds)
    trait_bounds: HashMap<TypeVarId, Vec<TraitBound>>,
    /// Origin span for each fresh type variable (for error messages)
    pub var_spans: HashMap<TypeVarId, Span>,
    var_kinds: HashMap<TypeVarId, Kind>,
    normalization_env: TypeNormalizationEnv,
    type_display_context: crate::type_services::display::TypeDisplayContext,
    generic_defaults: Vec<(Type, Type)>,
}

#[cfg(test)]
mod tests;

impl Default for InferenceEngine {
    fn default() -> Self {
        Self::new()
    }
}

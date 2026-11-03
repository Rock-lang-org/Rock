use std::collections::HashSet;

use crate::ids::TypeVarId;
use crate::infer::constraints::{Constraint, ConstraintStore};
use crate::lexer::Span;
use crate::type_services::kind::Kind;
use crate::type_services::normalize::{TypeNormalizationEnv, TypeNormalizer};
use crate::types::{TraitBound, Type};

use super::InferenceEngine;

impl InferenceEngine {
    /// Add a trait bound to a type variable
    pub fn add_bound(&mut self, var_id: TypeVarId, bound: TraitBound) {
        self.trait_bounds.entry(var_id).or_default().push(bound);
    }

    /// Get all trait bounds for a type variable
    pub fn get_bounds(&self, var_id: TypeVarId) -> Vec<TraitBound> {
        self.trait_bounds.get(&var_id).cloned().unwrap_or_default()
    }

    pub(crate) fn substitution_generation(&self) -> u64 {
        self.substitution_generation
    }

    pub(crate) fn rigid_substitution_generation(&self) -> u64 {
        self.rigid_substitution_generation
    }

    pub(crate) fn take_changed_type_vars(&mut self) -> Vec<TypeVarId> {
        std::mem::take(&mut self.changed_type_vars)
    }

    pub(crate) fn unresolved_type_var_representative_for_dependency(
        &self,
        var: TypeVarId,
    ) -> Option<TypeVarId> {
        self.unresolved_type_var_representative(var)
    }

    /// Apply defaults only to unresolved variables with numeric literal evidence.
    pub fn apply_numeric_defaults(&mut self, constraints: &ConstraintStore) {
        self.apply_numeric_defaults_for_owners(constraints, None);
    }

    pub(crate) fn apply_numeric_defaults_for_owners(
        &mut self,
        constraints: &ConstraintStore,
        owners: Option<&HashSet<crate::infer::ConstraintOwner>>,
    ) {
        let representatives = constraints
            .iter()
            .filter(|(id, _)| constraints.includes_owner(*id, owners))
            .filter_map(|(_, constraint)| match constraint {
                Constraint::IntLiteral { var, .. } | Constraint::FloatLiteral { var, .. } => {
                    self.unresolved_type_var_representative(*var)
                }
                Constraint::Trait { .. }
                | Constraint::Equality { .. }
                | Constraint::Coercion { .. }
                | Constraint::Try { .. } => None,
            })
            .collect::<HashSet<_>>();
        for representative in representatives {
            if self.kind_of_type_var(representative) != Kind::Type {
                continue;
            }
            let Some(default) = constraints
                .literal_default_type_for_representative(representative, |var| {
                    self.unresolved_type_var_representative(var)
                })
            else {
                continue;
            };
            if self.unresolved_type_var_representative(representative) == Some(representative) {
                let _ = self.bind_type_var(representative, default);
            }
        }
    }

    fn unresolved_type_var_representative(&self, var: TypeVarId) -> Option<TypeVarId> {
        match self.resolve(&Type::TypeVar(var)) {
            Type::TypeVar(representative) => Some(representative),
            _ => None,
        }
    }

    pub(crate) fn set_normalization_env(&mut self, mut env: TypeNormalizationEnv) {
        for (id, kind) in &self.var_kinds {
            env.register_inference_kind(*id, kind.clone());
        }
        self.normalization_env = env;
    }

    pub fn kind_of_type_var(&self, id: TypeVarId) -> Kind {
        self.var_kinds.get(&id).cloned().unwrap_or(Kind::Type)
    }

    pub fn span_for_type(&self, ty: &Type) -> Option<Span> {
        let mut span = None;
        crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
            if span.is_none() {
                if let Type::TypeVar(id) = nested {
                    span = self.var_spans.get(id).cloned();
                }
            }
        });
        span
    }

    pub fn kind_of(&self, ty: &Type) -> Result<Kind, String> {
        TypeNormalizer::new(&self.normalization_env)
            .kind_of(&self.resolve(ty))
            .map_err(|error| error.to_string())
    }

    pub(crate) fn normalization_env(&self) -> TypeNormalizationEnv {
        self.normalization_env.clone()
    }
}

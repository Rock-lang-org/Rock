use std::collections::HashMap;

use crate::ids::{IdGen, TypeVarId};
use crate::type_services::kind::Kind;
use crate::type_services::normalize::TypeNormalizationEnv;
use crate::types::Type;

use super::InferenceEngine;

impl InferenceEngine {
    pub fn new() -> Self {
        Self {
            next_var: IdGen::new(),
            substitutions: Default::default(),
            substitution_generation: 0,
            rigid_substitution_generation: 0,
            changed_type_vars: Vec::new(),
            trait_bounds: Default::default(),
            var_spans: Default::default(),
            var_kinds: Default::default(),
            normalization_env: Default::default(),
            type_display_context: Default::default(),
            generic_defaults: Vec::new(),
        }
    }

    pub fn with_next_type_var(next: u32) -> Self {
        let mut var_kinds = HashMap::new();
        let mut normalization_env = TypeNormalizationEnv::new();
        for raw in 0..next {
            let id = TypeVarId(raw);
            var_kinds.insert(id, Kind::Type);
            normalization_env.register_inference_kind(id, Kind::Type);
        }
        Self {
            next_var: IdGen::with_next_raw(next),
            substitutions: Default::default(),
            substitution_generation: 0,
            rigid_substitution_generation: 0,
            changed_type_vars: Vec::new(),
            trait_bounds: Default::default(),
            var_spans: Default::default(),
            var_kinds: var_kinds.into_iter().collect(),
            normalization_env: normalization_env.into(),
            type_display_context: Default::default(),
            generic_defaults: Vec::new(),
        }
    }

    pub fn clone_for_probe(&self) -> Self {
        Self {
            next_var: IdGen::with_next_raw(self.next_var.next_raw()),
            substitutions: self.substitutions.clone(),
            substitution_generation: self.substitution_generation,
            rigid_substitution_generation: self.rigid_substitution_generation,
            changed_type_vars: Vec::new(),
            trait_bounds: self.trait_bounds.clone(),
            var_spans: self.var_spans.clone(),
            var_kinds: self.var_kinds.clone(),
            normalization_env: self.normalization_env.clone(),
            type_display_context: self.type_display_context.clone(),
            generic_defaults: self.generic_defaults.clone(),
        }
    }

    pub fn set_type_display_context(
        &mut self,
        context: crate::type_services::display::TypeDisplayContext,
    ) {
        self.type_display_context = context.into();
    }

    pub fn display_type(&self, ty: &Type) -> String {
        crate::type_services::display::display_type_with_context(ty, &self.type_display_context)
            .to_string()
    }

    pub(crate) fn clone_for_commit(&self) -> Self {
        let mut clone = self.clone_for_probe();
        clone.changed_type_vars = self.changed_type_vars.clone();
        clone
    }

    pub(crate) fn commit_probe(&mut self, mut probe: Self) {
        let changed = probe.take_changed_type_vars();
        let generation_delta = probe
            .substitution_generation
            .wrapping_sub(self.substitution_generation);
        let rigid_delta = probe
            .rigid_substitution_generation
            .wrapping_sub(self.rigid_substitution_generation);
        probe.substitution_generation = self.substitution_generation.wrapping_add(generation_delta);
        probe.rigid_substitution_generation =
            self.rigid_substitution_generation.wrapping_add(rigid_delta);
        probe.changed_type_vars = changed;
        *self = probe;
    }
}

use crate::type_services::normalize::TypeNormalizer;
use crate::types::Type;

use super::Monomorphizer;

impl Monomorphizer {
    pub(super) fn intern_type(&mut self, ty: &Type) -> crate::ids::TypeId {
        let normalized = self.normalize_type(ty);
        self.type_context
            .intern_normalized_type(&normalized, &self.normalization_env)
            .expect("mono type normalization must preserve accepted HIR invariants")
    }

    pub(super) fn normalize_type(&self, ty: &Type) -> Type {
        TypeNormalizer::new(&self.normalization_env)
            .normalize(ty)
            .expect("mono type normalization must preserve accepted HIR invariants")
    }

    pub(super) fn type_for(&self, id: crate::ids::TypeId) -> Type {
        self.type_context.type_for(id)
    }
}

#[cfg(test)]
mod tests;

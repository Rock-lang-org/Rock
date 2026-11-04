use std::collections::HashMap;

use crate::crate_system::CrateContext;
use crate::ids::DefId;
use crate::types::Type;

use super::hir_types::{HirImpl, HirImplReceiverPattern};
use super::Monomorphizer;

impl Monomorphizer {
    /// Collect all trait impls from the program
    pub(super) fn collect_impls(&mut self, impls: &[HirImpl]) {
        self.collect_impls_with_body_provider(impls, |_| false);
    }

    pub(super) fn collect_impls_with_crate_capabilities(
        &mut self,
        impls: &[HirImpl],
        crate_ctx: &CrateContext,
    ) {
        self.collect_impls_with_body_provider(impls, |imp| crate_ctx.provides_impl_body(imp));
    }

    fn collect_impls_with_body_provider(
        &mut self,
        impls: &[HirImpl],
        _provided_by_dependency: impl Fn(&HirImpl) -> bool,
    ) {
        for imp in impls {
            if let Some(trait_id) = imp.trait_id {
                self.trait_impls
                    .entry(trait_id)
                    .or_insert_with(Vec::new)
                    .push(imp.clone());
            } else {
                self.generic_impls.insert(imp.id, imp.clone());
            }
        }
    }

    pub(super) fn impl_receiver_pattern_matches(&self, imp: &HirImpl, recv_ty: &Type) -> bool {
        if self
            .language_items
            .callable_protocols()
            .any(|(_, id, _)| imp.trait_id == Some(id))
        {
            if let HirImplReceiverPattern::Exact(pattern) = &imp.receiver_pattern {
                return crate::selection::type_pattern_matches(
                    pattern,
                    recv_ty,
                    &mut HashMap::new(),
                );
            }
        }
        crate::selection::impl_receiver_pattern_substitution(
            imp,
            recv_ty,
            self.trait_impls.values().flatten(),
        )
        .is_some()
    }

    pub(super) fn impl_receiver_pattern_by_id(
        &self,
        impl_id: DefId,
    ) -> Option<&HirImplReceiverPattern> {
        self.generic_impls
            .get(&impl_id)
            .or_else(|| {
                self.trait_impls
                    .values()
                    .flat_map(|impls| impls.iter())
                    .find(|imp| imp.id == impl_id)
            })
            .map(|imp| &imp.receiver_pattern)
    }

    pub(super) fn drop_receiver_matches(&self, imp: &HirImpl, recv_ty: &Type) -> bool {
        self.impl_receiver_pattern_matches(imp, recv_ty)
    }
}

#[cfg(test)]
mod tests;

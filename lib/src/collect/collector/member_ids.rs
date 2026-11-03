use super::LocalCollector;
use crate::ast;
use crate::collect::CollectedTraitMemberIds;
use crate::ids::{DefId, ModuleId};
use std::collections::HashMap;

impl LocalCollector {
    pub(super) fn item_id_at_source(
        &mut self,
        module_id: ModuleId,
        top_level_index: usize,
        top_level: &ast::TopLevel,
        span: &crate::lexer::Span,
    ) -> Option<DefId> {
        let Some(item) = self
            .id_environment
            .item_at_source(module_id, top_level_index)
        else {
            self.context.push_error_with_span(
                format!(
                    "missing canonical current-crate item identity at module {:?}, top-level {}",
                    module_id, top_level_index
                ),
                span.clone(),
            );
            return None;
        };
        if !item.kind.matches_top_level(top_level) {
            self.context.push_error_with_span(
                format!(
                    "current-crate item identity kind mismatch at module {:?}, top-level {}",
                    module_id, top_level_index
                ),
                span.clone(),
            );
            return None;
        }
        Some(item.def_id)
    }

    pub(super) fn child_module_id(
        &mut self,
        parent_id: ModuleId,
        name: &ast::Ident,
        span: &crate::lexer::Span,
    ) -> Option<ModuleId> {
        let Some(module_id) = self.id_environment.child_module_id(parent_id, &name.name) else {
            self.context.push_error_with_span(
                format!(
                    "missing canonical current-crate module identity for {} under module {:?}",
                    name.name, parent_id
                ),
                span.clone(),
            );
            return None;
        };
        Some(module_id)
    }

    pub(super) fn trait_member_ids_for(
        &mut self,
        trait_id: DefId,
        trait_name: &str,
        trait_decl: &ast::TraitDecl,
    ) -> Option<CollectedTraitMemberIds> {
        let member_ids = self
            .id_environment
            .trait_member_ids_by_owner
            .get(&trait_id)
            .cloned()
            .unwrap_or_default();
        let mut missing = false;

        for ident in trait_decl.methods.keys() {
            if !member_ids.methods.contains_key(&ident.name) {
                missing = true;
                self.context.push_error_with_span(
                    format!(
                        "missing canonical current-crate trait method identity for {trait_name}.{}",
                        ident.name
                    ),
                    ident.span.clone(),
                );
            }
        }

        for ident in trait_decl.signatures.keys() {
            if !member_ids.signatures.contains_key(&ident.name) {
                missing = true;
                self.context.push_error_with_span(
                    format!(
                    "missing canonical current-crate trait signature identity for {trait_name}.{}",
                    ident.name
                ),
                    ident.span.clone(),
                );
            }
        }

        if missing {
            None
        } else {
            Some(member_ids)
        }
    }

    pub(super) fn impl_method_ids_for(
        &mut self,
        impl_id: DefId,
        impl_key: &str,
        impl_decl: &ast::Impl,
    ) -> Option<HashMap<String, DefId>> {
        let method_ids = self
            .id_environment
            .impl_method_ids_by_owner
            .get(&impl_id)
            .cloned()
            .unwrap_or_default();
        let mut missing = false;

        for ident in impl_decl.methods.keys() {
            if !method_ids.contains_key(&ident.name) {
                missing = true;
                self.context.push_error_with_span(
                    format!(
                        "missing canonical current-crate impl method identity for {impl_key}.{}",
                        ident.name
                    ),
                    ident.span.clone(),
                );
            }
        }

        if missing {
            None
        } else {
            Some(method_ids)
        }
    }
}

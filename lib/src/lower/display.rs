//! Type display helpers for the lowerer.

use super::*;

impl Lowerer {
    pub(crate) fn type_display_context(&self) -> crate::type_services::display::TypeDisplayContext {
        let mut context =
            crate::type_services::display::TypeDisplayContext::from_resolver(&self.resolver);
        for resolver in self.dependency_resolvers.values() {
            context.extend_resolver(resolver);
        }
        for (_, function) in self.items.functions() {
            for generic in &function.generic_params {
                context.insert_generic_name(generic.id, generic.name.clone());
            }
        }
        for (_, signature) in self.items.function_sigs() {
            for generic in &signature.generic_params {
                context.insert_generic_name(generic.id, generic.name.clone());
            }
        }
        for (_, structure) in self.items.structures() {
            for generic in &structure.generic_params {
                context.insert_generic_name(generic.id, generic.name.clone());
            }
        }
        for (_, enumeration) in self.items.enumerations() {
            for generic in &enumeration.generic_params {
                context.insert_generic_name(generic.id, generic.name.clone());
            }
        }
        for (_, trait_def) in self.items.trait_defs() {
            for generic in &trait_def.generic_params {
                context.insert_generic_name(generic.id, generic.name.clone());
            }
            if let Some(target) = &trait_def.target {
                context.insert_generic_name(target.id, target.name.clone());
            }
            for associated in &trait_def.associated_types {
                context.insert_associated_name(
                    crate::types::AssociatedTypeKey {
                        owner: trait_def.id,
                        assoc_type_id: associated.id,
                    },
                    associated.name.clone(),
                );
            }
            for function in trait_def.methods.values() {
                for generic in &function.generic_params {
                    context.insert_generic_name(generic.id, generic.name.clone());
                }
            }
            for signature in trait_def.signatures.values() {
                for generic in &signature.generic_params {
                    context.insert_generic_name(generic.id, generic.name.clone());
                }
            }
        }
        for (_, impl_def) in self.items.impl_defs() {
            for generic in impl_def
                .type_generics
                .iter()
                .chain(&impl_def.trait_generics)
            {
                context.insert_generic_name(generic.id, generic.name.clone());
            }
            for associated in &impl_def.associated_types {
                context.insert_associated_name(
                    crate::types::AssociatedTypeKey {
                        owner: impl_def.id,
                        assoc_type_id: associated.id,
                    },
                    associated.name.clone(),
                );
            }
            for function in impl_def.methods.values() {
                for generic in &function.generic_params {
                    context.insert_generic_name(generic.id, generic.name.clone());
                }
            }
        }
        context
    }

    pub(crate) fn display_type(&self, ty: &Type) -> String {
        crate::type_services::display::display_type_with_context(ty, &self.type_display_context())
            .to_string()
    }

    pub(crate) fn refresh_type_display_context(&mut self) {
        let context = self.type_display_context();
        self.engine.set_type_display_context(context);
    }

    pub(crate) fn display_selection_error(
        &self,
        error: &crate::selection::SelectionDiagnostic,
    ) -> String {
        error.message_with_names(
            |ty| self.display_type(ty),
            |id| {
                crate::lower::resolution::LowerResolutionContext::new(self)
                    .canonical_name(id)
                    .map(str::to_string)
            },
        )
    }
}

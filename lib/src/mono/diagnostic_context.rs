use crate::source_map::SourceSymbol;
use crate::types::Type;

use super::hir_types::HirImpl;
use super::Monomorphizer;

impl Monomorphizer {
    pub(super) fn diagnostic_context(&self) -> crate::type_services::display::TypeDisplayContext {
        let mut context =
            crate::type_services::display::TypeDisplayContext::from_resolver(&self.resolver);
        for resolver in &self.dependency_resolvers {
            context.extend_resolver(resolver);
        }
        for function in self
            .concrete_functions
            .values()
            .chain(self.generic_functions.values())
        {
            context.insert_generic_names(&function.generic_params);
        }
        let mut extend_impl = |impl_def: &HirImpl| {
            context.insert_generic_names(&impl_def.type_generics);
            context.insert_generic_names(&impl_def.trait_generics);
            for associated in &impl_def.associated_types {
                for owner in [Some(impl_def.id), impl_def.trait_id] {
                    if let Some(owner) = owner {
                        context.insert_associated_name(
                            crate::types::AssociatedTypeKey {
                                owner,
                                assoc_type_id: associated.id,
                            },
                            associated.name.clone(),
                        );
                    }
                }
            }
            for method in impl_def.methods.values() {
                context.insert_generic_names(&method.generic_params);
            }
        };
        for impls in self.trait_impls.values() {
            for impl_def in impls {
                extend_impl(impl_def);
            }
        }
        for impl_def in self.generic_impls.values() {
            extend_impl(impl_def);
        }
        context
    }

    pub(super) fn display_type_for_diagnostic(&self, ty: &Type) -> String {
        let context = self.diagnostic_context();
        crate::type_services::display::display_type_with_context(ty, &context).to_string()
    }

    pub(super) fn source_span_for_type(&self, ty: &Type) -> Option<crate::lexer::Span> {
        let symbol = match ty {
            Type::Struct { id, .. } | Type::Enum { id, .. } | Type::Constructor { id, .. } => {
                SourceSymbol::Definition(*id)
            }
            Type::Generic(param) => SourceSymbol::Generic(*param),
            Type::Projection { assoc_type, .. } => SourceSymbol::AssociatedType {
                owner: assoc_type.owner,
                associated: assoc_type.assoc_type_id,
            },
            _ => return None,
        };
        self.source_map.symbol(&symbol).map(|info| {
            info.declaration_span
                .as_ref()
                .unwrap_or(&info.name_span)
                .clone()
        })
    }
}

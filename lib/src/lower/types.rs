//! Type lowering methods for Lowerer

use crate::ast;
use crate::hir::HirTrait;
use crate::type_lowering::{ResolvedNominalType, TypeLowerer, TypeLoweringContext};
use crate::types::Type;

use crate::lower::Lowerer;

impl TypeLoweringContext for Lowerer {
    fn constructor_params(&self, id: crate::ids::DefId) -> Option<Vec<crate::types::GenericParamDecl>> {
        self.items.structures().find(|(_, item)| item.id == id).map(|(_, item)| item.generic_params.clone())
            .or_else(|| self.items.enumerations().find(|(_, item)| item.id == id).map(|(_, item)| item.generic_params.clone()))
            .or_else(|| self.items.type_alias(id).map(|item| item.generic_params.clone()))
    }
    fn push_type_error(&mut self, message: String, span: crate::lexer::Span) {
        self.diagnostics.push(message, span);
    }

    fn record_type_reference(
        &mut self,
        span: crate::lexer::Span,
        target: crate::source_map::SourceSymbol,
    ) {
        self.record_source_reference(span, target);
    }

    fn current_module_prefix(&self) -> Option<String> {
        Lowerer::current_module_prefix(self)
    }

    fn current_trait_name(&self) -> Option<String> {
        self.current_trait.clone()
    }

    fn resolve_nominal_type(&self, name: &str) -> Option<ResolvedNominalType> {
        crate::lower::resolution::LowerResolutionContext::new(self).resolve_nominal_type(name)
    }

    fn resolve_trait_type(&self, name: &str) -> Option<HirTrait> {
        crate::lower::resolution::LowerResolutionContext::new(self).resolve_trait_type(name)
    }

    fn resolve_type_alias(&self, name: &str) -> Option<crate::hir::HirTypeAlias> {
        let id =
            crate::lower::resolution::LowerResolutionContext::new(self).resolve_item_id(name)?;
        self.items.type_alias(id).cloned()
    }

    fn generic_type_for_name(&mut self, name: &str, span: crate::lexer::Span) -> Type {
        self.current_generic_type_for_name(name, span)
            .unwrap_or(Type::Error)
    }

    fn populate_type_normalization_env(
        &self,
        env: &mut crate::type_services::normalize::TypeNormalizationEnv,
    ) {
        for (_, structure) in self.items.structures() {
            env.register_constructor(
                structure.id,
                crate::types::NominalTypeKind::Struct,
                crate::type_lowering::constructor_kind(&structure.generic_params),
            );
            for param in &structure.generic_params {
                env.register_generic_kind(param.id, param.kind.clone());
            }
        }
        for (_, enumeration) in self.items.enumerations() {
            env.register_constructor(
                enumeration.id,
                crate::types::NominalTypeKind::Enum,
                crate::type_lowering::constructor_kind(&enumeration.generic_params),
            );
            for param in &enumeration.generic_params {
                env.register_generic_kind(param.id, param.kind.clone());
            }
        }
        for (_, trait_def) in self.items.trait_defs() {
            for param in &trait_def.generic_params {
                env.register_generic_kind(param.id, param.kind.clone());
            }
            if let Some(target) = &trait_def.target {
                env.register_generic_kind(target.id, target.kind.clone());
            }
            for associated_type in &trait_def.associated_types {
                env.register_projection_kind(
                    crate::types::AssociatedTypeKey {
                        owner: trait_def.id,
                        assoc_type_id: associated_type.id,
                    },
                    associated_type.kind.clone(),
                );
            }
            for method in trait_def.methods.values() {
                for param in &method.generic_params {
                    env.register_generic_kind(param.id, param.kind.clone());
                }
            }
            for signature in trait_def.signatures.values() {
                for param in &signature.generic_params {
                    env.register_generic_kind(param.id, param.kind.clone());
                }
            }
        }
        for (_, function) in self.items.functions() {
            for param in &function.generic_params {
                env.register_generic_kind(param.id, param.kind.clone());
            }
        }
        for (_, signature) in self.items.function_sigs() {
            for param in &signature.generic_params {
                env.register_generic_kind(param.id, param.kind.clone());
            }
        }
        for (_, imp) in self.items.impl_defs() {
            for param in imp.type_generics.iter().chain(&imp.trait_generics) {
                env.register_generic_kind(param.id, param.kind.clone());
            }
            for method in imp.methods.values() {
                for param in &method.generic_params {
                    env.register_generic_kind(param.id, param.kind.clone());
                }
            }
        }
        crate::type_lowering::register_type_aliases(
            env,
            self.items.type_aliases().map(|(_, alias)| alias),
        );
    }
}

impl Lowerer {
    pub(crate) fn refresh_inference_normalization_env(&mut self) {
        let mut env = crate::type_services::normalize::TypeNormalizationEnv::new();
        self.populate_type_normalization_env(&mut env);
        self.engine.set_normalization_env(env);
    }

    pub(crate) fn lower_parse_type(&mut self, parse_type: &ast::ParseType) -> Type {
        TypeLowerer::lower_parse_type(self, parse_type)
    }
}

#[cfg(test)]
mod tests;

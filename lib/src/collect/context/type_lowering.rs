use super::type_names::{collect_generic_names_from_parse_type, is_known_nominal_type_name};
use super::CollectContext;
use crate::ast;
use crate::hir::{HirParam, HirTrait};
use crate::ids::DefId;
use crate::lexer::Span;
use crate::type_lowering::{ResolvedNominalType, TypeLowerer, TypeLoweringContext};
use crate::types::{GenericParamId, Type};
use std::collections::HashSet;

impl TypeLoweringContext for CollectContext {
    fn constructor_params(&self, id: DefId) -> Option<Vec<crate::types::GenericParamDecl>> {
        self.structs
            .values()
            .find(|item| item.id == id)
            .map(|item| item.generic_params.clone())
            .or_else(|| {
                self.enums
                    .values()
                    .find(|item| item.id == id)
                    .map(|item| item.generic_params.clone())
            })
            .or_else(|| {
                self.type_aliases
                    .values()
                    .find(|item| item.id == id)
                    .map(|item| item.generic_params.clone())
            })
    }

    fn push_type_error(&mut self, message: String, span: Span) {
        self.push_error_with_span(message, span);
    }

    fn current_module_prefix(&self) -> Option<String> {
        CollectContext::current_module_prefix(self)
    }

    fn current_trait_name(&self) -> Option<String> {
        self.current_trait.clone()
    }

    fn resolve_nominal_type(&self, name: &str) -> Option<ResolvedNominalType> {
        if let Some(structure) = self.structs.get(name).cloned() {
            return Some(ResolvedNominalType::Struct(structure));
        }

        if let Some(enum_def) = self.enums.get(name).cloned() {
            return Some(ResolvedNominalType::Enum(enum_def));
        }

        if let Some(id) = self.canonical_import_aliases.get(name).copied() {
            if let Some(structure) = self
                .structs
                .values()
                .find(|structure| structure.id == id)
                .cloned()
            {
                return Some(ResolvedNominalType::Struct(structure));
            }

            if let Some(enum_def) = self
                .enums
                .values()
                .find(|enum_def| enum_def.id == id)
                .cloned()
            {
                return Some(ResolvedNominalType::Enum(enum_def));
            }
        }

        if let Some(qualified) = self.import_aliases.get(name) {
            if let Some(structure) = self.structs.get(qualified).cloned() {
                return Some(ResolvedNominalType::Struct(structure));
            }

            if let Some(enum_def) = self.enums.get(qualified).cloned() {
                return Some(ResolvedNominalType::Enum(enum_def));
            }
        }

        None
    }

    fn resolve_trait_type(&self, name: &str) -> Option<HirTrait> {
        if let Some(trait_def) = self.traits.get(name).cloned() {
            return Some(trait_def);
        }

        if let Some(id) = self.canonical_import_aliases.get(name).copied() {
            if let Some(trait_def) = self.traits.values().find(|trait_def| trait_def.id == id) {
                return Some(trait_def.clone());
            }
        }

        self.import_aliases
            .get(name)
            .and_then(|qualified| self.traits.get(qualified))
            .cloned()
    }

    fn resolve_type_alias(&self, name: &str) -> Option<crate::hir::HirTypeAlias> {
        self.type_aliases
            .get(name)
            .cloned()
            .or_else(|| {
                self.canonical_import_aliases.get(name).and_then(|id| {
                    self.type_aliases
                        .values()
                        .find(|alias| alias.id == *id)
                        .cloned()
                })
            })
            .or_else(|| {
                self.import_aliases
                    .get(name)
                    .and_then(|qualified| self.type_aliases.get(qualified))
                    .cloned()
            })
    }

    fn generic_type_for_name(&mut self, name: &str, span: Span) -> Type {
        CollectContext::generic_type_for_name(self, name, span)
    }

    fn populate_type_normalization_env(
        &self,
        env: &mut crate::type_services::normalize::TypeNormalizationEnv,
    ) {
        for structure in self.structs.values() {
            env.register_constructor(
                structure.id,
                crate::types::NominalTypeKind::Struct,
                crate::type_lowering::constructor_kind(&structure.generic_params),
            );
            for param in &structure.generic_params {
                env.register_generic_kind(param.id, param.kind.clone());
            }
        }
        for enumeration in self.enums.values() {
            env.register_constructor(
                enumeration.id,
                crate::types::NominalTypeKind::Enum,
                crate::type_lowering::constructor_kind(&enumeration.generic_params),
            );
            for param in &enumeration.generic_params {
                env.register_generic_kind(param.id, param.kind.clone());
            }
        }
        for trait_def in self.traits.values() {
            for param in &trait_def.generic_params {
                env.register_generic_kind(param.id, param.kind.clone());
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
            for associated_type in &trait_def.associated_types {
                env.register_projection_kind(
                    crate::types::AssociatedTypeKey {
                        owner: trait_def.id,
                        assoc_type_id: associated_type.id,
                    },
                    associated_type.kind.clone(),
                );
            }
        }
        for function in self.functions.values() {
            for param in &function.generic_params {
                env.register_generic_kind(param.id, param.kind.clone());
            }
        }
        for signature in self.function_sigs.values() {
            for param in &signature.generic_params {
                env.register_generic_kind(param.id, param.kind.clone());
            }
        }
        for imp in &self.impls {
            for param in imp.type_generics.iter().chain(&imp.trait_generics) {
                env.register_generic_kind(param.id, param.kind.clone());
            }
            for method in imp.methods.values() {
                for param in &method.generic_params {
                    env.register_generic_kind(param.id, param.kind.clone());
                }
            }
        }
        crate::type_lowering::register_type_aliases(env, self.type_aliases.values());
        if let Some(owner) = self.current_generic_owner {
            for (index, kind) in self.current_generic_kinds.iter().enumerate() {
                env.register_generic_kind(
                    GenericParamId {
                        owner,
                        index: index as u32,
                    },
                    kind.clone(),
                );
            }
        }
    }
}

impl CollectContext {
    pub(crate) fn generic_type_for_name(&mut self, name: &str, span: Span) -> Type {
        let Some(owner) = self.current_generic_owner else {
            self.push_error_with_span(
                format!(
                    "generic type parameter '{}' appears outside a generic declaration",
                    name
                ),
                span,
            );
            return Type::Error;
        };

        let Some(index) = self
            .current_generic_params
            .iter()
            .position(|param| param == name)
            .or_else(|| {
                let mut chars = name.chars();
                let is_implicit_generic = matches!(chars.next(), Some(ch) if ch.is_ascii_uppercase())
                    && chars.next().is_none();
                if is_implicit_generic {
                    self.current_generic_params.push(name.to_string());
                    self.current_generic_kinds
                        .push(crate::type_services::kind::Kind::Type);
                    Some(self.current_generic_params.len() - 1)
                } else {
                    None
                }
            }) else {
            self.push_error_with_span(format!("unknown type '{}'", name), span);
            return Type::Error;
        };

        Type::Generic(GenericParamId {
            owner,
            index: index as u32,
        })
    }

    pub(crate) fn push_generic_context_with_kinds_at(
        &mut self,
        owner: DefId,
        params: Vec<String>,
        kinds: Vec<crate::type_services::kind::Kind>,
        spans: &[Span],
    ) -> (
        Option<DefId>,
        Vec<String>,
        Vec<crate::type_services::kind::Kind>,
    ) {
        debug_assert_eq!(params.len(), kinds.len());
        let mut seen = HashSet::new();
        for (index, param) in params.iter().enumerate() {
            if !seen.insert(param.clone()) {
                if let Some(span) = spans.get(index).cloned() {
                    self.push_error_with_span(
                        format!("duplicate generic parameter '{}'", param),
                        span,
                    );
                } else {
                    self.push_error(format!("duplicate generic parameter '{}'", param));
                }
            }
        }
        let prev_owner = self.current_generic_owner;
        let prev_params = std::mem::replace(&mut self.current_generic_params, params);
        let prev_kinds = std::mem::replace(&mut self.current_generic_kinds, kinds);
        self.current_generic_owner = Some(owner);
        (prev_owner, prev_params, prev_kinds)
    }

    pub(crate) fn pop_generic_context_with_kinds(
        &mut self,
        prev_owner: Option<DefId>,
        prev_params: Vec<String>,
        prev_kinds: Vec<crate::type_services::kind::Kind>,
    ) {
        self.current_generic_params = prev_params;
        self.current_generic_kinds = prev_kinds;
        self.current_generic_owner = prev_owner;
    }

    pub(crate) fn build_self_param_at(
        &mut self,
        self_receiver: ast::SelfReceiverMode,
        span: crate::lexer::Span,
    ) -> HirParam {
        HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: self.receiver_ty_for_self(self_receiver, span),
            mutable: matches!(self_receiver, ast::SelfReceiverMode::Mut),
            is_ref: false,
        }
    }

    pub(crate) fn receiver_ty_for_self(
        &mut self,
        self_receiver: ast::SelfReceiverMode,
        span: crate::lexer::Span,
    ) -> Type {
        let base = if let Some(index) = self
            .current_generic_params
            .iter()
            .position(|param| param == "Self")
        {
            self.current_generic_owner
                .map(|owner| {
                    Type::Generic(GenericParamId {
                        owner,
                        index: index as u32,
                    })
                })
                .unwrap_or_else(|| self.type_vars.fresh_type_var_at(span.clone()))
        } else {
            self.type_vars.fresh_type_var_at(span)
        };

        match self_receiver {
            ast::SelfReceiverMode::Shared => Type::Reference {
                mutable: false,
                inner: Box::new(base),
            },
            ast::SelfReceiverMode::Mut => Type::Reference {
                mutable: true,
                inner: Box::new(base),
            },
            ast::SelfReceiverMode::Move => base,
        }
    }

    pub(crate) fn lower_param_pattern_at(
        &mut self,
        pattern: &ast::Pattern,
        fallback: crate::lexer::Span,
    ) -> (String, Type, bool, bool) {
        match &pattern.kind {
            ast::PatternKind::Reference {
                pattern: inner,
                mutable,
            } => {
                let (name, ty, inner_mut, _) = self.lower_param_pattern_at(inner, fallback.clone());
                (name, ty, inner_mut || *mutable, true)
            }
            ast::PatternKind::Nested(inner) => self.lower_param_pattern_at(inner, fallback.clone()),
            ast::PatternKind::Ident(ident_pat) => {
                let ty = self
                    .type_vars
                    .fresh_type_var_at(ident_pat.name.span.clone());
                (ident_pat.name.name.clone(), ty, ident_pat.mut_, false)
            }
            ast::PatternKind::Wildcard => {
                let ty = self.type_vars.fresh_type_var_at(fallback.clone());
                ("_".to_string(), ty, false, false)
            }
            _ => {
                let ty = self.type_vars.fresh_type_var_at(fallback);
                let name = pattern
                    .binding
                    .as_ref()
                    .map(|binding| binding.name.clone())
                    .unwrap_or_else(|| "_arg".to_string());
                (name, ty, false, false)
            }
        }
    }

    pub(crate) fn lower_parse_type(&mut self, parse_type: &ast::ParseType) -> Type {
        TypeLowerer::lower_parse_type(self, parse_type)
    }

    pub(crate) fn impl_type_info(
        &mut self,
        imp: &ast::Impl,
    ) -> (String, Vec<String>, crate::hir::HirImplReceiverPattern) {
        let receiver = imp.for_.clone().unwrap_or_else(|| imp.name.clone());
        let mut type_generics = Vec::new();
        collect_generic_names_from_parse_type(&receiver, &mut type_generics, &|name| {
            is_known_nominal_type_name(self, name)
        });
        let pattern = crate::hir::HirImplReceiverPattern::Exact(self.lower_parse_type(&receiver));
        (receiver.type_name(), type_generics, pattern)
    }
}

#[cfg(test)]
mod tests;

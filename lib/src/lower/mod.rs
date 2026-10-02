//! Lowerer context: struct definition, constructors, and helper functions

pub mod bodies;
pub(crate) mod body_context;
pub(crate) mod body_lowerer;
pub mod collect;
pub mod control_flow;
pub mod crates;
pub(crate) mod diagnostics;
pub mod error;
pub mod expression;
mod existential;
pub mod function;
pub(crate) mod inference_scc;
pub mod intrinsics;
pub(crate) mod items;
pub(crate) mod module_context;
mod objects;
mod owner_coercion;
pub mod paths;
pub(crate) mod pipeline;
pub(crate) mod prelude;
pub mod program;
pub(crate) mod resolution;
pub mod scope;
pub(crate) mod services;
pub(crate) mod session;
pub mod statement;
pub mod traits;
pub mod r#types;
pub mod types_helpers;

pub use error::{ResolveError, ResolveErrorKind};
pub use program::lower_with_crates_and_options;

use std::collections::{BTreeSet, HashMap, HashSet};

use items::LowerItems;
use services::{
    LowerConstraintService, LowerDependencyResolverService, LowerDiagnosticService,
    LowerInferenceService, LowerItemService, LowerModuleService, LowerPreludeService,
    LowerResolverService, LowerScopeService, LowererServices,
};

use body_context::{BodyLoweringContext, BodyOwner, GenericLoweringContext};

use crate::ast;
use crate::collect::item_index::ItemIndex;
#[cfg(test)]
use crate::collect::resolver::ResolverTables;
use crate::hir::*;
use crate::ids::{CrateId, DefId, HirLocalId, IdGen, LocalDefId, TypeVarId};
use crate::infer::constraints::ConstraintOwner;
use crate::lexer::Span;
use crate::types::{GenericParamId, Type};

/// The main lowering context
pub struct Lowerer {
    pub(crate) engine: LowerInferenceService,
    pub(crate) scope: LowerScopeService,
    pub(crate) items: LowerItemService,
    pub(crate) diagnostics: LowerDiagnosticService,
    pub(crate) modules: LowerModuleService,
    pub(crate) prelude: LowerPreludeService,
    pub(crate) function_type_vars: HashMap<DefId, HashSet<TypeVarId>>,
    pub(crate) inference_sccs: HashMap<DefId, DefId>,
    pub(crate) inference_scc_order: Vec<DefId>,
    pub(crate) current_trait: Option<String>,
    pub(crate) current_trait_id: Option<DefId>,
    pub(crate) current_trait_generics: Vec<String>,
    pub(crate) generic_context: Option<GenericLoweringContext>,
    pub(crate) current_struct_impl: Option<String>,
    pub(crate) empty_impl_bounds: HirGenericBounds,
    pub(crate) infix_precedence: HashMap<String, u8>,
    pub(crate) current_impl_id: Option<DefId>,
    /// Canonical resolver tables from declaration collection.
    pub(crate) resolver: LowerResolverService,
    pub(crate) current_def_ids: BTreeSet<crate::ids::DefId>,
    /// Canonical resolver tables for dependency crates.
    pub(crate) dependency_resolvers: LowerDependencyResolverService,
    /// Deferred type constraints accumulated during lowering (trait bounds, literal types)
    pub(crate) constraint_store: LowerConstraintService,
    /// Canonical ID state carried from collection for downstream phases.
    pub(crate) root_crate_id: CrateId,
    pub(crate) local_def_ids: IdGen<LocalDefId>,
    pub(crate) item_index: ItemIndex,
    pub(crate) language_items: HirLanguageItems,
    pub(crate) imported_effective_trait_methods: HashMap<(DefId, DefId), DefId>,
    pub(crate) body_context: Option<BodyLoweringContext>,
    pub(crate) tuple_temp_counter: u32,
    pub(crate) source_map: crate::source_map::SemanticSourceMap,
    pub(crate) source_scope_stack: Vec<u32>,
    pub(crate) fold_loop_stack: Vec<Option<control_flow::fold_loop::FoldLoopContext>>,
    pub(crate) opened_witnesses: Vec<(String, HirOpenBinding)>,
}

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

    pub fn new() -> Self {
        Self::with_options(true)
    }

    #[cfg(test)]
    pub(crate) fn new_for_test() -> Self {
        let lowerer = Self::new();
        lowerer
    }

    pub(crate) fn from_services(services: LowererServices) -> Self {
        Self {
            engine: services.engine,
            scope: services.scope,
            items: services.items,
            diagnostics: services.diagnostics,
            modules: services.modules,
            prelude: services.prelude,
            function_type_vars: HashMap::new(),
            inference_sccs: HashMap::new(),
            inference_scc_order: Vec::new(),
            current_trait: None,
            current_trait_id: None,
            current_trait_generics: Vec::new(),
            generic_context: None,
            current_struct_impl: None,
            empty_impl_bounds: HirGenericBounds::new(),
            infix_precedence: HashMap::new(),
            current_impl_id: None,
            resolver: services.resolver,
            current_def_ids: BTreeSet::new(),
            dependency_resolvers: services.dependency_resolvers,
            constraint_store: services.constraint_store,
            root_crate_id: CrateId(0),
            local_def_ids: IdGen::<LocalDefId>::new(),
            item_index: ItemIndex::default(),
            language_items: HirLanguageItems::default(),
            imported_effective_trait_methods: HashMap::new(),
            body_context: None,
            tuple_temp_counter: 0,
            source_map: Default::default(),
            source_scope_stack: Vec::new(),
            fold_loop_stack: Vec::new(),
            opened_witnesses: Vec::new(),
        }
    }

    pub(crate) fn should_instantiate_inferred_function(&self, function_id: DefId) -> bool {
        let Some(owner) = self.body_context.as_ref().map(|context| context.owner()) else {
            return true;
        };
        let current_id = match owner {
            BodyOwner::Function(id)
            | BodyOwner::ImplMethod { method_id: id, .. }
            | BodyOwner::TraitMethod { method_id: id, .. } => *id,
        };

        if current_id == function_id {
            return false;
        }

        self.inference_sccs.get(&current_id) != self.inference_sccs.get(&function_id)
    }

    pub fn with_options(inject_prelude: bool) -> Self {
        Self::from_services(LowererServices::new(inject_prelude))
    }

    #[allow(dead_code)]
    pub(crate) fn fresh_local_id(&mut self) -> HirLocalId {
        self.body_context
            .as_mut()
            .expect("local IDs can only be allocated while lowering a body")
            .fresh_local_id()
    }

    pub(crate) fn with_body_context<R>(
        &mut self,
        context: BodyLoweringContext,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let previous_context = self.body_context.take();
        let previous_scope = (*self.scope).clone();
        let mut context = context;
        context.replace_scope(previous_scope.clone());
        self.scope =
            LowerScopeService::new(context.replace_scope(crate::lower::scope::Scope::new()));
        let owner = match context.owner() {
            BodyOwner::Function(id)
            | BodyOwner::ImplMethod { method_id: id, .. }
            | BodyOwner::TraitMethod { method_id: id, .. } => ConstraintOwner::Body(*id),
        };
        let source_owner = match context.owner() {
            BodyOwner::Function(id)
            | BodyOwner::ImplMethod { method_id: id, .. }
            | BodyOwner::TraitMethod { method_id: id, .. } => *id,
        };
        let previous_source_scope_stack =
            std::mem::replace(&mut self.source_scope_stack, Vec::new());
        let root_scope = self.source_map.insert_scope(source_owner, None);
        self.source_scope_stack.push(root_scope);
        let previous_constraint_owner = self.constraint_store.replace_owner(owner);
        self.body_context = Some(context);

        let result = f(self);

        if let Some(context) = self.body_context.as_mut() {
            context.replace_scope((*self.scope).clone());
        }
        self.scope = LowerScopeService::new(previous_scope);
        self.constraint_store
            .replace_owner(previous_constraint_owner);
        self.body_context = previous_context;
        self.source_scope_stack = previous_source_scope_stack;

        result
    }

    pub(crate) fn push_scope(&mut self) {
        self.scope.push();
        let Some(owner) = self.current_body_def_id() else {
            return;
        };
        let parent = self.source_scope_stack.last().copied();
        let scope = self.source_map.insert_scope(owner, parent);
        self.source_scope_stack.push(scope);
    }

    pub(crate) fn pop_scope(&mut self) {
        self.scope.pop();
        if self.current_body_def_id().is_some() {
            self.source_scope_stack.pop();
        }
    }

    #[allow(dead_code)]
    pub(crate) fn current_function_name(&self) -> Option<&str> {
        self.body_context
            .as_ref()
            .map(BodyLoweringContext::function_name)
    }

    #[allow(dead_code)]
    pub(crate) fn current_body_owner(&self) -> Option<BodyOwner> {
        self.body_context
            .as_ref()
            .map(|context| context.owner().clone())
    }

    pub(crate) fn current_body_def_id(&self) -> Option<DefId> {
        match self.current_body_owner()? {
            BodyOwner::Function(id)
            | BodyOwner::ImplMethod { method_id: id, .. }
            | BodyOwner::TraitMethod { method_id: id, .. } => Some(id),
        }
    }

    pub(crate) fn record_source_reference(
        &mut self,
        span: Span,
        target: crate::source_map::SourceSymbol,
    ) {
        let owner = self.current_body_def_id().or_else(|| match &target {
            crate::source_map::SourceSymbol::Generic(generic) => Some(generic.owner),
            _ => None,
        });
        let scope_id = self.source_scope_stack.last().copied();
        self.source_map
            .record_reference_in_scope(span, target, owner, scope_id);
    }

    pub(crate) fn pattern_binding_span(pattern: &ast::Pattern) -> Option<Span> {
        match &pattern.kind {
            ast::PatternKind::Ident(binding) => Some(binding.name.span.clone()),
            ast::PatternKind::Nested(pattern) | ast::PatternKind::Reference { pattern, .. } => {
                Self::pattern_binding_span(pattern)
            }
            ast::PatternKind::Tuple(patterns) => {
                patterns.iter().find_map(Self::pattern_binding_span)
            }
            ast::PatternKind::Array(patterns) => {
                patterns.iter().find_map(|pattern| match pattern {
                    ast::ArrayPattern::Pattern(pattern) => Self::pattern_binding_span(pattern),
                    ast::ArrayPattern::Rest(binding) => Some(binding.name.span.clone()),
                })
            }
            ast::PatternKind::Instance(instance) => match &instance.args {
                ast::FieldsPatternOrArgumentsPattern::Fields(fields) => fields
                    .iter()
                    .find_map(|field| Self::pattern_binding_span(&field.pattern)),
                ast::FieldsPatternOrArgumentsPattern::Arguments(patterns) => {
                    patterns.iter().find_map(Self::pattern_binding_span)
                }
            },
            ast::PatternKind::Wildcard | ast::PatternKind::Literal(_) => {
                pattern.binding.as_ref().map(|binding| binding.span.clone())
            }
        }
    }

    pub(crate) fn current_body_return_type(&mut self) -> Option<Type> {
        if let Some(return_type) = self
            .body_context
            .as_ref()
            .and_then(BodyLoweringContext::return_type_override)
        {
            return Some(return_type.clone());
        }

        match self.current_body_owner()? {
            BodyOwner::Function(function_id) => self
                .items
                .function(function_id)
                .map(|function| function.ret_type.clone()),
            BodyOwner::ImplMethod {
                impl_id,
                method_id,
                method_name,
            } => {
                let method = self
                    .items
                    .impl_def(impl_id)
                    .and_then(|impl_def| impl_def.methods.get(&method_name));
                let Some(method) = method else {
                    self.diagnostics.push_toolchain(format!(
                        "lowering invariant: impl {impl_id:?} has no body method '{method_name}'"
                    ));
                    return None;
                };
                if method.id != method_id {
                    self.diagnostics.push_toolchain(format!(
                        "lowering invariant: impl {impl_id:?} method '{method_name}' has DefId {:?}, expected {method_id:?}",
                        method.id
                    ));
                    return None;
                }
                Some(method.ret_type.clone())
            }
            BodyOwner::TraitMethod {
                trait_id,
                method_id,
                method_name,
            } => {
                let method = self
                    .items
                    .trait_def(trait_id)
                    .and_then(|trait_def| trait_def.methods.get(&method_name));
                let Some(method) = method else {
                    self.diagnostics.push_toolchain(format!(
                        "lowering invariant: trait {trait_id:?} has no body method '{method_name}'"
                    ));
                    return None;
                };
                if method.id != method_id {
                    self.diagnostics.push_toolchain(format!(
                        "lowering invariant: trait {trait_id:?} method '{method_name}' has DefId {:?}, expected {method_id:?}",
                        method.id
                    ));
                    return None;
                }
                Some(method.ret_type.clone())
            }
        }
    }

    pub(crate) fn with_body_return_type<R>(
        &mut self,
        return_type: Type,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let Some(context) = self.body_context.as_mut() else {
            return f(self);
        };
        let previous = context.replace_return_type_override(Some(return_type));
        let result = f(self);
        if let Some(context) = self.body_context.as_mut() {
            context.replace_return_type_override(previous);
        }
        result
    }

    pub(crate) fn current_generic_owner(&self) -> Option<DefId> {
        self.body_context
            .as_ref()
            .and_then(BodyLoweringContext::generic_owner)
            .or_else(|| {
                self.generic_context
                    .as_ref()
                    .map(GenericLoweringContext::owner)
            })
    }

    pub(crate) fn current_generic_params(&self) -> &[String] {
        if let Some(context) = self.body_context.as_ref() {
            context.generic_params()
        } else if let Some(context) = self.generic_context.as_ref() {
            context.params()
        } else {
            &[]
        }
    }

    pub(crate) fn current_impl_bounds(&self) -> &HirGenericBounds {
        self.body_context
            .as_ref()
            .map(BodyLoweringContext::impl_bounds)
            .unwrap_or(&self.empty_impl_bounds)
    }

    pub(crate) fn is_in_unsafe(&self) -> bool {
        self.body_context
            .as_ref()
            .is_some_and(BodyLoweringContext::in_unsafe)
    }

    #[cfg(test)]
    pub(crate) fn with_test_body_context<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        self.with_body_context(
            BodyLoweringContext::new(
                "test".to_string(),
                BodyOwner::Function(DefId::new(CrateId(0), LocalDefId(u32::MAX - 1))),
                self.current_generic_owner(),
                self.current_generic_params().to_vec(),
                self.current_impl_bounds().clone(),
                false,
            ),
            f,
        )
    }

    fn current_generic_params_mut(&mut self) -> Option<&mut Vec<String>> {
        if let Some(context) = self.body_context.as_mut() {
            Some(context.generic_params_mut())
        } else {
            self.generic_context
                .as_mut()
                .map(GenericLoweringContext::params_mut)
        }
    }

    pub(crate) fn has_errors(&self) -> bool {
        let has_errors = !self.diagnostics.is_empty();
        debug_assert_eq!(has_errors, !self.errors().is_empty());
        has_errors
    }

    pub(crate) fn errors(&self) -> &[ResolveError] {
        self.diagnostics.errors()
    }

    #[allow(dead_code)]
    pub(crate) fn resolve_item_def_id(&self, name: &str) -> Option<DefId> {
        crate::lower::resolution::LowerResolutionContext::new(self).resolve_item_id(name)
    }

    // Staging facades retained while lower resolution call sites migrate task-by-task.
    #[allow(dead_code)]
    pub(crate) fn resolve_module_alias_or_item_def_id(&self, name: &str) -> Option<DefId> {
        crate::lower::resolution::LowerResolutionContext::new(self)
            .resolve_module_alias_or_item_id(name)
    }

    #[allow(dead_code)]
    pub(crate) fn resolve_item_def_id_for_path(&self, name: &str) -> Option<DefId> {
        crate::lower::resolution::LowerResolutionContext::new(self).resolve_item_id_for_path(name)
    }

    pub(crate) fn canonical_name_for_def_id(&self, id: DefId) -> Option<&str> {
        crate::lower::resolution::LowerResolutionContext::new(self).canonical_name(id)
    }

    #[allow(dead_code)]
    pub(crate) fn canonical_name_for_alias_or_item(&self, name: &str) -> Option<String> {
        crate::lower::resolution::LowerResolutionContext::new(self).canonical_name_for_item(name)
    }

    #[allow(dead_code)]
    pub(crate) fn canonical_name_for_alias_or_item_lossy(&self, name: &str) -> String {
        self.canonical_name_for_alias_or_item(name)
            .unwrap_or_else(|| name.to_string())
    }

    #[allow(dead_code)]
    pub(crate) fn canonical_name_for_module_alias_or_item_lossy(&self, name: &str) -> String {
        crate::lower::resolution::LowerResolutionContext::new(self)
            .canonical_name_for_module_alias_or_item(name)
            .unwrap_or_else(|| name.to_string())
    }

    #[allow(dead_code)]
    pub(crate) fn function_by_def_id(&self, id: DefId) -> Option<&HirFunction> {
        self.items.function(id)
    }

    #[allow(dead_code)]
    pub(crate) fn extern_by_def_id(&self, id: DefId) -> Option<&HirExtern> {
        self.items.extern_def(id)
    }

    #[allow(dead_code)]
    pub(crate) fn top_level_var_target_by_def_id(&self, id: DefId) -> Option<HirVarTarget> {
        crate::lower::resolution::LowerResolutionContext::new(self).top_level_var_target_by_id(id)
    }

    pub(crate) fn generic_substitution_for_owner(
        _owner: DefId,
        generic_params: &[crate::types::GenericParamDecl],
        concrete_args: &[Type],
    ) -> HashMap<GenericParamId, Type> {
        generic_params
            .iter()
            .zip(concrete_args.iter())
            .map(|(param, concrete)| (param.id, concrete.clone()))
            .collect()
    }

    pub(crate) fn generic_display_name(&self, param: GenericParamId) -> Option<&str> {
        if self.current_generic_owner() == Some(param.owner) {
            return self
                .current_generic_params()
                .get(param.index as usize)
                .map(String::as_str);
        }

        None
    }

    pub(crate) fn current_generic_type_for_name(
        &mut self,
        name: &str,
        _span: crate::lexer::Span,
    ) -> Option<Type> {
        if let Some((_, binding)) = self.opened_witnesses.iter().rev().find(|(binder, _)| binder == name) {
            return Some(Type::Witness(binding.witness));
        }
        if let Some(param) = self
            .body_context
            .as_ref()
            .and_then(|context| context.generic_param_id(name))
        {
            return Some(Type::Generic(param));
        }
        let owner = self.current_generic_owner()?;
        let params = self.current_generic_params_mut()?;
        let index = params.iter().position(|param| param == name).or_else(|| {
            let mut chars = name.chars();
            let is_implicit_generic = matches!(chars.next(), Some(ch) if ch.is_ascii_uppercase())
                && chars.next().is_none();
            if is_implicit_generic {
                params.push(name.to_string());
                Some(params.len() - 1)
            } else {
                None
            }
        })?;

        Some(Type::Generic(crate::types::GenericParamId {
            owner,
            index: index as u32,
        }))
    }

    pub(crate) fn with_generic_context<T>(
        &mut self,
        owner: DefId,
        params: Vec<String>,
        f: impl FnOnce(&mut Self) -> T,
    ) -> (T, Vec<String>) {
        if let Some(context) = self.body_context.as_mut() {
            let previous = context.replace_generic_context(Some(owner), params);
            let result = f(self);
            let context = self
                .body_context
                .as_mut()
                .expect("body context should remain active");
            let (previous_owner, previous_params) = previous;
            let (_, params) = context.replace_generic_context(previous_owner, previous_params);
            return (result, params);
        }

        let previous = self
            .generic_context
            .replace(GenericLoweringContext::new(owner, params));

        let result = f(self);
        let params = self
            .generic_context
            .take()
            .expect("generic context should remain active")
            .params()
            .to_vec();
        self.generic_context = previous;

        (result, params)
    }

    #[allow(dead_code)]
    pub(crate) fn trait_by_name(&self, name: &str) -> Option<&HirTrait> {
        let id = crate::lower::resolution::LowerResolutionContext::new(self)
            .resolve_trait_type(name)?
            .id;
        self.trait_by_id(id)
    }

    pub(crate) fn trait_by_id(&self, id: DefId) -> Option<&HirTrait> {
        self.items.trait_def(id)
    }

    pub(crate) fn with_unsafe<F, T>(&mut self, f: F) -> T
    where
        F: FnOnce(&mut Self) -> T,
    {
        let Some(context) = self.body_context.as_mut() else {
            return f(self);
        };
        let previous = context.replace_in_unsafe(true);
        let result = f(self);
        self.body_context
            .as_mut()
            .expect("body context should remain active")
            .replace_in_unsafe(previous);
        result
    }

    pub(crate) fn with_current_struct_impl<F, T>(&mut self, owner: Option<String>, f: F) -> T
    where
        F: FnOnce(&mut Self) -> T,
    {
        let prev = self.current_struct_impl.clone();
        self.current_struct_impl = owner;
        let result = f(self);
        self.current_struct_impl = prev;
        result
    }
    pub(crate) fn lower_struct_field_type(
        &mut self,
        struct_name: &str,
        type_args: &[Type],
        field_name: &str,
        span: &Span,
    ) -> Option<Type> {
        let hir_struct = crate::lower::resolution::LowerResolutionContext::new(self)
            .resolve_struct_type(struct_name)?;
        let field = hir_struct
            .fields
            .iter()
            .find(|field| field.name == field_name)?;

        let current_impl_matches = self.current_struct_impl.as_deref() == Some(struct_name)
            || self.current_struct_impl.as_ref().is_some_and(|name| {
                crate::lower::resolution::LowerResolutionContext::new(self)
                    .resolve_item_id(name)
                    .is_some_and(|id| id == hir_struct.id)
            });

        if !field.public && !current_impl_matches {
            self.diagnostics.push_with_span(
                format!(
                    "Field '{}' of struct '{}' is private",
                    field_name,
                    struct_name.rsplit("::").next().unwrap_or(struct_name)
                ),
                span.clone(),
            );
            return None;
        }

        let subst = Self::generic_substitution_for_owner(
            hir_struct.id,
            &hir_struct.generic_params,
            type_args,
        );

        Some(field.ty.substitute_generics(&subst))
    }

    #[allow(dead_code)]
    pub(crate) fn canonical_owner_path(&self, name: &str) -> String {
        crate::lower::resolution::LowerResolutionContext::new(self)
            .try_canonical_owner_path(name)
            .unwrap_or_else(|| panic!("missing canonical owner path for impl type: {}", name))
    }

    pub(crate) fn try_canonical_owner_path(&self, name: &str) -> Option<String> {
        crate::lower::resolution::LowerResolutionContext::new(self).try_canonical_owner_path(name)
    }

    pub(crate) fn handle_optional_selection<T>(
        &mut self,
        result: Result<T, crate::selection::SelectionDiagnostic>,
        span: crate::lexer::Span,
    ) -> Option<T> {
        match result {
            Ok(selected) => Some(selected),
            Err(
                crate::selection::SelectionDiagnostic::NoImplementation { .. }
                | crate::selection::SelectionDiagnostic::ReceiverMismatch { .. },
            ) => None,
            Err(error) => {
                let message = self.display_selection_error(&error);
                self.diagnostics.push_selection_with_span(message, span);
                None
            }
        }
    }
}

impl Default for Lowerer {
    fn default() -> Self {
        Self::new()
    }
}

impl Lowerer {
    /// Construct a Lowerer from collected declarations (output of collect stage).
    /// Builds scope from the declarations so body lowering can proceed.
    pub fn from_declarations(
        decls: crate::collect::Declarations,
    ) -> Result<Self, Vec<ResolveError>> {
        use crate::types::Type;

        let crate::collect::Declarations {
            indexing_ids,
            item_index,
            resolver,
            current_def_ids,
            items,
            function_type_vars,
            infix_precedence,
            loaded_module_paths: _,
            type_vars,
            inject_prelude,
            loaded_prelude_export_ids,
            module_file_cache: _,
            source_modules,
            dependency_root_export_ids: _,
            source_map,
            language_items,
        } = decls;

        let lower_items = LowerItems::from_declarations(items)?;

        let mut scope = scope::Scope::new();

        // Populate scope from canonical resolver names, then fetch the exact function ID.
        let mut function_names: Vec<_> = resolver
            .item_names_by_id
            .iter()
            .map(|(id, name)| (name, id))
            .collect();
        function_names.sort_by_key(|(name, _)| *name);
        for (name, id) in function_names {
            let Some(func) = lower_items.function(*id) else {
                continue;
            };
            let param_types: Vec<Type> = func.params.iter().map(|p| p.ty.clone()).collect();
            let func_type = Type::function_with_safety(
                param_types,
                func.ret_type.clone(),
                crate::types::FunctionSafety::from_is_unsafe(func.is_unsafe),
            );
            scope.define_top_level(name.clone(), func_type, false);
        }
        let mut function_ids: Vec<_> = lower_items.functions().map(|(id, _)| id).collect();
        function_ids.sort();
        if let Some(id) = function_ids
            .into_iter()
            .find(|id| !resolver.item_names_by_id.contains_key(id))
        {
            return Err(vec![ResolveError::non_source(format!(
                "missing canonical function declaration name for DefId {id:?}"
            ))]);
        }
        // Note: struct names are NOT put in scope as values.
        // Use Type::method syntax (double-colon) for associated functions.
        // Populate scope from canonical resolver names, then fetch the exact enum ID.
        let mut enum_names: Vec<_> = lower_items.enumerations().map(|(id, _)| id).collect();
        enum_names.sort();
        for id in enum_names {
            let Some(name) = resolver.item_names_by_id.get(&id) else {
                return Err(vec![ResolveError::non_source(format!(
                    "missing canonical enum declaration name for DefId {id:?}"
                ))]);
            };
            let enum_def = lower_items
                .enumeration(id)
                .expect("enum ID collected from enum declaration map");
            scope.define_top_level(
                name.clone(),
                Type::Enum {
                    id: enum_def.id,
                    args: vec![],
                },
                false,
            );
        }
        // Populate scope from import aliases so short names (e.g. `add` for `utils::add`)
        // are visible during body lowering.
        for (short_name, def_id) in &resolver.import_aliases {
            if let Some(func) = lower_items.function(*def_id) {
                let param_types: Vec<Type> = func.params.iter().map(|p| p.ty.clone()).collect();
                let func_type = Type::function_with_safety(
                    param_types,
                    func.ret_type.clone(),
                    crate::types::FunctionSafety::from_is_unsafe(func.is_unsafe),
                );
                scope.define_alias(short_name.clone(), func_type, false);
            } else if let Some(ext) = lower_items.extern_def(*def_id) {
                let func_type = Type::function_with_safety(
                    ext.params.clone(),
                    ext.ret.clone(),
                    crate::types::FunctionSafety::from_is_unsafe(ext.is_unsafe),
                );
                scope.define_alias(short_name.clone(), func_type, false);
            }
        }

        // Populate scope from resolver canonical names and exact extern IDs. Short names come
        // from imports or module-local alias injection during body lowering.
        let mut extern_names: Vec<_> = resolver
            .item_names_by_id
            .iter()
            .filter_map(|(id, name)| {
                lower_items
                    .extern_def(*id)
                    .map(|extern_def| (name, extern_def))
            })
            .collect();
        extern_names.sort_by_key(|(name, _)| *name);
        for (name, ext) in extern_names {
            let func_type = Type::function_with_safety(
                ext.params.clone(),
                ext.ret.clone(),
                crate::types::FunctionSafety::from_is_unsafe(ext.is_unsafe),
            );
            scope.define_top_level(name.clone(), func_type, false);
        }
        let mut extern_ids: Vec<_> = lower_items.externs().map(|(id, _)| id).collect();
        extern_ids.sort();
        if let Some(id) = extern_ids
            .into_iter()
            .find(|id| !resolver.item_names_by_id.contains_key(id))
        {
            return Err(vec![ResolveError::non_source(format!(
                "missing canonical extern declaration name for DefId {id:?}"
            ))]);
        }

        let mut inference_engine =
            crate::infer::InferenceEngine::with_next_type_var(type_vars.next_raw());
        inference_engine.seed_var_spans(type_vars.var_spans().clone());
        let services = LowererServices {
            engine: LowerInferenceService::new(inference_engine),
            scope: LowerScopeService::new(scope),
            items: LowerItemService::new(lower_items),
            diagnostics: LowerDiagnosticService::new(
                crate::lower::diagnostics::LowerDiagnosticSink::new(),
            ),
            modules: LowerModuleService::from_source_modules(source_modules),
            prelude: LowerPreludeService::new(crate::lower::prelude::PreludeImports::with_exports(
                inject_prelude,
                loaded_prelude_export_ids,
            )),
            resolver: LowerResolverService::new(resolver),
            dependency_resolvers: LowerDependencyResolverService::new(HashMap::new()),
            constraint_store: LowerConstraintService::new(
                crate::infer::constraints::ConstraintStore::new(),
            ),
        };

        let mut lowerer = Self::from_services(services);
        lowerer.function_type_vars = function_type_vars;
        lowerer.infix_precedence = infix_precedence;
        lowerer.current_def_ids = current_def_ids;
        lowerer.root_crate_id = indexing_ids.root_crate_id();
        lowerer.local_def_ids = indexing_ids.into_local_def_ids();
        lowerer.item_index = item_index;
        lowerer.source_map = source_map;
        lowerer.language_items = language_items;
        lowerer.refresh_inference_normalization_env();
        lowerer.refresh_type_display_context();
        Ok(lowerer)
    }
}

pub fn seg_name(seg: &ast::IdentOrType) -> Option<String> {
    match seg {
        ast::IdentOrType::Ident(ident) => Some(ident.name.clone()),
        ast::IdentOrType::Type(ty) => Some(ty.type_name()),
    }
}

pub fn path_names(path: &[ast::IdentOrType]) -> Vec<String> {
    path.iter()
        .filter_map(seg_name)
        .flat_map(|name| name.split("::").map(str::to_string).collect::<Vec<_>>())
        .collect()
}

pub fn collect_exports(module: &ast::Module) -> HashMap<String, Option<String>> {
    let mut exports = HashMap::new();
    for top_level in &module.top_levels {
        match top_level {
            ast::TopLevel::Export(path) => {
                // Re-export (multi-segment) or legacy standalone export (single-segment)
                let names = match path {
                    ast::Path::Ident(ip) => path_names(&ip.path),
                    ast::Path::Type(tp) => path_names(&tp.path),
                };
                if let Some(short_name) = names.last().cloned() {
                    let source = if names.len() > 1 {
                        Some(names.join("::"))
                    } else {
                        None
                    };
                    exports.insert(short_name, source);
                }
            }
            // Inline exports via `< name = ...`, `< struct ...`, etc.
            ast::TopLevel::FunctionDecl(fd) if fd.exported => {
                exports.insert(fd.name.name.clone(), None);
            }
            ast::TopLevel::Extern(sig) if sig.exported => {
                exports.insert(sig.name.name.clone(), None);
            }
            ast::TopLevel::StructDecl(sd) if sd.exported => {
                exports.insert(sd.name.name.clone(), None);
            }
            ast::TopLevel::EnumDecl(ed) if ed.exported => {
                exports.insert(ed.name.name.clone(), None);
            }
            ast::TopLevel::TraitDecl(td) if td.exported => {
                exports.insert(td.name.name.clone(), None);
            }
            ast::TopLevel::Mod(ident, true) => {
                exports.insert(ident.name.clone(), None);
            }
            ast::TopLevel::GlobExport(module_path) => {
                // Store a sentinel: key = "module::path::*", source = Some("module::path::*")
                // Callers that have file-loading context will expand this into individual entries.
                let sentinel = format!("{}::*", module_path.join("::"));
                exports.insert(sentinel.clone(), Some(sentinel));
            }
            _ => {}
        }
    }
    exports
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ast::{Ident, IdentOrType, IdentifierPath, Program};
    use crate::collect::item_index::{IndexingIds, ItemIndex};
    use crate::collect::{DeclarationItems, DeclarationTypeVars, Declarations};
    use crate::crate_artifact::ArtifactExport;
    use crate::crate_system::CrateContext;
    use crate::ids::{DefId, LocalDefId, ModuleId};

    fn ident(name: &str) -> Ident {
        Ident {
            name: name.to_string(),
            span: crate::lexer::Span::test(),
        }
    }

    fn test_function(id: DefId, name: &str, stmts: Vec<HirStmt>) -> HirFunction {
        HirFunction {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: Vec::new(),
            ret_type: Type::I64,
            body: HirBlock {
                stmts,
                ty: Type::I64,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        }
    }

    fn declaration_items(
        functions: HashMap<DefId, HirFunction>,
        externs: HashMap<DefId, HirExtern>,
    ) -> DeclarationItems {
        DeclarationItems::from_id_maps(
            functions,
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            externs,
        )
        .unwrap()
    }

    fn declarations_with_items(resolver: ResolverTables, items: DeclarationItems) -> Declarations {
        Declarations {
            indexing_ids: IndexingIds::new_root(),
            item_index: ItemIndex::new(),
            resolver,
            current_def_ids: BTreeSet::new(),
            items,
            function_type_vars: HashMap::new(),
            infix_precedence: HashMap::new(),
            loaded_module_paths: Vec::new(),
            type_vars: DeclarationTypeVars::new(),
            inject_prelude: false,
            loaded_prelude_export_ids: HashMap::new(),
            module_file_cache: HashMap::new(),
            source_modules: crate::source_loader::SourceModuleSet::default(),
            dependency_root_export_ids: HashMap::new(),
            source_map: Default::default(),
            language_items: Default::default(),
        }
    }

    #[test]
    fn lowerer_from_declarations_rejects_missing_canonical_item_name() {
        let function_id = DefId::new(CrateId(0), LocalDefId(1));
        let decls = declarations_with_items(
            ResolverTables::default(),
            declaration_items(
                HashMap::from([(
                    function_id,
                    test_function(function_id, "answer", Vec::new()),
                )]),
                HashMap::new(),
            ),
        );

        let errors = Lowerer::from_declarations(decls)
            .err()
            .expect("missing canonical names should be reported");

        assert!(errors[0]
            .message
            .contains("missing canonical function declaration name"));
    }

    #[test]
    fn lowerer_from_declarations_reports_smallest_missing_canonical_id_regardless_of_insertion_order(
    ) {
        let first = DefId::new(CrateId(0), LocalDefId(1));
        let second = DefId::new(CrateId(0), LocalDefId(2));

        for ids in [[second, first], [first, second]] {
            let functions = ids
                .into_iter()
                .map(|id| (id, test_function(id, "missing", Vec::new())))
                .collect();
            let errors = Lowerer::from_declarations(declarations_with_items(
                ResolverTables::default(),
                declaration_items(functions, HashMap::new()),
            ))
            .err()
            .expect("missing canonical function names should be reported");

            assert_eq!(
                errors[0].message,
                format!("missing canonical function declaration name for DefId {first:?}")
            );
        }

        for ids in [[second, first], [first, second]] {
            let externs = ids.into_iter().map(|id| (id, test_extern(id))).collect();
            let errors = Lowerer::from_declarations(declarations_with_items(
                ResolverTables::default(),
                declaration_items(HashMap::new(), externs),
            ))
            .err()
            .expect("missing canonical extern names should be reported");

            assert_eq!(
                errors[0].message,
                format!("missing canonical extern declaration name for DefId {first:?}")
            );
        }
    }

    fn test_impl(id: DefId) -> HirImpl {
        HirImpl {
            id,
            owner: crate::hir::HirImplOwner::Named(format!("Type{}", id.local.0)),
            type_name: format!("Type{}", id.local.0),
            type_generics: vec![],
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Unit),
            trait_name: None,
            trait_id: None,
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: HashMap::new().into(),
            methods: HashMap::new(),
        }
    }

    fn test_extern(id: DefId) -> HirExtern {
        HirExtern {
            id,
            name: format!("extern{}", id.local.0),
            params: vec![],
            ret: Type::Unit,
            variadic: false,
            is_unsafe: false,
        }
    }

    #[test]
    fn lowerer_from_declarations_orders_impls_and_externs_by_def_id() {
        let first = DefId::new(CrateId(0), LocalDefId(1));
        let second = DefId::new(CrateId(0), LocalDefId(2));
        let mut resolver = ResolverTables::default();
        resolver
            .item_names_by_id
            .insert(first, "extern1".to_string());
        resolver
            .item_names_by_id
            .insert(second, "extern2".to_string());
        let items = DeclarationItems::from_id_maps(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(second, test_impl(second)), (first, test_impl(first))]),
            HashMap::from([(second, test_extern(second)), (first, test_extern(first))]),
        )
        .unwrap();

        let lowerer = Lowerer::from_declarations(declarations_with_items(resolver, items)).unwrap();

        assert_eq!(
            lowerer
                .items
                .impl_defs_in_order()
                .map(|(id, _)| id)
                .collect::<Vec<_>>(),
            vec![first, second]
        );
        assert_eq!(lowerer.items.extern_def(first).unwrap().id, first);
        assert_eq!(lowerer.items.extern_def(second).unwrap().id, second);
    }

    fn test_trait(id: DefId, name: &str) -> HirTrait {
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        }
    }

    fn int_return(value: i64) -> HirStmt {
        HirStmt::Return(Some(HirExpr {
            kind: HirExprKind::IntLiteral(value),
            ty: Type::I64,
            span: crate::lexer::Span::test(),
        }))
    }

    fn identifier_path(names: &[&str]) -> IdentifierPath {
        IdentifierPath {
            path: names
                .iter()
                .map(|name| IdentOrType::Ident(ident(name)))
                .collect(),
        }
    }

    #[test]
    fn prelude_imports_service_owns_enabled_state_and_captured_exports() {
        let print_id = DefId::new(CrateId(7), LocalDefId(3));
        let mut prelude = crate::lower::prelude::PreludeImports::new(false);

        assert!(!prelude.is_enabled());

        prelude.capture_loaded_prelude_exports(
            "stdlib",
            &std::collections::BTreeMap::from([(
                "print".to_string(),
                ArtifactExport {
                    source: "stdlib::prelude::print".to_string(),
                    id: print_id,
                },
            )]),
        );

        assert_eq!(
            prelude.export_source("print"),
            Some("stdlib::prelude::print")
        );
        assert!(!Lowerer::with_options(false).prelude.is_enabled());
    }

    #[test]
    fn lowerer_resolves_current_aliases_and_premerged_dependency_paths_through_facade() {
        let current_id = DefId::new(CrateId(0), LocalDefId(1));
        let dependency_item_id = DefId::new(CrateId(7), LocalDefId(2));
        let dependency_export_id = DefId::new(CrateId(7), LocalDefId(3));
        let mut lowerer = Lowerer::new_for_test();
        lowerer.resolver.insert_import_alias_with_name(
            "local_alias".to_string(),
            "demo::answer".to_string(),
            current_id,
        );

        let mut dependency = ResolverTables::default();
        dependency
            .item_paths
            .insert("dep::item".to_string(), dependency_item_id);
        dependency
            .item_names_by_id
            .insert(dependency_item_id, "dep::item".to_string());
        dependency.insert_export_alias_with_name(
            "dep::public".to_string(),
            "dep::internal::answer".to_string(),
            dependency_export_id,
        );
        lowerer.resolver.merge_global_inputs(&dependency);
        lowerer
            .dependency_resolvers
            .insert("dep".to_string(), dependency);

        assert_eq!(lowerer.resolve_item_def_id("local_alias"), Some(current_id));
        assert_eq!(
            lowerer.resolve_item_def_id("dep::item"),
            Some(dependency_item_id)
        );
        assert_eq!(
            lowerer
                .canonical_name_for_alias_or_item("dep::item")
                .as_deref(),
            Some("dep::item")
        );
        assert_eq!(
            lowerer.resolve_item_def_id("dep::public"),
            Some(dependency_export_id)
        );
        assert_eq!(
            lowerer
                .canonical_name_for_alias_or_item("dep::public")
                .as_deref(),
            Some("dep::internal::answer")
        );
    }

    #[test]
    fn trait_by_name_prefers_resolver_alias_id() {
        let canonical_id = DefId::new(CrateId(0), LocalDefId(30));
        let collision_id = DefId::new(CrateId(0), LocalDefId(31));
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .items
            .insert_trait_def(test_trait(canonical_id, "dep::Show"));
        lowerer
            .items
            .insert_trait_def(test_trait(collision_id, "other::Show"));
        lowerer.resolver.insert_import_alias_with_name(
            "Show".to_string(),
            "dep::Show".to_string(),
            canonical_id,
        );

        let trait_def = lowerer.trait_by_name("Show").unwrap();

        assert_eq!(trait_def.id, canonical_id);
    }

    #[test]
    fn lower_uses_canonical_current_crate_import_aliases() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_lower_canonical_import_alias_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let root_path = temp_dir.join("main.rk");
        let io_path = temp_dir.join("io.rk");
        std::fs::write(
            &io_path,
            "writer_name: I64\nwriter_name = -> 0\n< writer_name\n",
        )
        .unwrap();
        std::fs::write(&root_path, "mod io\n> io::writer_name\n").unwrap();

        let config = crate::Config {
            entry_file: root_path.clone(),
            no_std: true,
            no_prelude: true,
            current_crate_name: Some("test".to_string()),
            ..crate::Config::default()
        };
        let mut db = crate::source_loader::SourceDatabase::new();
        let graph = db.load_entry(root_path, &config).unwrap();
        let program = Program {
            module: graph.root_module().clone(),
        };

        let decls = crate::collect::collect_with_source_graph(
            &program,
            &graph,
            &CrateContext::new(),
            false,
            Some("test"),
        )
        .expect("collect should resolve canonical import aliases");

        let lowerer = Lowerer::from_declarations(decls).unwrap();

        assert_eq!(
            lowerer
                .canonical_name_for_alias_or_item("writer_name")
                .as_deref(),
            Some("test::io::writer_name")
        );
        assert!(lowerer.scope.lookup("writer_name").is_some());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn lower_from_declarations_uses_preloaded_module_cache() {
        let temp_dir =
            std::env::temp_dir().join(format!("rock_lower_source_graph_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let entry = temp_dir.join("main.rk");
        std::fs::write(&entry, "mod util\n> util::answer\nmain = -> answer!\n").unwrap();
        std::fs::write(temp_dir.join("util.rk"), "answer = -> 42\n< answer\n").unwrap();

        let config = crate::Config {
            entry_file: entry.clone(),
            no_std: true,
            no_prelude: true,
            current_crate_name: Some("demo".to_string()),
            ..crate::Config::default()
        };
        let mut db = crate::source_loader::SourceDatabase::new();
        let graph = db.load_entry(entry.clone(), &config).unwrap();
        let program = crate::ast::Program {
            module: graph.root_module().clone(),
        };
        let decls = crate::collect::collect_with_source_graph(
            &program,
            &graph,
            &crate::crate_system::CrateContext::new(),
            false,
            Some("demo"),
        )
        .unwrap();
        std::fs::remove_file(temp_dir.join("util.rk")).unwrap();

        let lowered = crate::lower::program::lower_from_declarations(
            &program,
            decls,
            &crate::crate_system::CrateContext::new(),
            Some("demo"),
        );

        assert!(lowered.is_ok(), "lowering should not re-read util.rk");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn lower_from_declarations_uses_loaded_path_for_directory_module() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_lower_source_graph_dir_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(temp_dir.join("util")).unwrap();
        let entry = temp_dir.join("main.rk");
        std::fs::write(&entry, "mod util\n> util::answer\nmain = -> answer!\n").unwrap();
        std::fs::write(
            temp_dir.join("util").join("mod.rk"),
            "answer = -> 42\n< answer\n",
        )
        .unwrap();

        let config = crate::Config {
            entry_file: entry.clone(),
            no_std: true,
            no_prelude: true,
            current_crate_name: Some("demo".to_string()),
            ..crate::Config::default()
        };
        let mut db = crate::source_loader::SourceDatabase::new();
        let graph = db.load_entry(entry.clone(), &config).unwrap();
        let program = crate::ast::Program {
            module: graph.root_module().clone(),
        };
        let decls = crate::collect::collect_with_source_graph(
            &program,
            &graph,
            &crate::crate_system::CrateContext::new(),
            false,
            Some("demo"),
        )
        .unwrap();
        std::fs::remove_dir_all(temp_dir.join("util")).unwrap();

        let lowered = crate::lower::program::lower_from_declarations(
            &program,
            decls,
            &crate::crate_system::CrateContext::new(),
            Some("demo"),
        )
        .expect("lowering should use cached directory module");

        assert!(
            lowered.functions[&lowered.resolver.item_paths["demo::util::answer"]]
                .body
                .stmts
                .len()
                > 0,
            "directory module body should be lowered from cache"
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn lower_from_declarations_accepts_graph_only_current_crate_prefixed_nested_inline_module_path()
    {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_lower_current_crate_nested_inline_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let entry = temp_dir.join("main.rk");
        let math_path = temp_dir.join("math.rk");
        let io_path = temp_dir.join("io.rk");
        std::fs::write(&entry, "mod math\nmain = -> 0\n").unwrap();
        std::fs::write(&math_path, "mod io\n").unwrap();
        std::fs::write(&io_path, "answer: I64\nanswer = -> 42\n< answer\n").unwrap();

        let config = crate::Config {
            entry_file: entry.clone(),
            no_std: true,
            no_prelude: true,
            current_crate_name: Some("test".to_string()),
            ..crate::Config::default()
        };
        let mut db = crate::source_loader::SourceDatabase::new();
        let graph = db.load_entry(entry.clone(), &config).unwrap();
        assert!(graph
            .loaded_module_paths()
            .iter()
            .any(|(name, path)| name == "test::math::io" && path == &io_path));

        let program = Program {
            module: crate::ast::Module {
                name: None,
                top_levels: vec![crate::ast::TopLevel::Module(crate::ast::ModuleDecl(
                    crate::ast::Module {
                        name: Some(ident("math")),
                        top_levels: vec![crate::ast::TopLevel::Mod(ident("io"), false)],
                        is_inline: true,
                        filepath: None,
                    },
                ))],
                is_inline: false,
                filepath: Some(entry),
            },
        };
        let mut decls = crate::collect::collect_with_source_graph(
            &program,
            &graph,
            &crate::crate_system::CrateContext::new(),
            false,
            Some("test"),
        )
        .expect("collect should resolve nested inline source-backed module");
        decls.loaded_module_paths = graph.loaded_module_paths();
        assert!(!decls
            .loaded_module_paths
            .iter()
            .any(|(name, _)| name == "math::io"));

        let lowered = crate::lower::program::lower_from_declarations(
            &program,
            decls,
            &crate::crate_system::CrateContext::new(),
            Some("test"),
        )
        .expect("lowering should use the current-crate-prefixed source graph path");

        assert!(
            lowered.functions[&lowered.resolver.item_paths["math::io::answer"]]
                .body
                .stmts
                .len()
                > 0,
            "nested source-backed module body should be lowered"
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn lower_loaded_module_bodies_preserves_paths_for_directory_glob_imports() {
        let temp_dir =
            std::env::temp_dir().join(format!("rock_lower_loaded_dir_glob_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(temp_dir.join("helper")).unwrap();
        let entry = temp_dir.join("main.rk");
        std::fs::write(&entry, "mod parent\nmain = -> parent::use_answer!\n").unwrap();
        std::fs::write(
            temp_dir.join("parent.rk"),
            "< mod helper\n> helper::*\nuse_answer: I64\nuse_answer = -> answer!\n< use_answer\n",
        )
        .unwrap();
        std::fs::write(
            temp_dir.join("helper").join("mod.rk"),
            "answer: I64\nanswer = -> 42\n< answer\n",
        )
        .unwrap();

        let config = crate::Config {
            entry_file: entry.clone(),
            no_std: true,
            no_prelude: true,
            current_crate_name: Some("demo".to_string()),
            ..crate::Config::default()
        };
        let mut db = crate::source_loader::SourceDatabase::new();
        let graph = db.load_entry(entry.clone(), &config).unwrap();
        let program = crate::ast::Program {
            module: graph.root_module().clone(),
        };
        let decls = crate::collect::collect_with_source_graph(
            &program,
            &graph,
            &crate::crate_system::CrateContext::new(),
            false,
            Some("demo"),
        )
        .unwrap();
        assert!(
            decls.items.functions().values().any(|function| {
                decls
                    .resolver
                    .item_names_by_id
                    .get(&function.id)
                    .map(String::as_str)
                    == Some("demo::parent::helper::answer")
            }),
            "functions: {:?}",
            decls.resolver.item_names_by_id.values().collect::<Vec<_>>()
        );

        let lowered = crate::lower::program::lower_from_declarations(
            &program,
            decls,
            &crate::crate_system::CrateContext::new(),
            Some("demo"),
        )
        .expect("loaded module body should resolve directory-backed glob import");

        assert!(
            lowered.functions[&lowered.resolver.item_paths["demo::parent::use_answer"]]
                .body
                .stmts
                .len()
                > 0,
            "loaded module body should be lowered"
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn lower_loaded_module_trait_defaults_does_not_skip_local_lib_module() {
        let temp_dir =
            std::env::temp_dir().join(format!("rock_lower_local_lib_mod_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let entry = temp_dir.join("main.rk");
        std::fs::write(&entry, "mod lib\n> lib::Animal\nmain = -> 0\n").unwrap();
        std::fs::write(
            temp_dir.join("lib.rk"),
            "trait Animal\n    @speak = -> 4\n< Animal\n",
        )
        .unwrap();

        let config = crate::Config {
            entry_file: entry.clone(),
            no_std: true,
            no_prelude: true,
            current_crate_name: Some("demo".to_string()),
            ..crate::Config::default()
        };
        let mut db = crate::source_loader::SourceDatabase::new();
        let graph = db.load_entry(entry.clone(), &config).unwrap();
        let program = crate::ast::Program {
            module: graph.root_module().clone(),
        };
        let decls = crate::collect::collect_with_source_graph(
            &program,
            &graph,
            &crate::crate_system::CrateContext::new(),
            false,
            Some("demo"),
        )
        .unwrap();

        let lowered = crate::lower::program::lower_from_declarations(
            &program,
            decls,
            &crate::crate_system::CrateContext::new(),
            Some("demo"),
        )
        .expect("local lib module should lower trait defaults");

        assert!(
            lowered.traits[&lowered.resolver.item_paths["demo::lib::Animal"]].methods["speak"]
                .body
                .stmts
                .len()
                > 0,
            "trait default body in local lib module should be lowered"
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn lower_loaded_modules_attach_same_named_trait_defaults_to_exact_ids() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_lower_loaded_same_trait_names_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let entry = temp_dir.join("main.rk");
        std::fs::write(&entry, "mod left\nmod right\nmain = -> 0\n").unwrap();
        std::fs::write(
            temp_dir.join("left.rk"),
            "trait Shared\n    @value = -> 1\n",
        )
        .unwrap();
        std::fs::write(
            temp_dir.join("right.rk"),
            "trait Shared\n    @value = -> true\n",
        )
        .unwrap();

        let config = crate::Config {
            entry_file: entry.clone(),
            no_std: true,
            no_prelude: true,
            current_crate_name: Some("demo".to_string()),
            ..crate::Config::default()
        };
        let mut db = crate::source_loader::SourceDatabase::new();
        let graph = db.load_entry(entry.clone(), &config).unwrap();
        let program = crate::ast::Program {
            module: graph.root_module().clone(),
        };
        let crate_ctx = crate::crate_system::CrateContext::new();
        let decls = crate::collect::collect_with_source_graph(
            &program,
            &graph,
            &crate_ctx,
            false,
            Some("demo"),
        )
        .unwrap();

        let lowered = crate::lower::program::lower_from_declarations(
            &program,
            decls,
            &crate_ctx,
            Some("demo"),
        )
        .expect("loaded modules with same-named traits should lower");
        let left_trait_id = lowered.resolver.item_paths["demo::left::Shared"];
        let right_trait_id = lowered.resolver.item_paths["demo::right::Shared"];

        assert_ne!(left_trait_id, right_trait_id);
        assert_eq!(
            lowered.traits[&left_trait_id].methods["value"].body.ty,
            Type::I64
        );
        assert_eq!(
            lowered.traits[&right_trait_id].methods["value"].body.ty,
            Type::Bool
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn glob_import_targets_uses_cached_nested_source_module() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_lower_nested_glob_cache_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let entry = temp_dir.join("main.rk");
        std::fs::write(&entry, "mod foo\nmain = -> 0\n").unwrap();
        std::fs::write(temp_dir.join("foo.rk"), "mod bar\n").unwrap();
        std::fs::write(temp_dir.join("bar.rk"), "answer = -> 42\n< answer\n").unwrap();

        let config = crate::Config {
            entry_file: entry.clone(),
            no_std: true,
            no_prelude: true,
            current_crate_name: Some("demo".to_string()),
            ..crate::Config::default()
        };
        let mut db = crate::source_loader::SourceDatabase::new();
        let graph = db.load_entry(entry.clone(), &config).unwrap();
        let mut lowerer = Lowerer::new_for_test();
        lowerer.modules =
            crate::lower::services::LowerModuleService::from_source_modules(graph.source_modules());
        lowerer
            .modules
            .configure_current_crate(Some("demo"), Some(&entry));

        let targets = lowerer
            .glob_import_targets(&["foo".to_string(), "bar".to_string()])
            .expect("nested file-backed module glob should resolve from cache");

        assert_eq!(
            targets,
            vec![("answer".to_string(), "demo::foo::bar::answer".to_string())]
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn lower_from_declarations_reports_missing_source_module() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_lower_missing_source_backed_module_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let root_path = temp_dir.join("main.rk");
        let helper_path = temp_dir.join("helper.rk");
        std::fs::write(&root_path, "mod helper\nmain = -> 0\n").unwrap();
        std::fs::write(&helper_path, "answer = -> 7\n").unwrap();

        let config = crate::Config {
            entry_file: root_path.clone(),
            no_std: true,
            no_prelude: true,
            current_crate_name: Some("demo".to_string()),
            ..crate::Config::default()
        };
        let mut db = crate::source_loader::SourceDatabase::new();
        let graph = db.load_entry(root_path.clone(), &config).unwrap();
        let program = Program {
            module: graph.root_module().clone(),
        };
        let helper_span = match &program.module.top_levels[0] {
            crate::ast::TopLevel::Mod(ident, _) => ident.span.clone(),
            _ => panic!("root module should retain the helper declaration"),
        };
        let mut decls = crate::collect::collect_with_source_graph(
            &program,
            &graph,
            &crate::crate_system::CrateContext::new(),
            false,
            Some("demo"),
        )
        .expect("collection should index the root and helper modules");
        assert!(decls
            .item_index
            .child_module_id(ModuleId(0), "helper")
            .is_some());
        decls.module_file_cache.clear();
        decls.source_modules = crate::source_loader::SourceModuleSet::default();
        decls.loaded_module_paths.clear();

        let result = crate::lower::program::lower_from_declarations(
            &program,
            decls,
            &crate::crate_system::CrateContext::new(),
            Some("demo"),
        );
        let errors = match result {
            Ok(_) => panic!("missing cached module should produce a resolve error"),
            Err(errors) => errors,
        };

        let error = errors
            .iter()
            .find(|error| {
                error
                    .message
                    .contains("Module 'demo::helper' was not loaded by the source database")
            })
            .expect("missing source-backed module should report its declaration");
        let span = error
            .span()
            .expect("missing source-backed module error should be span-aware");
        assert_eq!(span.file_path, helper_span.file_path);
        assert_eq!(span.start, helper_span.start);
        assert_eq!(span.end, helper_span.end);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn lowerer_from_declarations_preserves_dependency_import_alias_ids() {
        let dependency_id = DefId::new(CrateId(7), LocalDefId(3));
        let mut resolver = ResolverTables::default();
        resolver
            .import_aliases
            .insert("answer".to_string(), dependency_id);
        resolver
            .item_names_by_id
            .insert(dependency_id, "dep::answer".to_string());

        let decls = Declarations {
            indexing_ids: IndexingIds::new_root(),
            item_index: ItemIndex::new(),
            resolver,
            current_def_ids: BTreeSet::new(),
            items: declaration_items(HashMap::new(), HashMap::new()),
            function_type_vars: HashMap::new(),
            infix_precedence: HashMap::new(),
            loaded_module_paths: Vec::new(),
            type_vars: DeclarationTypeVars::new(),
            inject_prelude: false,
            loaded_prelude_export_ids: HashMap::new(),
            module_file_cache: HashMap::new(),
            source_modules: crate::source_loader::SourceModuleSet::default(),
            dependency_root_export_ids: HashMap::new(),
            source_map: Default::default(),
            language_items: Default::default(),
        };

        let lowerer = Lowerer::from_declarations(decls).unwrap();

        assert_eq!(
            lowerer
                .canonical_name_for_alias_or_item("answer")
                .as_deref(),
            Some("dep::answer")
        );
        assert_eq!(
            lowerer.resolver.import_aliases.get("answer").copied(),
            Some(dependency_id)
        );
    }

    #[test]
    fn lowerer_from_declarations_import_alias_lowers_to_resolved_function() {
        let dependency_id = DefId::new(CrateId(7), LocalDefId(3));
        let mut resolver = ResolverTables::default();
        resolver
            .import_aliases
            .insert("answer".to_string(), dependency_id);
        resolver
            .item_names_by_id
            .insert(dependency_id, "dep::answer".to_string());

        let decls = Declarations {
            indexing_ids: IndexingIds::new_root(),
            item_index: ItemIndex::new(),
            resolver,
            current_def_ids: BTreeSet::new(),
            items: declaration_items(
                HashMap::from([(
                    dependency_id,
                    test_function(dependency_id, "dep::answer", Vec::new()),
                )]),
                HashMap::new(),
            ),
            function_type_vars: HashMap::new(),
            infix_precedence: HashMap::new(),
            loaded_module_paths: Vec::new(),
            type_vars: DeclarationTypeVars::new(),
            inject_prelude: false,
            loaded_prelude_export_ids: HashMap::new(),
            module_file_cache: HashMap::new(),
            source_modules: crate::source_loader::SourceModuleSet::default(),
            dependency_root_export_ids: HashMap::new(),
            source_map: Default::default(),
            language_items: Default::default(),
        };
        let mut lowerer = Lowerer::from_declarations(decls).unwrap();

        let expr = lowerer.lower_identifier_path(&identifier_path(&["answer"]));

        match expr.kind {
            HirExprKind::ResolvedVar(reference) => {
                assert_eq!(reference.name, "dep::answer");
                assert_eq!(reference.target, HirVarTarget::Function(dependency_id));
            }
            other => panic!("expected resolved import alias, got {other:?}"),
        }
    }

    #[test]
    fn lowerer_from_declarations_extern_import_alias_lowers_to_resolved_extern() {
        let dependency_id = DefId::new(CrateId(7), LocalDefId(4));
        let mut resolver = ResolverTables::default();
        resolver
            .import_aliases
            .insert("puts".to_string(), dependency_id);
        resolver
            .item_names_by_id
            .insert(dependency_id, "dep::puts".to_string());

        let decls = Declarations {
            indexing_ids: IndexingIds::new_root(),
            item_index: ItemIndex::new(),
            resolver,
            current_def_ids: BTreeSet::new(),
            items: declaration_items(
                HashMap::new(),
                HashMap::from([(
                    dependency_id,
                    HirExtern {
                        id: dependency_id,
                        name: "dep::puts".to_string(),
                        params: vec![Type::I32],
                        ret: Type::I32,
                        variadic: false,
                        is_unsafe: false,
                    },
                )]),
            ),
            function_type_vars: HashMap::new(),
            infix_precedence: HashMap::new(),
            loaded_module_paths: Vec::new(),
            type_vars: DeclarationTypeVars::new(),
            inject_prelude: false,
            loaded_prelude_export_ids: HashMap::new(),
            module_file_cache: HashMap::new(),
            source_modules: crate::source_loader::SourceModuleSet::default(),
            dependency_root_export_ids: HashMap::new(),
            source_map: Default::default(),
            language_items: Default::default(),
        };
        let mut lowerer = Lowerer::from_declarations(decls).unwrap();

        let expr = lowerer.lower_identifier_path(&identifier_path(&["puts"]));

        match expr.kind {
            HirExprKind::ResolvedVar(reference) => {
                assert_eq!(reference.name, "dep::puts");
                assert_eq!(reference.target, HirVarTarget::Extern(dependency_id));
            }
            other => panic!("expected resolved extern import alias, got {other:?}"),
        }
    }

    #[test]
    fn injected_prelude_scope_only_extern_alias_lowers_to_resolved_extern() {
        let puts_id = DefId::new(CrateId(7), LocalDefId(4));
        let mut lowerer = Lowerer::new_for_test();
        lowerer.items.insert_extern(HirExtern {
            id: puts_id,
            name: "stdlib::libc::puts".to_string(),
            params: vec![Type::I32],
            ret: Type::I32,
            variadic: false,
            is_unsafe: false,
        });
        lowerer.scope.define(
            "stdlib::libc::puts".to_string(),
            Type::function(vec![Type::I32], Type::I32),
            false,
        );
        lowerer.prelude.capture_loaded_prelude_exports(
            "stdlib",
            &std::collections::BTreeMap::from([(
                "puts".to_string(),
                ArtifactExport {
                    source: "stdlib::libc::puts".to_string(),
                    id: puts_id,
                },
            )]),
        );
        let errors = lowerer.prelude.inject_loaded_prelude(
            &mut lowerer.scope,
            &mut lowerer.items,
            &mut lowerer.resolver,
        );
        assert!(errors.is_empty(), "unexpected prelude errors: {errors:?}");

        let expr = lowerer.lower_identifier_path(&identifier_path(&["puts"]));

        match expr.kind {
            HirExprKind::ResolvedVar(reference) => {
                assert_eq!(reference.name, "stdlib::libc::puts");
                assert_eq!(reference.target, HirVarTarget::Extern(puts_id));
            }
            other => panic!("expected resolved prelude extern alias, got {other:?}"),
        }
    }

    #[test]
    fn glob_import_targets_uses_dependency_resolver_exports() {
        let dependency_id = DefId::new(CrateId(7), LocalDefId(3));
        let mut dep_resolver = ResolverTables::default();
        dep_resolver.insert_export_alias_with_name(
            "answer".to_string(),
            "dep::answer".to_string(),
            dependency_id,
        );
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .dependency_resolvers
            .insert("dep".to_string(), dep_resolver);

        let targets = lowerer
            .glob_import_targets(&["dep".to_string()])
            .expect("dependency resolver glob should resolve from export aliases");

        assert_eq!(
            targets,
            vec![("answer".to_string(), "dep::answer".to_string())]
        );
    }

    #[test]
    fn handle_glob_import_records_dependency_resolver_export_ids() {
        let dependency_id = DefId::new(CrateId(7), LocalDefId(3));
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .items
            .insert_function(test_function(dependency_id, "dep::answer", Vec::new()));
        let mut dep_resolver = ResolverTables::default();
        dep_resolver.insert_export_alias_with_name(
            "answer".to_string(),
            "dep::answer".to_string(),
            dependency_id,
        );
        lowerer
            .dependency_resolvers
            .insert("dep".to_string(), dep_resolver);

        lowerer.handle_glob_import(&["dep".to_string()]);

        assert_eq!(
            lowerer.resolver.import_aliases.get("answer").copied(),
            Some(dependency_id)
        );
        assert_eq!(
            lowerer
                .resolver
                .item_names_by_id
                .get(&dependency_id)
                .map(String::as_str),
            Some("dep::answer")
        );
    }

    #[test]
    fn injected_prelude_function_alias_remains_id_backed_view() {
        let prelude_id = DefId::new(CrateId(7), LocalDefId(3));
        let mut lowerer = Lowerer::new_for_test();
        lowerer.prelude.capture_loaded_prelude_exports(
            "stdlib",
            &std::collections::BTreeMap::from([(
                "answer".to_string(),
                ArtifactExport {
                    source: "stdlib::answer".to_string(),
                    id: prelude_id,
                },
            )]),
        );
        lowerer.items.insert_function(test_function(
            prelude_id,
            "stdlib::answer",
            vec![int_return(42)],
        ));

        let errors = lowerer.prelude.inject_loaded_prelude(
            &mut lowerer.scope,
            &mut lowerer.items,
            &mut lowerer.resolver,
        );

        assert!(errors.is_empty(), "unexpected prelude errors: {errors:?}");
        assert_eq!(
            lowerer.resolver.import_aliases.get("answer").copied(),
            Some(prelude_id)
        );
        assert_eq!(lowerer.items.functions().count(), 1);
        assert_eq!(
            lowerer.items.function(prelude_id).unwrap().body.stmts.len(),
            1
        );
    }

    #[test]
    fn lowerer_from_declarations_preserves_real_local_extern_id_zero() {
        let extern_id = DefId::new(CrateId(0), LocalDefId(0));
        let collision_id = DefId::new(CrateId(0), LocalDefId(1));
        let mut resolver = ResolverTables::default();
        resolver.item_paths.insert("puts".to_string(), collision_id);
        resolver
            .item_names_by_id
            .insert(extern_id, "demo::puts".to_string());

        let decls = Declarations {
            indexing_ids: IndexingIds::new_root(),
            item_index: ItemIndex::new(),
            resolver,
            current_def_ids: BTreeSet::from([extern_id]),
            items: declaration_items(
                HashMap::new(),
                HashMap::from([(
                    extern_id,
                    HirExtern {
                        id: extern_id,
                        name: "demo::puts".to_string(),
                        params: vec![Type::Pointer(Box::new(Type::U8))],
                        ret: Type::I32,
                        variadic: false,
                        is_unsafe: false,
                    },
                )]),
            ),
            function_type_vars: HashMap::new(),
            infix_precedence: HashMap::new(),
            loaded_module_paths: Vec::new(),
            type_vars: DeclarationTypeVars::new(),
            inject_prelude: false,
            loaded_prelude_export_ids: HashMap::new(),
            module_file_cache: HashMap::new(),
            source_modules: crate::source_loader::SourceModuleSet::default(),
            dependency_root_export_ids: HashMap::new(),
            source_map: Default::default(),
            language_items: Default::default(),
        };

        let lowerer = Lowerer::from_declarations(decls).unwrap();

        assert_eq!(lowerer.items.extern_def(extern_id).unwrap().id, extern_id);
    }

    #[test]
    fn lowerer_construction_uses_explicit_service_wrappers() {
        let services = crate::lower::services::LowererServices::new(true);
        let lowerer = Lowerer::from_services(services);

        assert!(lowerer.prelude.is_enabled());
        assert!(lowerer.diagnostics.is_empty());
        assert!(lowerer.items.functions().next().is_none());
    }

    #[test]
    fn body_lowering_context_owns_body_local_ids_and_restores_lowerer_state() {
        let mut lowerer = Lowerer::new_for_test();
        let outer_owner = DefId::new(CrateId(0), LocalDefId(99));
        lowerer.generic_context = Some(crate::lower::body_context::GenericLoweringContext::new(
            outer_owner,
            vec!["Outer".to_string()],
        ));

        let body_owner = DefId::new(CrateId(0), LocalDefId(7));
        let context = crate::lower::body_context::BodyLoweringContext::new(
            "body".to_string(),
            BodyOwner::Function(body_owner),
            Some(body_owner),
            vec!["T".to_string()],
            HirGenericBounds::new(),
            true,
        );

        let ids = lowerer.with_body_context(context, |lowerer| {
            assert_eq!(lowerer.current_function_name(), Some("body"));
            assert_eq!(
                lowerer.current_body_owner(),
                Some(BodyOwner::Function(body_owner))
            );
            assert_eq!(lowerer.current_generic_owner(), Some(body_owner));
            assert_eq!(lowerer.current_generic_params(), &["T".to_string()]);
            assert!(lowerer.is_in_unsafe());
            (lowerer.fresh_local_id(), lowerer.fresh_local_id())
        });

        assert_eq!(ids.0, HirLocalId(0));
        assert_eq!(ids.1, HirLocalId(1));
        assert_eq!(lowerer.current_generic_owner(), Some(outer_owner));
        assert_eq!(lowerer.current_generic_params(), &["Outer".to_string()]);
        assert!(!lowerer.is_in_unsafe());
        assert!(lowerer.current_function_name().is_none());
        assert!(lowerer.current_body_owner().is_none());
    }

    #[test]
    fn body_owner_mismatched_method_name_and_id_does_not_rediscover_method() {
        let mut lowerer = Lowerer::new_for_test();
        let impl_id = DefId::new(CrateId(0), LocalDefId(20));
        let first_method_id = DefId::new(CrateId(0), LocalDefId(21));
        let second_method_id = DefId::new(CrateId(0), LocalDefId(22));
        let mut first = test_function(first_method_id, "first", Vec::new());
        first.ret_type = Type::I32;
        let mut second = test_function(second_method_id, "second", Vec::new());
        second.ret_type = Type::Bool;
        let mut impl_def = test_impl(impl_id);
        impl_def.methods.insert("first".to_string(), first);
        impl_def.methods.insert("second".to_string(), second);
        lowerer.items.insert_impl(impl_def).unwrap();

        let context = BodyLoweringContext::new(
            "first".to_string(),
            BodyOwner::ImplMethod {
                impl_id,
                method_id: second_method_id,
                method_name: "first".to_string(),
            },
            Some(impl_id),
            Vec::new(),
            HirGenericBounds::new(),
            false,
        );

        let return_type =
            lowerer.with_body_context(context, |lowerer| lowerer.current_body_return_type());

        assert_eq!(return_type, None);
        assert!(!lowerer.diagnostics.is_empty());
    }
}

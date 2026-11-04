//! Lowerer context: struct definition, constructors, and helper functions

pub mod bodies;
pub(crate) mod body_context;
pub(crate) mod body_lowerer;
pub mod collect;
pub mod control_flow;
pub mod crates;
pub(crate) mod diagnostics;
mod display;
pub mod error;
mod exports;
pub mod expression;
mod from_declarations;
pub mod function;
pub(crate) mod inference_scc;
pub mod intrinsics;
pub(crate) mod items;
pub(crate) mod module_context;
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
pub use exports::{collect_exports, path_names, seg_name};
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
    pub(crate) overflow_checks: bool,
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
}

impl Lowerer {
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
            overflow_checks: true,
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

    // Staging facades retained while lower resolution call sites migrate task-by-task.
    pub(crate) fn resolve_module_alias_or_item_def_id(&self, name: &str) -> Option<DefId> {
        crate::lower::resolution::LowerResolutionContext::new(self)
            .resolve_module_alias_or_item_id(name)
    }

    pub(crate) fn canonical_name_for_def_id(&self, id: DefId) -> Option<&str> {
        crate::lower::resolution::LowerResolutionContext::new(self).canonical_name(id)
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

#[cfg(test)]
impl Lowerer {
    pub(crate) fn resolve_item_def_id(&self, name: &str) -> Option<DefId> {
        crate::lower::resolution::LowerResolutionContext::new(self).resolve_item_id(name)
    }

    pub(crate) fn canonical_name_for_alias_or_item(&self, name: &str) -> Option<String> {
        crate::lower::resolution::LowerResolutionContext::new(self).canonical_name_for_item(name)
    }
}

#[cfg(test)]
mod tests;

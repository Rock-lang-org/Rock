use std::collections::{HashMap, HashSet};

use crate::hir::{HirBlock, HirExpr, HirFunction, HirStmt};
use crate::ids::DefId;
use crate::lower::ResolveError;
use crate::selection::SelectionService;
use crate::types::{FunctionSafety, Type};

use super::type_predicates::contains_recovery_type;
use super::type_vars::collect_type_vars;
use super::{AuthorityObligationKind, AuthoritySiteId, MethodAuthorityContext};

impl MethodAuthorityContext<'_> {
    pub(super) fn display_type(&self, ty: &Type) -> String {
        self.engine.borrow().display_type(ty)
    }

    pub(super) fn display_selection_error(
        &self,
        error: &crate::selection::SelectionDiagnostic,
    ) -> String {
        error.message_with_names(
            |ty| self.display_type(ty),
            |id| self.resolver.canonical_name(id).map(str::to_string),
        )
    }

    pub(super) fn service(&self) -> SelectionService<'_> {
        SelectionService::new(
            self.traits,
            self.impls,
            self.sized_trait_id,
            None,
            &self.bounds,
        )
        .with_effective_trait_methods(self.effective_trait_methods)
    }

    pub(super) fn materialize_function(
        &mut self,
        function: &mut HirFunction,
        errors: &mut Vec<ResolveError>,
    ) {
        self.bounds = function.generic_bounds.clone();
        self.mutable_locals.clear();
        self.local_bindings.clear();
        self.next_local_id = 0;
        self.collect_local_bindings(function);
        let previous = self.unsafe_context.replace(function.is_unsafe);
        self.materialize_block(&mut function.body, errors);
        let _ = self
            .engine
            .borrow_mut()
            .unify(&function.ret_type, &function.body.ty);
        function.ret_type = self.resolved_type(&function.ret_type);
        function.body.ty = self.resolved_type(&function.body.ty);
        self.unsafe_context.set(previous);
    }

    pub(super) fn visit_authority_slot(
        &self,
        kind: AuthorityObligationKind,
        span: &crate::lexer::Span,
    ) -> bool {
        let current = AuthoritySiteId::new(kind, span);
        self.selected_site
            .as_ref()
            .is_none_or(|selected| *selected == current)
    }

    pub(super) fn materialize_direct_function_call(
        &mut self,
        result_ty: &Type,
        callee: &HirExpr,
        args: &[HirExpr],
        function_id: DefId,
    ) {
        let Some(function) = self.functions.get(&function_id).cloned() else {
            return;
        };
        if function.is_method
            || !function.generic_params.is_empty()
            || function.params.iter().any(|param| param.is_ref)
        {
            return;
        }
        let source_type = Type::function_with_safety(
            function
                .params
                .iter()
                .map(|param| self.resolved_type(&param.ty))
                .collect(),
            self.resolved_type(&function.ret_type),
            FunctionSafety::from_is_unsafe(function.is_unsafe),
        );
        if !contains_recovery_type(&source_type) {
            let _ = self.engine.borrow_mut().unify(&callee.ty, &source_type);
        }
        if let Type::Function { params, ret, .. } = self.resolved_type(&callee.ty) {
            let mut engine = self.engine.borrow_mut();
            for (arg, expected) in args.iter().zip(params.iter()) {
                let _ = engine.unify(&arg.ty, expected);
            }
            let result = if args.len() < params.len() {
                Type::function_with_safety(
                    params[args.len()..].to_vec(),
                    *ret,
                    FunctionSafety::from_is_unsafe(function.is_unsafe),
                )
            } else {
                *ret
            };
            let _ = engine.unify(result_ty, &result);
            return;
        }
        let source_params = function
            .params
            .iter()
            .map(|param| self.resolved_type(&param.ty))
            .collect::<Vec<_>>();
        let source_ret = self.resolved_type(&function.ret_type);
        let mut vars = HashSet::new();
        for ty in source_params.iter().chain(std::iter::once(&source_ret)) {
            collect_type_vars(ty, &mut vars);
        }
        let substitutions = vars
            .into_iter()
            .map(|id| {
                let kind = self.engine.borrow().kind_of_type_var(id);
                let fresh = self
                    .engine
                    .borrow_mut()
                    .fresh_type_var_at_kind(callee.span.clone(), kind);
                (id, fresh)
            })
            .collect::<HashMap<_, _>>();
        let params = source_params
            .iter()
            .map(|ty| ty.substitute(&substitutions))
            .collect::<Vec<_>>();
        let ret = source_ret.substitute(&substitutions);
        let instance = Type::function_with_safety(
            params.clone(),
            ret.clone(),
            FunctionSafety::from_is_unsafe(function.is_unsafe),
        );
        let mut engine = self.engine.borrow_mut();
        let _ = engine.unify(&callee.ty, &instance);
        for (arg, expected) in args.iter().zip(params.iter()) {
            let _ = engine.unify(&arg.ty, expected);
        }
        let result = if args.len() < params.len() {
            Type::function_with_safety(
                params[args.len()..].to_vec(),
                ret,
                FunctionSafety::from_is_unsafe(function.is_unsafe),
            )
        } else {
            ret
        };
        let _ = engine.unify(result_ty, &result);
    }

    pub(super) fn materialize_block(
        &mut self,
        block: &mut HirBlock,
        errors: &mut Vec<ResolveError>,
    ) {
        for stmt in &mut block.stmts {
            match stmt {
                HirStmt::Let { value, .. }
                | HirStmt::Expr(value)
                | HirStmt::Return(Some(value))
                | HirStmt::Break(Some(value)) => self.materialize_expr(value, errors),
                HirStmt::Return(None) | HirStmt::Break(None) | HirStmt::Continue => {}
            }
        }
    }
}

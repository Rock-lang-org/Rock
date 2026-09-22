use std::collections::HashMap;

use crate::ast;
use crate::hir::*;
use crate::ids::DefId;
use crate::lexer::Span;
use crate::lower::expression::ExprUse;
use crate::lower::intrinsics::{
    infer_intrinsic_arg_types, infer_intrinsic_return_type, is_intrinsic_name,
};
use crate::lower::Lowerer;
use crate::type_services::facts::TypeFacts;
use crate::types::{GenericParamId, TraitBound, Type};

impl Lowerer {
    fn lambda_from_expression(expr: &ast::Expression) -> Option<&ast::LambdaDecl> {
        let ast::Expression::UnaryExpr(ast::UnaryExpr::PrimaryExpr(primary)) = expr else {
            return None;
        };
        match &primary.operand {
            ast::Operand::LambdaDecl(lambda) => Some(lambda),
            ast::Operand::Expression(inner) => Self::lambda_from_expression(inner),
            _ => None,
        }
    }

    fn selected_callable_parameter_types(
        &self,
        selected: &crate::selection::SelectedMethod,
        parameter: &HirParam,
    ) -> Option<Vec<Type>> {
        let callable_ids = [
            self.language_items
                .fn_once
                .as_ref()
                .map(|items| items.trait_id),
            self.language_items
                .fn_mut
                .as_ref()
                .map(|items| items.trait_id),
            self.language_items
                .fn_trait
                .as_ref()
                .map(|items| items.trait_id),
        ];
        let is_callable_bound = |bound: &TraitBound| {
            callable_ids
                .iter()
                .flatten()
                .any(|trait_id| *trait_id == bound.trait_id)
                && bound.type_args.len() == 2
        };
        let pending_bound = selected
            .pending_impl_bounds
            .iter()
            .find(|(subject, bound)| subject == &parameter.ty && is_callable_bound(bound))
            .map(|(_, bound)| bound.clone());
        let function_bound = match &parameter.ty {
            Type::Generic(parameter_id) => selected
                .function
                .as_ref()
                .and_then(|function| function.generic_bounds.get(parameter_id))
                .and_then(|bounds| bounds.iter().find(|bound| is_callable_bound(bound)))
                .cloned(),
            Type::TypeVar(id) => self
                .engine
                .get_bounds(*id)
                .into_iter()
                .find(is_callable_bound),
            _ => None,
        };
        let bound = pending_bound.or(function_bound)?;
        let args = bound.type_args[0].substitute_generics(&selected.owner_substitution);
        Some(match args {
            Type::Unit => Vec::new(),
            Type::Tuple(params) => params,
            param => vec![param],
        })
    }

    fn lower_selected_method_argument(
        &mut self,
        argument: &ast::Expression,
        selected: &crate::selection::SelectedMethod,
        index: usize,
    ) -> HirExpr {
        let Some(lambda) = Self::lambda_from_expression(argument) else {
            return self.lower_expression(argument);
        };
        let mut expected = selected
            .substituted_params
            .get(index)
            .and_then(|parameter| self.selected_callable_parameter_types(selected, parameter));
        if let (Some(params), Some(function)) = (&mut expected, &selected.function) {
            // Method-local generics are inference variables at a call site, not
            // rigid parameters in the caller's lambda body.
            let substitution = function
                .generic_params
                .iter()
                .map(|param| {
                    (
                        param.id,
                        self.engine
                            .fresh_type_var_at_kind(lambda.span.clone(), param.kind.clone()),
                    )
                })
                .collect();
            for param in params {
                *param = param.substitute_generics(&substitution);
            }
        }
        if lambda.parameters.len() == 1 {
            if let Some(params) = &mut expected {
                if params.len() > 1 {
                    *params = vec![Type::Tuple(std::mem::take(params))];
                }
            }
        }
        self.lower_lambda_with_expected_params(lambda, expected.as_deref())
    }

    fn callable_argument_parameters(&self, expected: &Type) -> Option<Vec<Type>> {
        match self.engine.resolve(expected) {
            Type::Function { params, .. } => Some(params),
            Type::TypeVar(id) => {
                let callable_ids = [
                    self.language_items
                        .fn_once
                        .as_ref()
                        .map(|items| items.trait_id),
                    self.language_items
                        .fn_mut
                        .as_ref()
                        .map(|items| items.trait_id),
                    self.language_items
                        .fn_trait
                        .as_ref()
                        .map(|items| items.trait_id),
                ];
                let bounds = self.engine.get_bounds(id);
                let bound = bounds.iter().find(|bound| {
                    bound.type_args.len() == 2
                        && callable_ids
                            .iter()
                            .flatten()
                            .any(|id| *id == bound.trait_id)
                })?;
                Some(match self.engine.resolve(&bound.type_args[0]) {
                    Type::Unit => Vec::new(),
                    Type::Tuple(params) => params,
                    param => vec![param],
                })
            }
            _ => None,
        }
    }

    fn propagate_call_argument_bounds(&mut self, expected: &Type, actual: &Type) {
        let bounds = match expected {
            Type::TypeVar(id) => self.engine.get_bounds(*id),
            _ => Vec::new(),
        };
        let mut probe = self.engine.clone_for_probe();
        if probe.unify(expected, actual).is_err() {
            return;
        }
        self.engine.commit_probe(probe);
        let actual = self.engine.resolve(actual);
        for bound in bounds {
            if !bound.type_args.iter().any(|arg| {
                crate::type_services::visit::type_any(&self.engine.resolve(arg), |ty| {
                    matches!(ty, Type::TypeVar(_))
                })
            }) {
                continue;
            }
            let Some(args) = self
                .selection_service()
                .infer_trait_arguments(&actual, bound.trait_id)
            else {
                continue;
            };
            if args.len() != bound.type_args.len() {
                continue;
            }
            let mut probe = self.engine.clone_for_probe();
            if args
                .iter()
                .zip(&bound.type_args)
                .all(|(actual, expected)| probe.unify(expected, actual).is_ok())
            {
                self.engine.commit_probe(probe);
            }
        }
    }

    pub(crate) fn call_target_for_callee(&self, callee: &HirExpr) -> Option<HirCallTarget> {
        match &callee.kind {
            HirExprKind::ResolvedVar(reference) => match &reference.target {
                HirVarTarget::Function(id) => Some(HirCallTarget::Function(*id)),
                HirVarTarget::Extern(id) => Some(HirCallTarget::Extern(*id)),
                HirVarTarget::Instance(id) => Some(HirCallTarget::Instance(*id)),
                HirVarTarget::Local(id) => Some(HirCallTarget::Local(*id)),
            },
            HirExprKind::Var(name) if is_intrinsic_name(name) => {
                Some(HirCallTarget::Intrinsic(name.clone()))
            }
            HirExprKind::Var(_) => None,
            _ => None,
        }
    }

    pub(crate) fn report_unsafe_function_value_call_from_type(&mut self, callee: &HirExpr) {
        let resolved = self.resolve_projection_type(&self.engine.resolve(&callee.ty));
        if let Type::Function {
            safety: crate::types::FunctionSafety::Unsafe,
            ..
        } = resolved
        {
            if !self.is_in_unsafe() {
                self.diagnostics.push_selection_with_span(
                    "Call to unsafe function value requires an unsafe block".to_string(),
                    callee.span.clone(),
                );
            }
        }
    }

    fn current_function(&self, id: crate::ids::DefId) -> Option<&HirFunction> {
        self.items.function(id)
    }

    #[cfg(test)]
    fn fixed_array_len_from_type_name(type_name: &str) -> Option<usize> {
        let inner = type_name.strip_prefix('[')?.strip_suffix(']')?;
        let (_, len) = inner.split_once(';')?;
        len.trim().parse().ok()
    }

    #[cfg(test)]
    fn receiver_type_name_matches(candidate: &str, lookup: &str) -> bool {
        if candidate == lookup {
            return true;
        }

        fn borrowed_slice_elem(name: &str) -> Option<&str> {
            name.strip_prefix("&[")
                .and_then(|name| name.strip_suffix(']'))
                .filter(|name| !name.contains(';'))
        }

        fn mut_borrowed_slice_elem(name: &str) -> Option<&str> {
            name.strip_prefix("&mut [")
                .and_then(|name| name.strip_suffix(']'))
                .filter(|name| !name.contains(';'))
        }

        let is_generic_elem = |name: &str| {
            !matches!(
                name,
                "I8" | "I16"
                    | "I32"
                    | "I64"
                    | "U8"
                    | "U16"
                    | "U32"
                    | "U64"
                    | "F32"
                    | "F64"
                    | "Bool"
                    | "Char"
                    | "Str"
            )
        };

        if let (Some(candidate_elem), Some(lookup_elem)) =
            (borrowed_slice_elem(candidate), borrowed_slice_elem(lookup))
        {
            return candidate_elem == lookup_elem || is_generic_elem(candidate_elem);
        }

        if let (Some(candidate_elem), Some(lookup_elem)) = (
            mut_borrowed_slice_elem(candidate),
            mut_borrowed_slice_elem(lookup),
        ) {
            return candidate_elem == lookup_elem || is_generic_elem(candidate_elem);
        }

        match (
            Self::fixed_array_len_from_type_name(candidate),
            Self::fixed_array_len_from_type_name(lookup),
        ) {
            (Some(candidate_len), Some(lookup_len)) => candidate_len == lookup_len,
            _ => false,
        }
    }

    pub(crate) fn coerce_argument_to_expected(
        &mut self,
        mut arg: HirExpr,
        expected_ty: &Type,
    ) -> HirExpr {
        let resolved_expected = self.resolve_projection_type(expected_ty);
        let resolved_arg = self.resolve_projection_type(&self.engine.resolve(&arg.ty));

        if let (
            Type::Reference {
                mutable: expected_mutable,
                inner: expected_inner,
            },
            Type::Reference {
                mutable: arg_mutable,
                inner: arg_inner,
            },
        ) = (&resolved_expected, &resolved_arg)
        {
            if (!expected_mutable || *arg_mutable)
                && matches!(expected_inner.as_ref(), Type::Slice(_))
                && matches!(arg_inner.as_ref(), Type::Array(_, _))
            {
                if let Some(coerced) = self
                    .coerce_array_ref_to_slice_ref_with_mutability(arg.clone(), *expected_mutable)
                {
                    let _ = self.engine.unify(&coerced.ty, &resolved_expected);
                    return coerced;
                }
            }
        }

        if let Err(err) = self.engine.unify(&arg.ty, &resolved_expected) {
            self.diagnostics
                .push_type_with_span(err.render(&self.engine), arg.span.clone());
            return HirExpr {
                ty: Type::Error,
                kind: arg.kind,
                span: arg.span,
            };
        }

        self.resolve_all_types_in_expr(&mut arg);
        if let HirExprKind::Lambda {
            params,
            body,
            captures,
        } = &arg.kind
        {
            let safety = match self.resolve_projection_type(&self.engine.resolve(&arg.ty)) {
                Type::Function { safety, .. } => safety,
                _ => crate::types::FunctionSafety::Safe,
            };
            arg.ty = Self::lambda_function_type(
                params.iter().map(|param| param.ty.clone()).collect(),
                body.ty.clone(),
                safety,
                captures,
            );
        }

        arg
    }

    pub(crate) fn concrete_method_candidate(
        &mut self,
        mut recv: HirExpr,
        method_name: &str,
    ) -> Option<crate::selection::SelectedMethod> {
        fn collect_type_vars(ty: &Type, vars: &mut Vec<crate::ids::TypeVarId>) {
            crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
                if let Type::TypeVar(var) = nested {
                    if !vars.contains(var) {
                        vars.push(*var);
                    }
                }
            });
        }

        let mut receiver_vars = Vec::new();
        let resolved_receiver = self.engine.resolve(&recv.ty);
        collect_type_vars(&resolved_receiver, &mut receiver_vars);
        if let Type::TypeVar(id) = resolved_receiver {
            if self.engine.get_bounds(id).is_empty() {
                return None;
            }
        }
        for var in receiver_vars {
            if let Some(default) = self
                .constraint_store
                .literal_default_type_for_representative(var, |constrained| {
                    match self.engine.resolve(&Type::TypeVar(constrained)) {
                        Type::TypeVar(representative) => Some(representative),
                        _ => None,
                    }
                })
            {
                let _ = self.engine.unify(&Type::TypeVar(var), &default);
            }
        }
        recv.ty = self.engine.resolve(&recv.ty);
        if let Some(selected) = self.callable_method_candidate(recv.clone(), Some(method_name)) {
            return Some(selected);
        }
        let recv_span = recv.span.clone();
        let receiver_candidates = self.receiver_adjustment_candidates(recv);
        // Prefer proven adjusted candidates without discarding unresolved generic fallbacks.
        let mut deferred_candidate = None;
        for receiver_candidate in &receiver_candidates {
            let mut concrete = self.selection_service().select_concrete_method_candidates(
                std::slice::from_ref(receiver_candidate),
                method_name,
                |ty| self.resolve_projection_type(&self.engine.resolve(ty)),
            );
            let has_proven_candidate = concrete
                .iter()
                .any(|candidate| candidate.pending_impl_bounds.is_empty());
            if has_proven_candidate {
                concrete.retain(|candidate| candidate.pending_impl_bounds.is_empty());
            }
            match concrete.len() {
                0 => {}
                1 if concrete[0].pending_impl_bounds.is_empty() => return concrete.pop(),
                1 => {
                    if deferred_candidate.is_none() {
                        deferred_candidate = concrete.pop();
                    }
                }
                _ => {
                    if crate::type_services::visit::type_any(
                        &receiver_candidate.expr.ty,
                        |nested| {
                            matches!(nested, Type::TypeVar(_) | Type::Generic(_) | Type::Error)
                        },
                    ) {
                        continue;
                    }
                    let receiver_type = self.display_type(&receiver_candidate.expr.ty);
                    self.diagnostics.push_selection_with_span(
                        format!(
                            "Ambiguous selection for '{}' on type {}: multiple matching implementations",
                            method_name, receiver_type
                        ),
                        recv_span.clone(),
                    );
                    return None;
                }
            }
        }

        if deferred_candidate.is_some() {
            return deferred_candidate;
        }

        for candidate in &receiver_candidates {
            let candidate_ty =
                self.resolve_projection_type(&self.engine.resolve(&candidate.expr.ty));
            let receiver_is_current_trait_self = self.current_trait_self_receiver(&candidate_ty);
            let result = self.selection_service().select_current_trait_method(
                candidate.expr.clone(),
                method_name,
                receiver_is_current_trait_self,
            );
            if let Some(selected) = self.handle_optional_selection(result, recv_span.clone()) {
                return Some(selected);
            }
        }

        if self
            .selection_service()
            .mut_receiver_method_requires_mutable_receiver(
                &receiver_candidates,
                method_name,
                |ty| self.resolve_projection_type(&self.engine.resolve(ty)),
            )
        {
            self.diagnostics.push_selection_with_span(
                format!(
                    "Cannot call mutable receiver method '{}' without a mutable receiver",
                    method_name
                ),
                recv_span,
            );
        }

        None
    }

    pub(crate) fn selected_method_call_types(
        &mut self,
        selected: &crate::selection::SelectedMethod,
        args: Vec<HirExpr>,
    ) -> Option<(HirFunction, Type, Vec<HirExpr>, HirMethodCallTarget)> {
        let mut instantiated = selected.clone();
        self.instantiate_selected_callable_parameters(&mut instantiated);
        let selected = &instantiated;
        let method_func = selected.function.clone()?;
        let recv_resolved = self.engine.resolve(&selected.receiver.ty);
        let operation_span = selected.receiver.span.clone();
        let initial_method_subst = self.infer_selected_method_substitution(
            selected,
            &recv_resolved,
            &method_func,
            &args,
            operation_span.clone(),
        );

        let mut coerced_args = Vec::with_capacity(args.len());
        for (index, arg) in args.into_iter().enumerate() {
            if let Some(param) = selected.substituted_params.get(index) {
                let substituted_ty = if initial_method_subst.is_empty() {
                    param.ty.clone()
                } else {
                    param.ty.substitute_generics(&initial_method_subst)
                };
                coerced_args.push(self.coerce_argument_to_expected(arg, &substituted_ty));
            } else {
                coerced_args.push(arg);
            }
        }

        let method_subst = self.infer_selected_method_substitution(
            selected,
            &recv_resolved,
            &method_func,
            &coerced_args,
            operation_span,
        );
        let ret_ty = self
            .engine
            .resolve(&selected.return_type)
            .substitute_generics(&method_subst);
        let ret_ty = self.resolve_selected_projection_type(selected, &ret_ty);
        self.record_pending_impl_bounds(selected, &method_subst, "method call");
        self.record_function_generic_bounds(
            &method_func,
            &method_subst,
            selected.receiver.span.clone(),
            "method call",
        );
        let target = selected.target_with_substitution(&method_subst, |ty| self.engine.resolve(ty));

        Some((method_func, ret_ty, coerced_args, target))
    }

    pub(crate) fn record_function_generic_bounds(
        &mut self,
        func: &HirFunction,
        subst: &HashMap<GenericParamId, Type>,
        span: Span,
        context: &str,
    ) {
        for (generic_param, bounds) in &func.generic_bounds {
            let ty = subst
                .get(generic_param)
                .cloned()
                .unwrap_or(Type::Generic(*generic_param));
            let ty = self.engine.resolve(&ty);

            for bound in bounds {
                let bound = TraitBound {
                    trait_id: bound.trait_id,
                    type_args: bound
                        .type_args
                        .iter()
                        .map(|arg| self.engine.resolve(&arg.substitute_generics(subst)))
                        .collect(),
                };
                if let Type::TypeVar(id) = ty {
                    self.engine.add_bound(id, bound.clone());
                    self.constraint_store.add_trait(
                        Type::TypeVar(id),
                        bound,
                        span.clone(),
                        context,
                    );
                } else {
                    self.constraint_store
                        .add_trait(ty.clone(), bound, span.clone(), context);
                }
            }
        }
    }

    fn fresh_method_generic_subst(
        &mut self,
        method_func: &HirFunction,
        span: Span,
    ) -> HashMap<GenericParamId, Type> {
        method_func
            .generic_params
            .iter()
            .map(|param| {
                (
                    param.id,
                    self.engine
                        .fresh_type_var_at_kind(span.clone(), param.kind.clone()),
                )
            })
            .collect()
    }

    fn infer_selected_method_substitution(
        &mut self,
        selected: &crate::selection::SelectedMethod,
        recv_ty: &Type,
        method_func: &HirFunction,
        args: &[HirExpr],
        span: Span,
    ) -> HashMap<GenericParamId, Type> {
        let mut subst = self.infer_method_substitution(recv_ty, method_func, args, span.clone());
        for (param, ty) in &selected.owner_substitution {
            subst.insert(*param, ty.clone());
        }
        for param in &selected.owner_generic_params {
            let kind = self
                .engine
                .kind_of(&Type::Generic(*param))
                .unwrap_or(crate::type_services::kind::Kind::Type);
            subst
                .entry(*param)
                .or_insert_with(|| self.engine.fresh_type_var_at_kind(span.clone(), kind));
        }
        subst
    }

    fn resolve_selected_projection_type(
        &self,
        selected: &crate::selection::SelectedMethod,
        ty: &Type,
    ) -> Type {
        let ty = match ty {
            Type::Projection {
                ty: base_ty,
                trait_id,
                assoc_type,
                trait_args,
            } => {
                if selected.target.trait_id() == Some(*trait_id) && assoc_type.owner == *trait_id {
                    if let Some(assoc) = selected
                        .associated_types
                        .iter()
                        .find(|assoc| assoc.id == assoc_type.assoc_type_id)
                    {
                        return self.resolve_selected_projection_type(selected, &assoc.ty);
                    }
                }

                Type::Projection {
                    ty: Box::new(self.resolve_selected_projection_type(selected, base_ty)),
                    trait_id: *trait_id,
                    assoc_type: *assoc_type,
                    trait_args: trait_args
                        .iter()
                        .map(|arg| self.resolve_selected_projection_type(selected, arg))
                        .collect(),
                }
            }
            Type::Reference { mutable, inner } => Type::Reference {
                mutable: *mutable,
                inner: Box::new(self.resolve_selected_projection_type(selected, inner)),
            },
            Type::Pointer(inner) => Type::Pointer(Box::new(
                self.resolve_selected_projection_type(selected, inner),
            )),
            Type::Slice(inner) => Type::Slice(Box::new(
                self.resolve_selected_projection_type(selected, inner),
            )),
            Type::Array(inner, len) => Type::Array(
                Box::new(self.resolve_selected_projection_type(selected, inner)),
                *len,
            ),
            Type::Tuple(elems) => Type::Tuple(
                elems
                    .iter()
                    .map(|elem| self.resolve_selected_projection_type(selected, elem))
                    .collect(),
            ),
            Type::Function {
                params,
                ret,
                safety,
                callable_kind,
                captures,
            } => Type::Function {
                params: params
                    .iter()
                    .map(|param| self.resolve_selected_projection_type(selected, param))
                    .collect(),
                ret: Box::new(self.resolve_selected_projection_type(selected, ret)),
                safety: *safety,
                callable_kind: *callable_kind,
                captures: captures
                    .iter()
                    .map(|capture| crate::types::FunctionCapture {
                        kind: capture.kind,
                        ty: self.resolve_selected_projection_type(selected, &capture.ty),
                    })
                    .collect(),
            },
            Type::Struct { id, args } => Type::Struct {
                id: *id,
                args: args
                    .iter()
                    .map(|arg| self.resolve_selected_projection_type(selected, arg))
                    .collect(),
            },
            Type::Enum { id, args } => Type::Enum {
                id: *id,
                args: args
                    .iter()
                    .map(|arg| self.resolve_selected_projection_type(selected, arg))
                    .collect(),
            },
            _ => ty.clone(),
        };

        self.resolve_projection_type(&ty)
    }

    pub(crate) fn record_pending_impl_bounds(
        &mut self,
        selected: &crate::selection::SelectedMethod,
        method_subst: &HashMap<GenericParamId, Type>,
        context: &str,
    ) {
        for (ty, bound) in &selected.pending_impl_bounds {
            let ty = self.engine.resolve(&ty.substitute_generics(method_subst));
            let bound = TraitBound {
                trait_id: bound.trait_id,
                type_args: bound
                    .type_args
                    .iter()
                    .map(|arg| self.engine.resolve(&arg.substitute_generics(method_subst)))
                    .collect(),
            };
            if let Type::TypeVar(id) = ty {
                self.engine.add_bound(id, bound.clone());
                self.constraint_store.add_trait(
                    Type::TypeVar(id),
                    bound,
                    selected.receiver.span.clone(),
                    context,
                );
            } else {
                self.constraint_store
                    .add_trait(ty, bound, selected.receiver.span.clone(), context);
            }
        }
    }

    fn current_trait_self_receiver(&self, ty: &Type) -> bool {
        self.current_trait.is_some()
            && (matches!(ty, Type::TypeVar(_))
                || matches!(ty, Type::Generic(param) if self.generic_display_name(*param) == Some("Self")))
    }

    pub(crate) fn selection_service(&self) -> crate::selection::SelectionService<'_> {
        crate::selection::SelectionService::new(
            self.items.traits_for_selection(),
            self.items.impls_for_selection(),
            self.language_items
                .sized
                .as_ref()
                .map(|items| items.trait_id),
            self.current_trait_id,
            self.current_impl_bounds(),
        )
        .with_effective_trait_methods(&self.imported_effective_trait_methods)
    }

    fn fold_unary_calls(&mut self, mut expr: HirExpr, args: Vec<HirExpr>, span: Span) -> HirExpr {
        for arg in args {
            let resolved_callee_ty = self.resolve_projection_type(&self.engine.resolve(&expr.ty));
            if let Type::Reference { inner, .. } = resolved_callee_ty {
                if matches!(
                    self.resolve_projection_type(&self.engine.resolve(&inner)),
                    Type::Function { .. }
                ) {
                    let expr_span = expr.span.clone();
                    expr = HirExpr {
                        ty: *inner,
                        kind: HirExprKind::Deref(Box::new(expr)),
                        span: expr_span,
                    };
                }
            }
            let ret_ty = self.engine.fresh_type_var_at(span.clone());
            let safety = match self.resolve_projection_type(&self.engine.resolve(&expr.ty)) {
                Type::Function { safety, .. } => safety,
                _ => crate::types::FunctionSafety::Safe,
            };
            let expected_fn_ty =
                Type::function_with_safety(vec![arg.ty.clone()], ret_ty.clone(), safety);
            let _ = self.engine.unify(&expr.ty, &expected_fn_ty);
            let resolved_ret_ty = self.engine.resolve(&ret_ty);
            self.report_unsafe_function_value_call_from_type(&expr);

            let target = self.call_target_for_callee(&expr);
            expr = HirExpr {
                ty: resolved_ret_ty,
                kind: HirExprKind::Call(Box::new(expr), vec![arg], target),
                span: span.clone(),
            };
        }

        expr
    }

    fn index_display_type(&self, ty: &Type) -> String {
        self.display_type(ty)
    }

    pub(crate) fn apply_secondary(
        &mut self,
        expr: HirExpr,
        secondary: &ast::SecondaryExpr,
        use_kind: ExprUse,
        method_as_value: bool,
    ) -> HirExpr {
        let span = expr.span.clone();
        match secondary {
            ast::SecondaryExpr::Arguments(args) => {
                let mut expr = expr;
                let mut hir_args: Vec<HirExpr>;

                if matches!(self.engine.resolve(&expr.ty), Type::Error) {
                    return HirExpr {
                        ty: Type::Error,
                        kind: expr.kind.clone(),
                        span,
                    };
                }

                if let HirExprKind::FieldAccess(recv, method_name, _) = &expr.kind {
                    let recv_ty = self.engine.resolve(&recv.ty);
                    let found_method =
                        self.concrete_method_candidate((**recv).clone(), method_name);

                    let found_method = found_method.or_else(|| {
                        if let Type::TypeVar(var_id) = &recv_ty {
                            let bounds = self.engine.get_bounds(*var_id);
                            if bounds.is_empty() {
                                return None;
                            }
                            let receiver_candidates =
                                self.receiver_adjustment_candidates((**recv).clone());
                            let result = self
                                .selection_service()
                                .select_bound_method_preferring_non_ref_receiver(
                                    &receiver_candidates,
                                    &bounds,
                                    method_name,
                                    recv_ty.clone(),
                                    matches!(&recv.kind, HirExprKind::Call(_, _, _)),
                                );
                            if let Some(selected) =
                                self.handle_optional_selection(result, span.clone())
                            {
                                return Some(selected);
                            }
                        }
                        if let Type::Generic(gen_param) = &recv_ty {
                            let bounds = self
                                .current_impl_bounds()
                                .get(gen_param)
                                .cloned()
                                .unwrap_or_default();
                            let receiver_candidates =
                                self.receiver_adjustment_candidates((**recv).clone());
                            let result = self
                                .selection_service()
                                .select_bound_method_preferring_non_ref_receiver(
                                    &receiver_candidates,
                                    &bounds,
                                    method_name,
                                    recv_ty.clone(),
                                    matches!(&recv.kind, HirExprKind::Call(_, _, _)),
                                );
                            if let Some(selected) =
                                self.handle_optional_selection(result, span.clone())
                            {
                                return Some(selected);
                            }
                        }
                        if let Type::Reference { inner, .. } = &recv_ty {
                            if let Type::Generic(gen_param) = inner.as_ref() {
                                let bounds = self
                                    .current_impl_bounds()
                                    .get(gen_param)
                                    .cloned()
                                    .unwrap_or_default();
                                let receiver_candidates =
                                    self.receiver_adjustment_candidates((**recv).clone());
                                let result = self
                                    .selection_service()
                                    .select_bound_method_preferring_non_ref_receiver(
                                        &receiver_candidates,
                                        &bounds,
                                        method_name,
                                        inner.as_ref().clone(),
                                        matches!(&recv.kind, HirExprKind::Call(_, _, _)),
                                    );
                                if let Some(selected) =
                                    self.handle_optional_selection(result, span.clone())
                                {
                                    return Some(selected);
                                }
                            }
                        }
                        None
                    });

                    if let Some(selected) = found_method {
                        hir_args = args
                            .iter()
                            .enumerate()
                            .map(|(index, argument)| {
                                self.lower_selected_method_argument(&argument.arg, &selected, index)
                            })
                            .collect();
                        if hir_args.is_empty() && !selected.substituted_params.is_empty() {
                            self.diagnostics.push_type_with_span(
                                format!(
                                    "Type mismatch: method '{}' expected {} args, got 0",
                                    method_name,
                                    selected.substituted_params.len()
                                ),
                                span.clone(),
                            );
                            return self.error_expression_at(span.clone());
                        }

                        let adjusted_recv = self.apply_receiver_adjustment(
                            selected.receiver.clone(),
                            selected.receiver_adjustment,
                        );
                        let Some((method_func, ret_ty, coerced_args, target)) =
                            self.selected_method_call_types(&selected, hir_args)
                        else {
                            return self.error_expression_at(span.clone());
                        };
                        if let Some(method_id) = target.method_id() {
                            self.record_source_reference(
                                span.clone(),
                                crate::source_map::SourceSymbol::Definition(method_id),
                            );
                        }
                        hir_args = coerced_args;

                        if method_func.is_unsafe && !self.is_in_unsafe() {
                            self.diagnostics.push_selection_with_span(
                                format!(
                                    "Call to unsafe function '{}' requires an unsafe block",
                                    method_name
                                ),
                                span.clone(),
                            );
                        }

                        if method_func.is_curried && !hir_args.is_empty() {
                            let mut applied = HirExpr {
                                ty: ret_ty,
                                kind: HirExprKind::MethodCall(
                                    Box::new(adjusted_recv.clone()),
                                    method_name.clone(),
                                    vec![hir_args[0].clone()],
                                    method_func.self_receiver,
                                    Some(target),
                                ),
                                span: span.clone(),
                            };

                            if hir_args.len() > 1 {
                                applied = self.fold_unary_calls(
                                    applied,
                                    hir_args[1..].to_vec(),
                                    span.clone(),
                                );
                            }

                            return applied;
                        }

                        return HirExpr {
                            ty: ret_ty,
                            kind: HirExprKind::MethodCall(
                                Box::new(adjusted_recv),
                                method_name.clone(),
                                hir_args,
                                method_func.self_receiver,
                                Some(target),
                            ),
                            span,
                        };
                    }
                }

                let callable_selection =
                    if matches!(self.engine.resolve(&expr.ty), Type::Function { .. })
                        || matches!(expr.kind, HirExprKind::EnumVariant(..))
                    {
                        None
                    } else {
                        self.callable_method_candidate(expr.clone(), None)
                    };
                let expected_param_types = match self.engine.resolve(&expr.ty) {
                    Type::Function { params, .. } => Some(params),
                    _ => callable_selection
                        .as_ref()
                        .and_then(|selected| selected.substituted_params.first())
                        .map(|param| match (&param.ty, args.len()) {
                            (Type::Unit, 0) => Vec::new(),
                            (Type::Tuple(types), count) if count > 1 => types.clone(),
                            (ty, _) => vec![ty.clone()],
                        }),
                };
                hir_args = Vec::with_capacity(args.len());
                for (index, argument) in args.iter().enumerate() {
                    let expected = expected_param_types
                        .as_ref()
                        .and_then(|params| params.get(index))
                        .cloned();
                    let hir_arg = if let Some(lambda) = Self::lambda_from_expression(&argument.arg)
                    {
                        let mut expected_params = expected
                            .as_ref()
                            .and_then(|ty| self.callable_argument_parameters(ty));
                        if lambda.parameters.len() == 1
                            && !expected.as_ref().is_some_and(|ty| {
                                matches!(self.engine.resolve(ty), Type::Function { .. })
                            })
                        {
                            if let Some(params) = &mut expected_params {
                                if params.len() > 1 {
                                    *params = vec![Type::Tuple(std::mem::take(params))];
                                }
                            }
                        }
                        self.lower_lambda_with_expected_params(lambda, expected_params.as_deref())
                    } else {
                        self.lower_expression(&argument.arg)
                    };
                    hir_args.push(match expected {
                        Some(expected) => {
                            let hir_arg = self.coerce_argument_to_expected(hir_arg, &expected);
                            self.propagate_call_argument_bounds(&expected, &hir_arg.ty);
                            hir_arg
                        }
                        None => hir_arg,
                    });
                }

                if matches!(&expr.kind, HirExprKind::MethodCall(_, _, _, _, _))
                    && hir_args.is_empty()
                {
                    return expr;
                }

                if let HirExprKind::EnumVariant(enum_name, variant_name, _, variant_location) =
                    &expr.kind
                {
                    let enum_name = enum_name.clone();
                    let variant_name = variant_name.clone();
                    let variant_location = variant_location.clone();
                    let enum_id = variant_location
                        .as_ref()
                        .map(|location| location.owner)
                        .or_else(|| {
                            crate::lower::resolution::LowerResolutionContext::new(self)
                                .resolve_enum_type(&enum_name)
                                .map(|enum_info| enum_info.id)
                        });
                    let Some(enum_id) = enum_id else {
                        return self.error_expression_at(span.clone());
                    };
                    let type_args = if let Some(enum_info) =
                        self.items.enumeration(enum_id).cloned()
                    {
                        if !enum_info.generic_params.is_empty() {
                            let mut type_var_mapping: HashMap<GenericParamId, Type> =
                                HashMap::new();
                            for (index, _) in enum_info.generic_params.iter().enumerate() {
                                let tv = self.engine.fresh_type_var_at(span.clone());
                                type_var_mapping.insert(
                                    GenericParamId {
                                        owner: enum_info.id,
                                        index: index as u32,
                                    },
                                    tv,
                                );
                            }
                            if let Some(variant) =
                                enum_info.variants.iter().find(|v| v.name == variant_name)
                            {
                                if let HirVariantFields::Positional(field_types) = &variant.fields {
                                    for (arg, field_ty) in hir_args.iter().zip(field_types.iter()) {
                                        let expected =
                                            field_ty.substitute_generics(&type_var_mapping);
                                        let _ = self.engine.unify(&arg.ty, &expected);
                                    }
                                }
                            }
                            enum_info
                                .generic_params
                                .iter()
                                .enumerate()
                                .map(|(index, _)| {
                                    if let Some(tv) = type_var_mapping.get(&GenericParamId {
                                        owner: enum_info.id,
                                        index: index as u32,
                                    }) {
                                        self.engine.resolve(tv)
                                    } else {
                                        Type::Error
                                    }
                                })
                                .collect()
                        } else {
                            vec![]
                        }
                    } else {
                        vec![]
                    };
                    return HirExpr {
                        ty: Type::Enum {
                            id: enum_id,
                            args: type_args,
                        },
                        kind: HirExprKind::EnumVariant(
                            enum_name,
                            variant_name,
                            hir_args,
                            variant_location,
                        ),
                        span,
                    };
                }

                if let HirExprKind::Var(name) = &expr.kind {
                    if is_intrinsic_name(name) {
                        let unsafe_intrinsics = [
                            "PtrOffset",
                            "MakeArr",
                            "BorrowSlice",
                            "BorrowStr",
                            "DropInPlace",
                            "AtomicU64Exchange",
                            "AtomicU64FetchAdd",
                            "AtomicU64FetchSub",
                            "AtomicU64Store",
                        ];
                        if unsafe_intrinsics.contains(&name.as_str()) && !self.is_in_unsafe() {
                            self.diagnostics.push_selection_with_span(
                                format!("Intrinsic '~{}' requires an unsafe block", name),
                                span.clone(),
                            );
                        }
                        let expected_arg_types = infer_intrinsic_arg_types(name);
                        let is_atomic = name.starts_with("AtomicU64");
                        if is_atomic && hir_args.len() != expected_arg_types.len() {
                            self.diagnostics.push_with_span(
                                format!(
                                    "Intrinsic '~{}' expects exactly {} arguments, found {}",
                                    name,
                                    expected_arg_types.len(),
                                    hir_args.len()
                                ),
                                span.clone(),
                            );
                        }
                        for (arg, expected_ty) in hir_args.iter().zip(expected_arg_types.iter()) {
                            let _ = self.engine.unify(&arg.ty, expected_ty);
                            if is_atomic {
                                let resolved = self.engine.resolve(&arg.ty);
                                if resolved != *expected_ty {
                                    let expected = self.display_type(expected_ty);
                                    let found = self.display_type(&resolved);
                                    self.diagnostics.push_with_span(
                                        format!(
                                            "Intrinsic '~{}' expected argument type {}, found {}",
                                            name, expected, found
                                        ),
                                        arg.span.clone(),
                                    );
                                }
                            }
                        }
                        let mut arrptr_elem_ty: Option<Type> = None;
                        let mut intrinsic_ty_error = false;
                        if (name == "ArrPtr" || name == "ArrayLen") && !hir_args.is_empty() {
                            let resolved_arg_ty =
                                self.resolve_projection_type(&self.engine.resolve(&hir_args[0].ty));

                            match &resolved_arg_ty {
                                Type::Reference { inner, .. }
                                    if matches!(inner.as_ref(), Type::Str) =>
                                {
                                    if name == "ArrPtr" {
                                        arrptr_elem_ty = Some(Type::U8);
                                    }
                                }
                                Type::Reference { inner, .. }
                                    if matches!(
                                        inner.as_ref(),
                                        Type::Slice(_) | Type::Array(_, _)
                                    ) => {}
                                Type::Array(_, _) if name == "ArrPtr" => {
                                    let actual = self.display_type(&resolved_arg_ty);
                                    self.diagnostics.push_with_span(
                                        format!("ArrPtr expected slice, got {}", actual),
                                        hir_args[0].span.clone(),
                                    );
                                    intrinsic_ty_error = true;
                                }
                                _ => {
                                    let elem_ty = self.engine.fresh_type_var_at(span.clone());
                                    let array_ty = Type::Slice(Box::new(elem_ty.clone()));
                                    let _ = self.engine.unify(&hir_args[0].ty, &array_ty);
                                    if name == "ArrPtr" {
                                        arrptr_elem_ty = Some(elem_ty);
                                    }
                                }
                            }
                        }
                        if name == "DropInPlace" {
                            if hir_args.len() != 1 {
                                self.diagnostics.push_type_with_span(
                                    "DropInPlace requires exactly one argument".to_string(),
                                    span.clone(),
                                );
                                intrinsic_ty_error = true;
                            } else {
                                let pointee_ty = self.engine.fresh_type_var_at(span.clone());
                                let ptr_ty = Type::Pointer(Box::new(pointee_ty));
                                if self.engine.unify(&hir_args[0].ty, &ptr_ty).is_err() {
                                    let resolved_arg_ty = self.resolve_projection_type(
                                        &self.engine.resolve(&hir_args[0].ty),
                                    );
                                    let actual = self.display_type(&resolved_arg_ty);
                                    self.diagnostics.push_type_with_span(
                                        format!("DropInPlace expected raw pointer, got {}", actual),
                                        hir_args[0].span.clone(),
                                    );
                                    intrinsic_ty_error = true;
                                }
                            }
                        }
                        let ret_ty = if intrinsic_ty_error {
                            Type::Error
                        } else if let Some(elem_ty) = arrptr_elem_ty {
                            Type::Pointer(Box::new(elem_ty))
                        } else {
                            infer_intrinsic_return_type(name, &hir_args)
                        };
                        return HirExpr {
                            ty: ret_ty,
                            kind: HirExprKind::Intrinsic {
                                name: name.clone(),
                                args: hir_args,
                            },
                            span,
                        };
                    }
                }

                if let HirExprKind::Var(func_name) = &expr.kind {
                    let function_id = self.resolve_module_alias_or_item_def_id(func_name);
                    if let Some(func) = function_id.and_then(|id| self.items.function(id)).cloned()
                    {
                        if func.is_unsafe && !self.is_in_unsafe() {
                            self.diagnostics.push_selection_with_span(
                                format!(
                                    "Call to unsafe function '{}' requires an unsafe block",
                                    func_name
                                ),
                                span.clone(),
                            );
                        }
                        if !func.generic_params.is_empty()
                            && !matches!(expr.ty, Type::Function { .. })
                        {
                            expr.ty = self.instantiate_function_type(&func, expr.span.clone());
                        }
                    }
                }
                if let HirExprKind::ResolvedVar(reference) = &expr.kind {
                    if let HirVarTarget::Function(function_id) = reference.target {
                        if let Some(func) = self.current_function(function_id).cloned() {
                            if func.is_unsafe && !self.is_in_unsafe() {
                                self.diagnostics.push_selection_with_span(
                                    format!(
                                        "Call to unsafe function '{}' requires an unsafe block",
                                        reference.name
                                    ),
                                    span.clone(),
                                );
                            }
                            if !func.generic_params.is_empty()
                                && !matches!(expr.ty, Type::Function { .. })
                            {
                                expr.ty = self.instantiate_function_type(&func, expr.span.clone());
                            }
                        }
                    }
                }

                let resolved_expr_ty = self.engine.resolve(&expr.ty);
                if let Some(selected) = callable_selection {
                    return self.lower_callable_call(selected, hir_args);
                }
                let defer_argument_coercions = matches!(resolved_expr_ty, Type::TypeVar(_));
                if let Type::Function { params, .. } = &resolved_expr_ty {
                    if params.len() == 1 && hir_args.len() > 1 {
                        return self.fold_unary_calls(expr, hir_args, span);
                    }

                    let mut args_iter = hir_args.into_iter();
                    let mut coerced_args = Vec::new();

                    for param in params {
                        let Some(arg) = args_iter.next() else {
                            break;
                        };
                        coerced_args.push(self.coerce_argument_to_expected(arg, param));
                    }

                    coerced_args.extend(args_iter);
                    hir_args = coerced_args;

                    if params.len() != hir_args.len() {
                        self.diagnostics.push_selection_with_span(
                            format!(
                                "Type mismatch: function expected {} args, got {}",
                                params.len(),
                                hir_args.len()
                            ),
                            span.clone(),
                        );
                        let target = self.call_target_for_callee(&expr);
                        return HirExpr {
                            ty: Type::Error,
                            kind: HirExprKind::Call(Box::new(expr), hir_args, target),
                            span,
                        };
                    }
                }

                let ret_ty = self.engine.fresh_type_var_at(span.clone());
                let arg_types: Vec<Type> = hir_args
                    .iter()
                    .map(|arg| {
                        if defer_argument_coercions
                            && (matches!(expr.kind, HirExprKind::FieldAccess(..))
                                || self.language_items.fn_once.is_none())
                            && matches!(
                                self.engine.resolve(&arg.ty),
                                Type::Reference { inner, .. }
                                    if matches!(inner.as_ref(), Type::Array(_, _))
                            )
                        {
                            let expected = self.engine.fresh_type_var_at(arg.span.clone());
                            self.constraint_store.add_coercion(
                                arg.ty.clone(),
                                expected.clone(),
                                arg.span.clone(),
                                "deferred callable argument coercion",
                            );
                            expected
                        } else {
                            arg.ty.clone()
                        }
                    })
                    .collect();
                let safety = match self.resolve_projection_type(&self.engine.resolve(&expr.ty)) {
                    Type::Function { safety, .. } => safety,
                    _ => crate::types::FunctionSafety::Safe,
                };
                let expected_fn_ty = Type::function_with_safety(arg_types, ret_ty.clone(), safety);
                let mut callable_ty = self.engine.resolve(&expr.ty);
                while let Type::Reference { inner, .. } = callable_ty {
                    callable_ty = *inner;
                }
                let numeric_callee = if let Type::TypeVar(id) = callable_ty {
                    self.constraint_store
                        .literal_default_type_for_representative(id, |var| {
                            match self.engine.resolve(&Type::TypeVar(var)) {
                                Type::TypeVar(id) => Some(id),
                                _ => None,
                            }
                        })
                        .is_some()
                } else {
                    false
                };
                if !numeric_callee && !matches!(expr.kind, HirExprKind::FieldAccess(..)) {
                    if let Type::TypeVar(id) = callable_ty {
                        let protocol = if self.expr_is_mutable_lvalue(&expr) {
                            self.language_items
                                .fn_mut
                                .as_ref()
                                .map(|items| items.trait_id)
                        } else {
                            None
                        }
                        .or_else(|| {
                            self.language_items
                                .fn_once
                                .as_ref()
                                .map(|items| items.trait_id)
                        });
                        if let Some(trait_id) = protocol {
                            let args_ty = match hir_args.as_slice() {
                                [] => Type::Unit,
                                [arg] => arg.ty.clone(),
                                args => {
                                    Type::Tuple(args.iter().map(|arg| arg.ty.clone()).collect())
                                }
                            };
                            let bound = TraitBound {
                                trait_id,
                                type_args: vec![args_ty, ret_ty.clone()],
                            };
                            self.engine.add_bound(id, bound.clone());
                            self.constraint_store.add_trait(
                                callable_ty.clone(),
                                bound,
                                span.clone(),
                                "function call",
                            );
                            if let Some(selected) =
                                self.callable_method_candidate(expr.clone(), None)
                            {
                                return self.lower_callable_call(selected, hir_args);
                            }
                            return self.error_expression_at(span);
                        }
                    }
                }
                if (numeric_callee || self.engine.unify(&callable_ty, &expected_fn_ty).is_err())
                    && !matches!(expr.kind, HirExprKind::FieldAccess(..))
                {
                    let message = format!(
                        "value of type {} is not callable with these arguments",
                        self.display_type(&callable_ty)
                    );
                    self.diagnostics.push_type_with_span(message, span.clone());
                    return self.error_expression_at(span);
                }
                let resolved_ret_ty = self.engine.resolve(&ret_ty);
                self.report_unsafe_function_value_call_from_type(&expr);

                HirExpr {
                    ty: resolved_ret_ty,
                    kind: {
                        let target = self.call_target_for_callee(&expr);
                        HirExprKind::Call(Box::new(expr), hir_args, target)
                    },
                    span,
                }
            }
            ast::SecondaryExpr::Dot(ident_or_num) => match ident_or_num {
                ast::IdentOrNumber::Ident(ident) => {
                    let span = ident.span.clone();
                    let recv_ty = self.engine.resolve(&expr.ty);
                    let found_method = if method_as_value {
                        self.concrete_method_candidate(expr.clone(), &ident.name)
                    } else {
                        None
                    };

                    let mut found_method = found_method.or_else(|| {
                        if let Type::TypeVar(var_id) = &recv_ty {
                            let bounds = self.engine.get_bounds(*var_id);
                            if bounds.is_empty() {
                                return None;
                            }
                            let receiver_candidates =
                                self.receiver_adjustment_candidates(expr.clone());
                            let result = self
                                .selection_service()
                                .select_bound_method_preferring_non_ref_receiver(
                                    &receiver_candidates,
                                    &bounds,
                                    &ident.name,
                                    recv_ty.clone(),
                                    matches!(&expr.kind, HirExprKind::Call(_, _, _)),
                                );
                            if let Some(selected) =
                                self.handle_optional_selection(result, span.clone())
                            {
                                return Some(selected);
                            }
                        }
                        if let Type::Generic(gen_param) = &recv_ty {
                            let bounds = self
                                .current_impl_bounds()
                                .get(gen_param)
                                .cloned()
                                .unwrap_or_default();
                            let receiver_candidates =
                                self.receiver_adjustment_candidates(expr.clone());
                            let result = self
                                .selection_service()
                                .select_bound_method_preferring_non_ref_receiver(
                                    &receiver_candidates,
                                    &bounds,
                                    &ident.name,
                                    recv_ty.clone(),
                                    matches!(&expr.kind, HirExprKind::Call(_, _, _)),
                                );
                            if let Some(selected) =
                                self.handle_optional_selection(result, span.clone())
                            {
                                return Some(selected);
                            }
                        }
                        if let Type::Reference { inner, .. } = &recv_ty {
                            if let Type::Generic(gen_param) = inner.as_ref() {
                                let bounds = self
                                    .current_impl_bounds()
                                    .get(gen_param)
                                    .cloned()
                                    .unwrap_or_default();
                                let receiver_candidates =
                                    self.receiver_adjustment_candidates(expr.clone());
                                let result = self
                                    .selection_service()
                                    .select_bound_method_preferring_non_ref_receiver(
                                        &receiver_candidates,
                                        &bounds,
                                        &ident.name,
                                        inner.as_ref().clone(),
                                        matches!(&expr.kind, HirExprKind::Call(_, _, _)),
                                    );
                                if let Some(selected) =
                                    self.handle_optional_selection(result, span.clone())
                                {
                                    return Some(selected);
                                }
                            }
                        }
                        None
                    });

                    if method_as_value && found_method.is_none() {
                        let inferred = self.selection_service().select_inferred_method_candidates(
                            &expr,
                            &ident.name,
                            |ty| self.resolve_projection_type(&self.engine.resolve(ty)),
                        );
                        if inferred.len() == 1 {
                            found_method = inferred.into_iter().next();
                        } else if inferred.len() > 1
                            && !crate::type_services::visit::type_any(&recv_ty, |ty| {
                                matches!(ty, Type::TypeVar(_))
                            })
                        {
                            let receiver_type = self.display_type(&recv_ty);
                            self.diagnostics.push_selection_with_span(
                                format!(
                                    "Ambiguous selection for '{}' on type {}",
                                    ident.name, receiver_type
                                ),
                                span.clone(),
                            );
                            return self.error_expression_at(span.clone());
                        }
                    }

                    if method_as_value {
                        if let Some(selected) = found_method.clone() {
                            let Some(method_func) = selected.function.clone() else {
                                return self.error_expression_at(span.clone());
                            };
                            let method_subst =
                                self.fresh_method_generic_subst(&method_func, span.clone());
                            let lambda_params: Vec<HirParam> = selected
                                .substituted_params
                                .iter()
                                .cloned()
                                .map(|mut param| {
                                    param.ty = param.ty.substitute_generics(&method_subst);
                                    param
                                })
                                .collect();
                            let lambda_arg_types: Vec<Type> =
                                lambda_params.iter().map(|param| param.ty.clone()).collect();
                            let call_args: Vec<HirExpr> = lambda_params
                                .iter()
                                .map(|param| HirExpr {
                                    ty: param.ty.clone(),
                                    kind: HirExprKind::Var(param.name.clone()),
                                    span: span.clone(),
                                })
                                .collect();
                            let call_ret_ty = self.resolve_projection_type(
                                &selected.return_type.substitute_generics(&method_subst),
                            );
                            self.record_pending_impl_bounds(
                                &selected,
                                &method_subst,
                                "method value",
                            );
                            self.record_function_generic_bounds(
                                &method_func,
                                &method_subst,
                                span.clone(),
                                "method value",
                            );
                            if method_func.is_unsafe && !self.is_in_unsafe() {
                                self.diagnostics.push_selection_with_span(
                                    format!(
                                        "Call to unsafe function '{}' requires an unsafe block",
                                        ident.name
                                    ),
                                    span.clone(),
                                );
                            }
                            let adjusted_receiver = self.apply_receiver_adjustment(
                                selected.receiver.clone(),
                                selected.receiver_adjustment,
                            );
                            let mut target = selected.target;
                            let owner_params = target
                                .owner_substitution
                                .iter()
                                .map(|binding| binding.param)
                                .collect::<std::collections::HashSet<_>>();
                            let selected_trait_id = target.trait_id();
                            target.method_substitution = method_subst
                                .iter()
                                .filter(|(param, _)| {
                                    !owner_params.contains(param)
                                        && selected_trait_id
                                            .is_none_or(|trait_id| param.owner != trait_id)
                                })
                                .map(|(&param, ty)| crate::hir::HirTypeBinding {
                                    param,
                                    ty: self.engine.resolve(ty),
                                })
                                .collect();
                            target.method_substitution.sort_by_key(|binding| {
                                (
                                    binding.param.owner.crate_id.0,
                                    binding.param.owner.local.0,
                                    binding.param.index,
                                )
                            });
                            let call_expr = HirExpr {
                                ty: call_ret_ty.clone(),
                                kind: HirExprKind::MethodCall(
                                    Box::new(adjusted_receiver),
                                    ident.name.clone(),
                                    call_args,
                                    method_func.self_receiver,
                                    Some(target),
                                ),
                                span: span.clone(),
                            };

                            let body = HirBlock {
                                ty: call_ret_ty.clone(),
                                stmts: vec![HirStmt::Expr(call_expr)],
                            };
                            let captures = self.collect_lambda_captures(&body, &lambda_params);

                            return HirExpr {
                                ty: Self::lambda_function_type(
                                    lambda_arg_types,
                                    call_ret_ty,
                                    crate::types::FunctionSafety::from_is_unsafe(
                                        method_func.is_unsafe,
                                    ),
                                    &captures,
                                ),
                                kind: HirExprKind::Lambda {
                                    params: lambda_params,
                                    body,
                                    captures,
                                },
                                span,
                            };
                        }
                    }

                    let field_name = &ident.name;
                    let mut field_ty = self.engine.fresh_type_var_at(span.clone());
                    let resolved_ty = self.engine.resolve(&expr.ty);
                    let original_expr = expr.clone();

                    if !method_as_value {
                        let mut receiver_candidates =
                            self.receiver_adjustment_candidates(original_expr.clone());
                        for candidate in &mut receiver_candidates {
                            candidate.can_autoref_mut = true;
                        }
                        if !self
                            .selection_service()
                            .select_concrete_method_candidates(
                                &receiver_candidates,
                                field_name,
                                |ty| self.resolve_projection_type(&self.engine.resolve(ty)),
                            )
                            .is_empty()
                        {
                            return HirExpr {
                                ty: field_ty,
                                kind: HirExprKind::FieldAccess(
                                    Box::new(original_expr),
                                    ident.name.clone(),
                                    None,
                                ),
                                span,
                            };
                        }
                    }

                    let (base_expr, inner_ty) = if let Type::Reference { inner, .. } = &resolved_ty
                    {
                        let deref_expr = HirExpr {
                            ty: inner.as_ref().clone(),
                            kind: HirExprKind::Deref(Box::new(expr)),
                            span: span.clone(),
                        };
                        (deref_expr, self.engine.resolve(inner.as_ref()))
                    } else {
                        (expr, resolved_ty)
                    };

                    if let Some(selected) = found_method {
                        let Some(method_func) = selected.function.as_ref() else {
                            return self.error_expression_at(span.clone());
                        };
                        let method_subst =
                            self.fresh_method_generic_subst(method_func, span.clone());
                        self.record_pending_impl_bounds(&selected, &method_subst, "method value");
                        self.record_function_generic_bounds(
                            method_func,
                            &method_subst,
                            span.clone(),
                            "method value",
                        );
                        if method_func.is_unsafe && !self.is_in_unsafe() {
                            self.diagnostics.push_selection_with_span(
                                format!(
                                    "Call to unsafe function '{}' requires an unsafe block",
                                    ident.name
                                ),
                                span.clone(),
                            );
                        }
                        let param_types = selected
                            .substituted_params
                            .iter()
                            .map(|param| param.ty.substitute_generics(&method_subst))
                            .collect();
                        field_ty = Type::function_with_safety(
                            param_types,
                            self.resolve_projection_type(
                                &selected.return_type.substitute_generics(&method_subst),
                            ),
                            crate::types::FunctionSafety::from_is_unsafe(method_func.is_unsafe),
                        );
                        return HirExpr {
                            ty: field_ty,
                            kind: HirExprKind::FieldAccess(
                                Box::new(original_expr),
                                ident.name.clone(),
                                None,
                            ),
                            span,
                        };
                    }

                    let mut field_location = None;
                    if matches!(inner_ty, Type::TypeVar(_)) {
                        let mut matches = self
                            .items
                            .structures()
                            .filter(|(_, structure)| self.current_def_ids.contains(&structure.id))
                            .filter_map(|(id, structure)| {
                                structure
                                    .fields
                                    .iter()
                                    .find(|field| field.name == *field_name)
                                    .map(|field| {
                                        (
                                            self.canonical_name_for_def_id(id)
                                                .map(str::to_string)
                                                .unwrap_or_else(|| structure.name.clone()),
                                            structure.clone(),
                                            field.clone(),
                                        )
                                    })
                            })
                            .collect::<Vec<_>>();
                        matches.sort_by_key(|(_, structure, _)| structure.id);
                        matches.dedup_by_key(|(_, structure, _)| structure.id);
                        if let [(struct_name, structure, field)] = matches.as_slice() {
                            let type_args = structure
                                .generic_params
                                .iter()
                                .map(|param| {
                                    self.engine
                                        .fresh_type_var_at_kind(span.clone(), param.kind.clone())
                                })
                                .collect::<Vec<_>>();
                            field_ty = self
                                .lower_struct_field_type(struct_name, &type_args, field_name, &span)
                                .unwrap_or(Type::Error);
                            field_location = Some(HirFieldLocation {
                                owner: structure.id,
                                field_id: field.id,
                                name: field_name.clone(),
                            });
                        }
                    } else if let Type::Struct {
                        id,
                        args: type_args,
                    } = &inner_ty
                    {
                        let found_struct = self.items.structure(*id);
                        let found_struct_name = found_struct.and_then(|hir_struct| {
                            self.canonical_name_for_def_id(hir_struct.id)
                                .map(str::to_string)
                                .or_else(|| Some(hir_struct.name.clone()))
                        });
                        let found_field = found_struct.and_then(|hir_struct| {
                            hir_struct
                                .fields
                                .iter()
                                .find(|field| field.name == *field_name)
                                .map(|field| HirFieldLocation {
                                    owner: hir_struct.id,
                                    field_id: field.id,
                                    name: field_name.clone(),
                                })
                        });

                        if found_field.is_some() {
                            field_ty = self
                                .lower_struct_field_type(
                                    found_struct_name.as_deref().unwrap_or(""),
                                    type_args,
                                    field_name,
                                    &span,
                                )
                                .unwrap_or(Type::Error);
                            field_location = found_field;
                        }
                    }

                    let field_base = if field_location.is_some() {
                        base_expr
                    } else {
                        original_expr
                    };
                    HirExpr {
                        ty: field_ty,
                        kind: HirExprKind::FieldAccess(
                            Box::new(field_base),
                            ident.name.clone(),
                            field_location,
                        ),
                        span,
                    }
                }
                ast::IdentOrNumber::Number(n) => {
                    let elem_ty = match &expr.ty {
                        Type::Tuple(elems) if (*n as usize) < elems.len() => {
                            elems[*n as usize].clone()
                        }
                        _ => self.engine.fresh_type_var_at(span.clone()),
                    };

                    HirExpr {
                        ty: elem_ty,
                        kind: HirExprKind::TupleIndex(Box::new(expr), *n as u32),
                        span,
                    }
                }
            },
            ast::SecondaryExpr::Indice(index_expr) => {
                let index = self.lower_expression(index_expr);
                let resolved_expr_ty = self.resolve_projection_type(&self.engine.resolve(&expr.ty));
                if matches!(resolved_expr_ty, Type::Str)
                    || matches!(
                        &resolved_expr_ty,
                        Type::Reference { inner, .. } if matches!(inner.as_ref(), Type::Str)
                    )
                {
                    self.diagnostics.push_selection_with_span(
                        "cannot index Str by integer; string slices are UTF-8 text, use an explicit string or byte API"
                            .to_string(),
                        span.clone(),
                    );
                    return self.error_expression_at(span.clone());
                }

                let is_array_receiver = matches!(resolved_expr_ty, Type::Array(_, _))
                    || matches!(
                        &resolved_expr_ty,
                        Type::Reference { inner, .. }
                            if matches!(inner.as_ref(), Type::Array(_, _))
                    );
                let autoderef_candidates = if is_array_receiver {
                    let mut receiver_candidates = self.receiver_adjustment_candidates(expr.clone());
                    if use_kind == ExprUse::Value {
                        receiver_candidates.retain(|candidate| {
                            !matches!(
                                self.resolve_projection_type(
                                    &self.engine.resolve(&candidate.expr.ty)
                                ),
                                Type::Reference { mutable: true, .. }
                            )
                        });
                    }
                    receiver_candidates
                        .into_iter()
                        .flat_map(|candidate| {
                            let adjustment = candidate.adjustment;
                            self.autoderef_candidates(candidate.expr)
                                .into_iter()
                                .map(move |expr| crate::selection::ReceiverCandidate {
                                    expr,
                                    adjustment,
                                    can_autoref_mut: false,
                                })
                        })
                        .collect::<Vec<_>>()
                } else {
                    self.autoderef_candidates(expr.clone())
                        .into_iter()
                        .enumerate()
                        .map(|(index, expr)| crate::selection::ReceiverCandidate {
                            can_autoref_mut: index == 0
                                && (self.expr_is_mutable_lvalue(&expr)
                                    || matches!(
                                        self.resolve_projection_type(
                                            &self.engine.resolve(&expr.ty)
                                        ),
                                        Type::Pointer(_)
                                    )),
                            expr,
                            adjustment: if index == 0 {
                                crate::selection::ReceiverAdjustment::None
                            } else {
                                crate::selection::ReceiverAdjustment::BuiltinDeref
                            },
                        })
                        .collect::<Vec<_>>()
                };
                let resolved_index_ty = self.resolve_projection_type(&index.ty);
                let index_protocol = match use_kind {
                    ExprUse::Value => self
                        .language_items
                        .index
                        .as_ref()
                        .map(|items| (items.trait_id, items.method_id, items.output_id, false)),
                    ExprUse::AssignmentPlace => self
                        .language_items
                        .index_mut
                        .as_ref()
                        .map(|items| (items.trait_id, items.method_id, items.output_id, true)),
                };
                let operation = if use_kind == ExprUse::AssignmentPlace {
                    "mutable []"
                } else {
                    "[]"
                };

                let is_unresolved_receiver =
                    matches!(&resolved_expr_ty, Type::TypeVar(_) | Type::Generic(_));
                let Some((index_trait_id, index_method_id, index_output_id, index_mutable)) =
                    index_protocol
                else {
                    self.diagnostics.push_selection_with_span(
                        if use_kind == ExprUse::AssignmentPlace {
                            "Cannot use mutable indexing because the IndexMut language-item protocol is unavailable".to_string()
                        } else {
                            "Cannot use indexing because the Index language-item protocol is unavailable".to_string()
                        },
                        span.clone(),
                    );
                    return self.error_expression_at(span.clone());
                };
                let mut selected_index = None;
                match self.selection_service().select_required_index_method(
                    &autoderef_candidates,
                    index_trait_id,
                    index_method_id,
                    index_output_id,
                    &resolved_index_ty,
                    operation,
                    |ty| self.resolve_projection_type(&self.engine.resolve(ty)),
                ) {
                    Ok(selected) => selected_index = Some(selected),
                    Err(crate::selection::SelectionDiagnostic::NoImplementation { .. })
                        if is_unresolved_receiver => {}
                    Err(crate::selection::SelectionDiagnostic::NoImplementation { .. }) => {
                        let display_ty = self.index_display_type(&resolved_expr_ty);
                        self.diagnostics.push_selection_with_span(
                            if use_kind == ExprUse::AssignmentPlace {
                                format!(
                                    "No mutable indexing implementation found for operator '[]' on type {}",
                                    display_ty
                                )
                            } else {
                                format!(
                                    "No implementation found for operator '[]' on type {}",
                                    display_ty
                                )
                            },
                            span.clone(),
                        );
                        return self.error_expression_at(span.clone());
                    }
                    Err(error) => {
                        let message = self.display_selection_error(&error);
                        self.diagnostics
                            .push_selection_with_span(message, span.clone());
                        return self.error_expression_at(span.clone());
                    }
                }

                match &resolved_expr_ty {
                    Type::TypeVar(id) => {
                        let bound = TraitBound {
                            trait_id: index_trait_id,
                            type_args: vec![index.ty.clone()],
                        };
                        self.engine.add_bound(*id, bound.clone());
                        self.constraint_store.add_trait(
                            resolved_expr_ty.clone(),
                            bound,
                            span.clone(),
                            "index operator",
                        );
                    }
                    Type::Generic(_) => {}
                    _ if selected_index.is_some() => {}
                    _ if TypeFacts::is_concrete(&resolved_expr_ty) => {
                        let display_ty = self.index_display_type(&resolved_expr_ty);
                        self.diagnostics.push_selection_with_span(
                            if use_kind == ExprUse::AssignmentPlace {
                                format!(
                                    "No mutable indexing implementation found for operator '[]' on type {}",
                                    display_ty
                                )
                            } else {
                                format!(
                                    "No implementation found for operator '[]' on type {}",
                                    display_ty
                                )
                            },
                            span.clone(),
                        );
                    }
                    _ => {}
                }

                let unresolved_generic_index = match &resolved_expr_ty {
                    Type::TypeVar(_) | Type::Generic(_) => {
                        let return_type = Type::Reference {
                            mutable: index_mutable,
                            inner: Box::new(Type::Projection {
                                ty: Box::new(resolved_expr_ty.clone()),
                                trait_id: index_trait_id,
                                assoc_type: crate::types::AssociatedTypeKey {
                                    owner: index_trait_id,
                                    assoc_type_id: index_output_id,
                                },
                                trait_args: vec![index.ty.clone()],
                            }),
                        };
                        let result = self
                            .selection_service()
                            .select_unresolved_generic_required_index_method(
                                expr.clone(),
                                index_trait_id,
                                index_method_id,
                                index_output_id,
                                index.ty.clone(),
                                return_type,
                                operation,
                            );
                        self.handle_optional_selection(result, span.clone())
                    }
                    _ => None,
                };

                let selected_index = selected_index.or(unresolved_generic_index);
                let Some(selected) = selected_index else {
                    return self.error_expression_at(span.clone());
                };
                let (receiver_expr, receiver_ty, target, method_name, self_receiver) = {
                    let selected_method_name = selected
                        .function
                        .as_ref()
                        .map(|function| function.name.clone())
                        .unwrap_or_else(|| "[]".to_string());
                    if selected
                        .function
                        .as_ref()
                        .is_some_and(|func| func.is_unsafe)
                        && !self.is_in_unsafe()
                    {
                        self.diagnostics.push_selection_with_span(
                            format!(
                                "Call to unsafe function '{}' requires an unsafe block",
                                selected_method_name
                            ),
                            span.clone(),
                        );
                    }
                    self.record_pending_impl_bounds(&selected, &HashMap::new(), "index operator");
                    let selected_return_type = &selected.return_type;
                    let expected_output =
                        Type::Projection {
                            ty: Box::new(self.resolve_projection_type(
                                &self.engine.resolve(&selected.receiver.ty),
                            )),
                            trait_id: index_trait_id,
                            assoc_type: crate::types::AssociatedTypeKey {
                                owner: index_trait_id,
                                assoc_type_id: index_output_id,
                            },
                            trait_args: vec![index.ty.clone()],
                        };
                    let expected_output =
                        self.resolve_selected_projection_type(&selected, &expected_output);
                    let return_matches = match selected_return_type {
                        Type::Reference {
                            mutable: actual,
                            inner,
                        } => {
                            let actual_inner =
                                self.resolve_projection_type(&self.engine.resolve(inner));
                            let expected_inner = self
                                .resolve_projection_type(&self.engine.resolve(&expected_output));
                            *actual == index_mutable
                                && (!TypeFacts::is_concrete(&actual_inner)
                                    || !TypeFacts::is_concrete(&expected_inner)
                                    || self.engine.unify(&actual_inner, &expected_inner).is_ok())
                        }
                        _ => false,
                    };
                    if !return_matches {
                        self.diagnostics.push_selection_with_span(
                            format!(
                                "Index implementation must return {}",
                                if index_mutable {
                                    "&mut Output"
                                } else {
                                    "&Output"
                                }
                            ),
                            span.clone(),
                        );
                        return self.error_expression_at(span.clone());
                    }
                    (
                        self.apply_receiver_adjustment(
                            selected.receiver.clone(),
                            selected.receiver_adjustment,
                        ),
                        self.resolve_projection_type(&self.engine.resolve(&selected.receiver.ty)),
                        Some(selected.target.clone()),
                        selected_method_name,
                        selected
                            .function
                            .as_ref()
                            .and_then(|function| function.self_receiver)
                            .or(Some(crate::types::ReceiverMode::Shared)),
                    )
                };

                let projection = Type::Projection {
                    ty: Box::new(receiver_ty.clone()),
                    trait_id: index_trait_id,
                    assoc_type: crate::types::AssociatedTypeKey {
                        owner: index_trait_id,
                        assoc_type_id: index_output_id,
                    },
                    trait_args: vec![index.ty.clone()],
                };
                let output_ty = self.resolve_selected_projection_type(&selected, &projection);
                let method_call = HirExpr {
                    ty: Type::Reference {
                        mutable: index_mutable,
                        inner: Box::new(output_ty),
                    },
                    kind: HirExprKind::MethodCall(
                        Box::new(receiver_expr),
                        method_name,
                        vec![index],
                        self_receiver,
                        target,
                    ),
                    span: span.clone(),
                };

                HirExpr {
                    ty: self.resolve_projection_type(match &method_call.ty {
                        Type::Reference { inner, .. } => inner.as_ref(),
                        _ => &Type::Error,
                    }),
                    kind: HirExprKind::Deref(Box::new(method_call)),
                    span,
                }
            }
            ast::SecondaryExpr::Interogation => self.lower_try_interogation(expr, span),
        }
    }

    fn lower_try_interogation(&mut self, mut expr: HirExpr, span: Span) -> HirExpr {
        let carrier_ty = self.resolve_projection_type(&self.engine.resolve(&expr.ty));
        expr.ty = carrier_ty.clone();
        let return_ty = self
            .current_body_return_type()
            .unwrap_or_else(|| self.engine.fresh_type_var_at(span.clone()));
        let return_ty = self.resolve_projection_type(&self.engine.resolve(&return_ty));

        let Some(try_protocol) = self.language_items.try_protocol.clone() else {
            self.diagnostics.push_selection_with_span(
                "Cannot use '?' because the Try language-item protocol is unavailable".to_string(),
                span.clone(),
            );
            return HirExpr {
                ty: Type::Error,
                kind: expr.kind,
                span,
            };
        };

        let Some(try_trait) = self.trait_by_id(try_protocol.try_trait_id).cloned() else {
            self.diagnostics.push_selection_with_span(
                "Cannot use '?' because the Try language-item protocol is unavailable".to_string(),
                span.clone(),
            );
            return self.error_expression_at(span.clone());
        };

        if matches!(carrier_ty, Type::TypeVar(_)) {
            let output_ty = self.engine.fresh_type_var_at(span.clone());
            let residual_ty = self.engine.fresh_type_var_at(span.clone());
            return self.build_try_expression(
                expr,
                None,
                None,
                None,
                output_ty,
                residual_ty,
                return_ty,
                &try_protocol,
                span,
            );
        }

        if Self::try_type_contains_unresolved(&carrier_ty) {
            let output_ty = self.engine.fresh_type_var_at(span.clone());
            let residual_ty = self.engine.fresh_type_var_at(span.clone());
            return self.build_try_expression(
                expr,
                None,
                None,
                None,
                output_ty,
                residual_ty,
                return_ty,
                &try_protocol,
                span,
            );
        }

        let selected_branch = match self.selection_service().select_required_trait_member(
            &expr,
            &carrier_ty,
            try_trait.id,
            try_protocol.branch_method_id,
        ) {
            Ok(selected) => selected,
            Err(error) => {
                let message = match error {
                    crate::selection::SelectionDiagnostic::NoImplementation { .. } => {
                        format!(
                            "Cannot use '?' on non-carrier type {}",
                            self.display_type(&carrier_ty)
                        )
                    }
                    _ => self.display_selection_error(&error),
                };
                self.diagnostics
                    .push_selection_with_span(message, span.clone());
                return HirExpr {
                    ty: Type::Error,
                    kind: expr.kind,
                    span,
                };
            }
        };

        let branch_subst = selected_branch
            .function
            .as_ref()
            .map(|func| {
                self.infer_selected_method_substitution(
                    &selected_branch,
                    &carrier_ty,
                    func,
                    &[],
                    span.clone(),
                )
            })
            .unwrap_or_default();
        self.record_pending_impl_bounds(&selected_branch, &HashMap::new(), "try operator");
        if let Some(func) = selected_branch.function.as_ref() {
            self.record_function_generic_bounds(func, &branch_subst, span.clone(), "try operator");
            if func.is_unsafe && !self.is_in_unsafe() {
                self.diagnostics.push_selection_with_span(
                    format!(
                        "Call to unsafe function '{}' requires an unsafe block",
                        func.name
                    ),
                    span.clone(),
                );
            }
        }

        let Some(output_ty) =
            self.selected_associated_type(&selected_branch, &try_trait, try_protocol.output_id)
        else {
            self.diagnostics.push_with_span(
                "Cannot use '?' because the Try language-item output type is unavailable"
                    .to_string(),
                span.clone(),
            );
            return self.error_expression_at(span.clone());
        };
        let Some(residual_ty) =
            self.selected_associated_type(&selected_branch, &try_trait, try_protocol.residual_id)
        else {
            self.diagnostics.push_with_span(
                "Cannot use '?' because the Try language-item residual type is unavailable"
                    .to_string(),
                span.clone(),
            );
            return self.error_expression_at(span.clone());
        };

        let branch_method =
            selected_branch.target_with_substitution(&branch_subst, |ty| self.engine.resolve(ty));
        if matches!(return_ty, Type::TypeVar(_)) {
            let expr = self.apply_receiver_adjustment(
                selected_branch.receiver.clone(),
                selected_branch.receiver_adjustment,
            );
            return self.build_try_expression(
                expr,
                Some(branch_method),
                selected_branch
                    .function
                    .as_ref()
                    .and_then(|func| func.self_receiver),
                None,
                output_ty,
                residual_ty,
                return_ty,
                &try_protocol,
                span,
            );
        }

        if Self::try_type_contains_unresolved(&return_ty) {
            let expr = self.apply_receiver_adjustment(
                selected_branch.receiver.clone(),
                selected_branch.receiver_adjustment,
            );
            return self.build_try_expression(
                expr,
                Some(branch_method),
                selected_branch
                    .function
                    .as_ref()
                    .and_then(|func| func.self_receiver),
                None,
                output_ty,
                residual_ty,
                return_ty,
                &try_protocol,
                span,
            );
        }

        let Some(from_residual_trait) = self
            .trait_by_id(try_protocol.from_residual_trait_id)
            .cloned()
        else {
            self.diagnostics.push_with_span(
                "Cannot use '?' because the FromResidual language-item trait is unavailable"
                    .to_string(),
                span.clone(),
            );
            return self.error_expression_at(span.clone());
        };
        let return_owner = HirExpr {
            ty: return_ty.clone(),
            kind: HirExprKind::Unit,
            span: span.clone(),
        };
        let selected_from_residual = match self.selection_service().select_static_trait_member(
            &return_owner,
            &return_ty,
            from_residual_trait.id,
            std::slice::from_ref(&residual_ty),
            try_protocol.from_residual_method_id,
        ) {
            Ok(selected) => selected,
            Err(error) => {
                let message = match error {
                    crate::selection::SelectionDiagnostic::NoImplementation { .. } => {
                        let return_type = self.display_type(&return_ty);
                        let residual_type = self.display_type(&residual_ty);
                        format!(
                            "Cannot use '?' because {} does not implement FromResidual {}",
                            return_type, residual_type
                        )
                    }
                    _ => self.display_selection_error(&error),
                };
                self.diagnostics
                    .push_selection_with_span(message, span.clone());
                return self.error_expression_at(span.clone());
            }
        };

        let residual_arg = HirExpr {
            ty: residual_ty.clone(),
            kind: HirExprKind::Unit,
            span: span.clone(),
        };
        let from_residual_subst = selected_from_residual
            .function
            .as_ref()
            .map(|func| {
                self.infer_selected_method_substitution(
                    &selected_from_residual,
                    &return_ty,
                    func,
                    &[residual_arg],
                    span.clone(),
                )
            })
            .unwrap_or_default();
        self.record_pending_impl_bounds(&selected_from_residual, &HashMap::new(), "try operator");
        if let Some(func) = selected_from_residual.function.as_ref() {
            self.record_function_generic_bounds(
                func,
                &from_residual_subst,
                span.clone(),
                "try operator",
            );
            if func.is_unsafe && !self.is_in_unsafe() {
                self.diagnostics.push_selection_with_span(
                    format!(
                        "Call to unsafe function '{}' requires an unsafe block",
                        func.name
                    ),
                    span.clone(),
                );
            }
        }

        let from_residual_method = selected_from_residual
            .target_with_substitution(&from_residual_subst, |ty| self.engine.resolve(ty));

        let expr = self.apply_receiver_adjustment(
            selected_branch.receiver.clone(),
            selected_branch.receiver_adjustment,
        );

        self.build_try_expression(
            expr,
            Some(branch_method),
            selected_branch
                .function
                .as_ref()
                .and_then(|func| func.self_receiver),
            Some(HirCallTarget::StaticMethod(HirStaticMethodTarget {
                owner_ty: return_ty.clone(),
                method: from_residual_method,
            })),
            output_ty,
            residual_ty,
            return_ty,
            &try_protocol,
            span,
        )
    }

    fn record_try_trait_bound(
        &mut self,
        carrier_ty: &Type,
        protocol: &crate::language_items::TryLanguageItems<DefId>,
        span: Span,
    ) {
        let bound = TraitBound {
            trait_id: protocol.try_trait_id,
            type_args: Vec::new(),
        };
        if let Type::TypeVar(id) = carrier_ty {
            self.engine.add_bound(*id, bound.clone());
        }
        self.constraint_store
            .add_trait(carrier_ty.clone(), bound, span, "try operator");
    }

    fn try_type_contains_unresolved(ty: &Type) -> bool {
        crate::type_services::visit::type_any(ty, |nested| {
            matches!(
                nested,
                Type::TypeVar(_)
                    | Type::Generic(_)
                    | Type::Projection { .. }
                    | Type::Apply { .. }
                    | Type::Constructor { .. }
                    | Type::Lambda { .. }
                    | Type::BoundVar { .. }
                    | Type::Error
            )
        })
    }

    fn build_try_expression(
        &mut self,
        expr: HirExpr,
        branch_method: Option<HirMethodCallTarget>,
        branch_self_receiver: Option<crate::types::ReceiverMode>,
        from_residual_target: Option<HirCallTarget>,
        output_ty: Type,
        residual_ty: Type,
        return_ty: Type,
        protocol: &crate::language_items::TryLanguageItems<DefId>,
        span: Span,
    ) -> HirExpr {
        let Some(control_flow_enum_info) = self.items.enumeration(protocol.control_flow_enum_id)
        else {
            self.diagnostics.push_with_span(
                "Cannot use '?' because the ControlFlow language-item enum is unavailable"
                    .to_string(),
                span.clone(),
            );
            return self.error_expression_at(span);
        };
        let Some(break_variant_info) = control_flow_enum_info
            .variants
            .iter()
            .find(|variant| variant.id == protocol.break_variant_id)
        else {
            self.diagnostics.push_with_span(
                "Cannot use '?' because the ControlFlow language-item break variant is unavailable"
                    .to_string(),
                span.clone(),
            );
            return self.error_expression_at(span);
        };
        let Some(continue_variant_info) = control_flow_enum_info
            .variants
            .iter()
            .find(|variant| variant.id == protocol.continue_variant_id)
        else {
            self.diagnostics.push_with_span(
                "Cannot use '?' because the ControlFlow language-item continue variant is unavailable"
                    .to_string(),
                span.clone(),
            );
            return self.error_expression_at(span);
        };
        let control_flow_enum = control_flow_enum_info.id;
        let break_variant = HirVariantLocation {
            owner: control_flow_enum,
            variant_id: break_variant_info.id,
            name: break_variant_info.name.clone(),
        };
        let continue_variant = HirVariantLocation {
            owner: control_flow_enum,
            variant_id: continue_variant_info.id,
            name: continue_variant_info.name.clone(),
        };

        let carrier_ty = expr.ty.clone();
        self.record_try_trait_bound(&carrier_ty, protocol, span.clone());
        self.constraint_store.add_try(
            carrier_ty,
            output_ty.clone(),
            residual_ty.clone(),
            return_ty.clone(),
            span.clone(),
        );

        HirExpr {
            ty: output_ty.clone(),
            kind: HirExprKind::Try {
                expr: Box::new(expr),
                branch_method,
                branch_target: None,
                branch_self_receiver,
                from_residual_target,
                output_ty,
                residual_ty,
                return_ty,
                control_flow_enum,
                break_variant,
                continue_variant,
            },
            span,
        }
    }

    fn selected_associated_type(
        &self,
        selected: &crate::selection::SelectedMethod,
        trait_def: &HirTrait,
        assoc_type_id: crate::ids::AssocTypeId,
    ) -> Option<Type> {
        let assoc = trait_def
            .associated_types
            .iter()
            .find(|assoc| assoc.id == assoc_type_id)?;
        let ty = selected
            .associated_types
            .iter()
            .find(|item| item.id == assoc.id)
            .map(|item| item.ty.clone())?;
        Some(self.resolve_projection_type(&self.engine.resolve(&ty)))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::Lowerer;
    use crate::ast::{
        Expression, Ident, IdentOrNumber, IdentOrType, IdentifierPath, Literal, LiteralKind,
        Operand, PrimaryExpr, SecondaryExpr, UnaryExpr,
    };
    use crate::hir::{
        HirBlock, HirCallTarget, HirExpr, HirExprKind, HirExtern, HirField, HirFunction,
        HirFunctionSig, HirImpl, HirImplOwner, HirParam, HirStruct, HirTrait,
    };
    use crate::ids::{AssocTypeId, CrateId, DefId, FieldId, LocalDefId, VariantId};
    use crate::language_items::{IndexLanguageItems, TryLanguageItems};
    use crate::lexer::Span;
    use crate::lower::expression::ExprUse;
    use crate::types::{GenericParamDecl, GenericParamId, TraitBound, Type};

    fn test_function(id: DefId, name: &str, ret_type: Type) -> HirFunction {
        HirFunction {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: Vec::new(),
            ret_type: ret_type.clone(),
            body: HirBlock {
                stmts: Vec::new(),
                ty: ret_type,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        }
    }

    fn lowerer_with_type_var_method_bound() -> (Lowerer, Type, DefId, DefId) {
        let mut lowerer = Lowerer::new_for_test();

        let trait_id = DefId::new(CrateId(0), LocalDefId(10));
        let method_id = DefId::new(CrateId(0), LocalDefId(11));
        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Wanted".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([(
                "value".to_string(),
                test_function(method_id, "value", Type::I64),
            )]),
            signatures: HashMap::new(),
        });

        let receiver_ty = lowerer.engine.fresh_type_var();
        let Type::TypeVar(var_id) = receiver_ty else {
            panic!("expected fresh type variable");
        };
        lowerer.engine.add_bound(
            var_id,
            TraitBound {
                trait_id,
                type_args: Vec::new(),
            },
        );

        (lowerer, Type::TypeVar(var_id), trait_id, method_id)
    }

    #[test]
    fn try_with_missing_marked_trait_id_reports_protocol_unavailable() {
        let mut lowerer = Lowerer::new_for_test();
        lowerer.language_items.try_protocol = Some(TryLanguageItems {
            try_trait_id: DefId::new(CrateId(0), LocalDefId(100)),
            output_id: AssocTypeId(0),
            residual_id: AssocTypeId(1),
            branch_method_id: DefId::new(CrateId(0), LocalDefId(101)),
            from_residual_trait_id: DefId::new(CrateId(0), LocalDefId(102)),
            from_residual_method_id: DefId::new(CrateId(0), LocalDefId(103)),
            control_flow_enum_id: DefId::new(CrateId(0), LocalDefId(104)),
            break_variant_id: VariantId(0),
            continue_variant_id: VariantId(1),
        });

        let lowered = lowerer.apply_secondary(
            receiver_expr(Type::I64),
            &SecondaryExpr::Interogation,
            ExprUse::Value,
            false,
        );

        assert_eq!(lowered.ty, Type::Error);
        assert_eq!(
            lowerer.errors()[0].message,
            "Cannot use '?' because the Try language-item protocol is unavailable"
        );
    }

    fn lowerer_with_type_var_signature_bound() -> (Lowerer, Type, DefId, DefId) {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = DefId::new(CrateId(0), LocalDefId(30));
        let signature_id = DefId::new(CrateId(0), LocalDefId(31));
        let self_param = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Readable".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "read".to_string(),
                HirFunctionSig {
                    id: signature_id,
                    name: "read".to_string(),
                    generic_params: vec![GenericParamDecl::type_param(self_param, "Self")],
                    params: vec![Type::Reference {
                        mutable: false,
                        inner: Box::new(Type::Generic(self_param)),
                    }],
                    ret: Type::I64,
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(crate::types::ReceiverMode::Shared),
                    is_unsafe: false,
                },
            )]),
        });

        let receiver_ty = lowerer.engine.fresh_type_var();
        let Type::TypeVar(var_id) = receiver_ty else {
            panic!("expected fresh type variable");
        };
        lowerer.engine.add_bound(
            var_id,
            TraitBound {
                trait_id,
                type_args: Vec::new(),
            },
        );

        (lowerer, Type::TypeVar(var_id), trait_id, signature_id)
    }

    fn lowerer_with_inherent_method() -> (Lowerer, DefId, DefId) {
        let mut lowerer = Lowerer::new_for_test();
        let struct_id = DefId::new(CrateId(0), LocalDefId(10));
        let impl_id = DefId::new(CrateId(0), LocalDefId(20));
        let method_id = DefId::new(CrateId(0), LocalDefId(21));
        lowerer.items.insert_structure(HirStruct {
            id: struct_id,
            name: "Box".to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        });
        lowerer
            .resolver
            .item_names_by_id
            .insert(struct_id, "Box".to_string());
        let mut method = test_function(method_id, "value", Type::I64);
        method.is_method = true;
        method.self_receiver = Some(crate::types::ReceiverMode::Shared);
        method.params.push(HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Struct {
                id: struct_id,
                args: Vec::new(),
            },
            mutable: false,
            is_ref: false,
        });

        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                    id: struct_id,
                    args: Vec::new(),
                }),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("value".to_string(), method)]),
            })
            .unwrap();
        (lowerer, impl_id, method_id)
    }

    fn lowerer_with_curried_inherent_method() -> (Lowerer, DefId, DefId) {
        let (mut lowerer, impl_id, method_id) = lowerer_with_inherent_method();
        let method = lowerer
            .items
            .impl_def_mut(impl_id)
            .and_then(|imp| imp.methods.get_mut("value"))
            .expect("test helper should create value method");
        method.is_curried = true;
        method.params.push(HirParam {
            name: "amount".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::I64,
            mutable: false,
            is_ref: false,
        });
        method.ret_type = Type::function(vec![Type::I64], Type::I64);

        (lowerer, impl_id, method_id)
    }

    fn receiver_expr(ty: Type) -> HirExpr {
        HirExpr {
            kind: HirExprKind::Var("receiver".to_string()),
            ty,
            span: Span::test(),
        }
    }

    fn number_expression(value: u64) -> Expression {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Literal(Literal {
                kind: LiteralKind::Number(value),
                span: Span::test(),
            }),
            secondaries: None,
            type_annotation: None,
        }))
    }

    fn ident_expression(name: &str) -> Expression {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Ident(IdentifierPath {
                path: vec![IdentOrType::Ident(Ident {
                    name: name.to_string(),
                    span: Span::test(),
                })],
            }),
            secondaries: None,
            type_annotation: None,
        }))
    }

    fn call_expr(name: &str, args: Vec<Expression>) -> Expression {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Ident(IdentifierPath {
                path: vec![IdentOrType::Ident(Ident {
                    name: name.to_string(),
                    span: Span::test(),
                })],
            }),
            secondaries: Some(vec![SecondaryExpr::Arguments(
                args.into_iter()
                    .map(|arg| crate::ast::Argument { arg })
                    .collect(),
            )]),
            type_annotation: None,
        }))
    }

    #[test]
    fn direct_function_call_records_call_target() {
        let mut lowerer = Lowerer::new_for_test();
        let function_id = DefId::new(CrateId(0), LocalDefId(88));
        lowerer
            .items
            .insert_function(test_function(function_id, "answer", Type::I64));
        lowerer
            .resolver
            .item_paths
            .insert("answer".to_string(), function_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(function_id, "answer".to_string());
        lowerer.scope.define_alias(
            "answer".to_string(),
            Type::function(Vec::new(), Type::I64),
            false,
        );

        let expr = call_expr("answer", Vec::new());
        let hir = lowerer.lower_expression(&expr);

        match hir.kind {
            HirExprKind::Call(_, _, Some(HirCallTarget::Function(id))) => {
                assert_eq!(id, function_id);
            }
            other => panic!("expected function call target, got {other:?}"),
        }
    }

    #[test]
    fn direct_function_lookup_does_not_resolve_method_id() {
        let (lowerer, _impl_id, method_id) = lowerer_with_inherent_method();

        assert!(lowerer.current_function(method_id).is_none());
    }

    #[test]
    fn direct_extern_call_records_call_target() {
        let mut lowerer = Lowerer::new_for_test();
        let extern_id = DefId::new(CrateId(0), LocalDefId(89));
        lowerer.items.insert_extern(HirExtern {
            id: extern_id,
            name: "puts".to_string(),
            params: vec![Type::I32],
            ret: Type::I32,
            variadic: false,
            is_unsafe: false,
        });
        lowerer
            .resolver
            .item_paths
            .insert("puts".to_string(), extern_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(extern_id, "puts".to_string());
        lowerer.scope.define_alias(
            "puts".to_string(),
            Type::function(vec![Type::I32], Type::I32),
            false,
        );

        let expr = call_expr("puts", vec![number_expression(0)]);
        let hir = lowerer.lower_expression(&expr);

        match hir.kind {
            HirExprKind::Call(_, _, Some(HirCallTarget::Extern(id))) => assert_eq!(id, extern_id),
            other => panic!("expected extern call target, got {other:?}"),
        }
    }

    #[test]
    fn local_function_value_call_records_call_target() {
        let mut lowerer = Lowerer::new_for_test();
        let expr = call_expr("callback", Vec::new());
        let (local_id, hir) = lowerer.with_test_body_context(|lowerer| {
            let local_id = lowerer.fresh_local_id();
            lowerer.scope.define_local(
                "callback".to_string(),
                Type::function(Vec::new(), Type::I64),
                false,
                local_id,
            );
            (local_id, lowerer.lower_expression(&expr))
        });

        match hir.kind {
            HirExprKind::Call(_, _, Some(HirCallTarget::Local(id))) => assert_eq!(id, local_id),
            other => panic!("expected local function-value call target, got {other:?}"),
        }
    }

    #[test]
    fn arity_error_direct_call_records_call_target() {
        let mut lowerer = Lowerer::new_for_test();
        let function_id = DefId::new(CrateId(0), LocalDefId(91));
        lowerer
            .items
            .insert_function(test_function(function_id, "pair", Type::I64));
        lowerer
            .resolver
            .item_paths
            .insert("pair".to_string(), function_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(function_id, "pair".to_string());
        lowerer.scope.define_alias(
            "pair".to_string(),
            Type::function(vec![Type::I64, Type::I64], Type::I64),
            false,
        );

        let expr = call_expr("pair", vec![number_expression(1)]);
        let hir = lowerer.lower_expression(&expr);

        assert_eq!(hir.ty, Type::Error);
        match hir.kind {
            HirExprKind::Call(_, _, Some(HirCallTarget::Function(id))) => {
                assert_eq!(id, function_id);
            }
            other => panic!("expected arity-error call target, got {other:?}"),
        }
    }

    #[test]
    fn unsupported_index_produces_error_expression() {
        let mut lowerer = Lowerer::new_for_test();
        let source_span = Span {
            file_path: "main.rk".into(),
            start: 10,
            end: 15,
        };
        let mut receiver = receiver_expr(Type::Str);
        receiver.span = source_span.clone();
        let lowered = lowerer.apply_secondary(
            receiver,
            &SecondaryExpr::Indice(Box::new(number_expression(0))),
            ExprUse::Value,
            false,
        );

        assert_eq!(lowered.ty, Type::Error);
        assert_eq!(lowered.span.file_path, source_span.file_path);
        assert_eq!(lowered.span.start, source_span.start);
        assert_eq!(lowered.span.end, source_span.end);
        assert!(matches!(
            lowered.kind,
            HirExprKind::Var(ref name) if name == "<error>"
        ));
        assert!(!lowerer.errors().is_empty());
        let diagnostic_origin = lowerer.errors()[0].span();
        let diagnostic_span = diagnostic_origin
            .as_ref()
            .expect("unsupported index diagnostic should have a span");
        assert_eq!(diagnostic_span.file_path, source_span.file_path);
        assert_eq!(diagnostic_span.start, source_span.start);
        assert_eq!(diagnostic_span.end, source_span.end);
    }

    #[test]
    fn accepted_index_is_selected_method_call() {
        let mut lowerer = Lowerer::new_for_test();
        let bag_id = DefId::new(CrateId(0), LocalDefId(50));
        let trait_id = DefId::new(CrateId(1), LocalDefId(150));
        let impl_id = DefId::new(CrateId(0), LocalDefId(52));
        let method_id = DefId::new(CrateId(0), LocalDefId(53));
        let generic = GenericParamId {
            owner: impl_id,
            index: 0,
        };
        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Index".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "Idx",
            )],
            associated_types: vec![crate::hir::HirAssociatedTypeDecl {
                id: AssocTypeId(0),
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
            }],
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "index".to_string(),
                HirFunctionSig {
                    id: method_id,
                    name: "index".to_string(),
                    generic_params: Vec::new(),
                    params: vec![
                        Type::TypeVar(crate::ids::TypeVarId(0)),
                        Type::Generic(generic),
                    ],
                    ret: Type::Reference {
                        mutable: false,
                        inner: Box::new(Type::Generic(generic)),
                    },
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(crate::types::ReceiverMode::Shared),
                    is_unsafe: false,
                },
            )]),
        });
        lowerer.language_items.index = Some(IndexLanguageItems {
            trait_id,
            output_id: AssocTypeId(0),
            method_id,
        });
        lowerer
            .resolver
            .item_names_by_id
            .insert(bag_id, "Bag".to_string());
        let mut method = test_function(
            method_id,
            "index",
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::Generic(generic)),
            },
        );
        method.is_method = true;
        method.self_receiver = Some(crate::types::ReceiverMode::Shared);
        method.params.push(HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::TypeVar(crate::ids::TypeVarId(0)),
            mutable: false,
            is_ref: false,
        });
        method.params.push(HirParam {
            name: "index".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Generic(generic),
            mutable: false,
            is_ref: false,
        });
        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Bag".to_string()),
                type_name: "Bag".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                    id: bag_id,
                    args: Vec::new(),
                }),
                trait_name: Some("Index".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![GenericParamDecl::type_param(generic, "T")],
                trait_arg_types: vec![Type::Generic(generic)],
                associated_types: vec![crate::hir::HirAssociatedTypeDef {
                    id: AssocTypeId(0),
                    name: "Output".to_string(),
                    kind: crate::type_services::kind::Kind::Type,
                    ty: Type::I64,
                }],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("index".to_string(), method)]),
            })
            .unwrap();
        lowerer
            .imported_effective_trait_methods
            .insert((impl_id, method_id), method_id);
        let receiver = receiver_expr(Type::Struct {
            id: bag_id,
            args: Vec::new(),
        });
        lowerer.scope.define("index".to_string(), Type::I64, false);

        let lowered = lowerer.apply_secondary(
            receiver,
            &SecondaryExpr::Indice(Box::new(ident_expression("index"))),
            ExprUse::Value,
            false,
        );

        assert_eq!(lowered.ty, Type::I64, "errors: {:?}", lowerer.errors());
        assert!(
            lowerer.errors().is_empty(),
            "errors: {:?}",
            lowerer.errors()
        );
        match lowered.kind {
            HirExprKind::Deref(method_call) => match method_call.kind {
                HirExprKind::MethodCall(_, name, args, _, Some(target)) => {
                    assert_eq!(name, "index");
                    assert_eq!(args.len(), 1);
                    assert_eq!(target.impl_id(), Some(impl_id));
                    assert_eq!(target.trait_id(), Some(trait_id));
                    assert_eq!(target.method_id(), Some(method_id));
                }
                other => panic!("expected selected index method call, got {other:?}"),
            },
            other => panic!("expected dereference of selected index method call, got {other:?}"),
        }
    }

    #[test]
    fn type_var_method_call_uses_trait_bound_target_before_name_candidates() {
        let (mut lowerer, receiver_ty, trait_id, method_id) = lowerer_with_type_var_method_bound();
        let field = HirExpr {
            kind: HirExprKind::FieldAccess(
                Box::new(receiver_expr(receiver_ty)),
                "value".to_string(),
                None,
            ),
            ty: lowerer.engine.fresh_type_var(),
            span: Span::test(),
        };

        let lowered = lowerer.apply_secondary(
            field,
            &SecondaryExpr::Arguments(Vec::new()),
            ExprUse::Value,
            false,
        );

        assert!(matches!(lowered.ty, Type::I64));
        match lowered.kind {
            HirExprKind::MethodCall(_, _, _, _, Some(target)) => {
                assert_eq!(target.trait_id(), Some(trait_id));
                assert_eq!(target.method_id(), Some(method_id));
            }
            other => panic!("expected targeted method call, got {other:?}"),
        }
    }

    #[test]
    fn type_var_without_bounds_does_not_select_unowned_method() {
        let mut lowerer = Lowerer::new_for_test();
        let receiver_ty = lowerer.engine.fresh_type_var();
        let field = HirExpr {
            kind: HirExprKind::FieldAccess(
                Box::new(receiver_expr(receiver_ty)),
                "value".to_string(),
                None,
            ),
            ty: lowerer.engine.fresh_type_var(),
            span: Span::test(),
        };

        let lowered = lowerer.apply_secondary(
            field,
            &SecondaryExpr::Arguments(Vec::new()),
            ExprUse::Value,
            false,
        );

        assert!(
            !matches!(lowered.kind, HirExprKind::MethodCall(..)),
            "unconstrained type variable used an unowned method candidate: {:?}",
            lowered.kind
        );
    }

    #[test]
    fn concrete_receiver_does_not_select_method_without_impl_identity() {
        let mut lowerer = Lowerer::new_for_test();
        let struct_id = DefId::new(CrateId(0), LocalDefId(10));
        lowerer.items.insert_structure(HirStruct {
            id: struct_id,
            name: "Box".to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        });
        let field = HirExpr {
            kind: HirExprKind::FieldAccess(
                Box::new(receiver_expr(Type::Struct {
                    id: struct_id,
                    args: Vec::new(),
                })),
                "value".to_string(),
                None,
            ),
            ty: lowerer.engine.fresh_type_var(),
            span: Span::test(),
        };

        let lowered = lowerer.apply_secondary(
            field,
            &SecondaryExpr::Arguments(Vec::new()),
            ExprUse::Value,
            false,
        );

        assert!(
            !matches!(lowered.kind, HirExprKind::MethodCall(..)),
            "concrete receiver used a method without impl identity: {:?}",
            lowered.kind
        );
    }

    #[test]
    fn type_var_signature_call_uses_signature_id_target() {
        let (mut lowerer, receiver_ty, trait_id, signature_id) =
            lowerer_with_type_var_signature_bound();
        let field = HirExpr {
            kind: HirExprKind::FieldAccess(
                Box::new(receiver_expr(receiver_ty)),
                "read".to_string(),
                None,
            ),
            ty: lowerer.engine.fresh_type_var(),
            span: Span::test(),
        };

        let lowered = lowerer.apply_secondary(
            field,
            &SecondaryExpr::Arguments(Vec::new()),
            ExprUse::Value,
            false,
        );

        match lowered.kind {
            HirExprKind::MethodCall(_, _, _, _, Some(target)) => {
                assert_eq!(target.trait_id(), Some(trait_id));
                assert_eq!(target.method_id(), Some(signature_id));
                assert_ne!(target.method_id(), Some(trait_id));
            }
            other => panic!("expected signature-backed targeted method call, got {other:?}"),
        }
    }

    #[test]
    fn current_trait_self_signature_call_uses_signature_return_type() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = DefId::new(CrateId(0), LocalDefId(40));
        let signature_id = DefId::new(CrateId(0), LocalDefId(41));
        let invalid_signature_id = DefId::new(CrateId(u32::MAX), LocalDefId(u32::MAX - 1));
        let string_id = DefId::new(CrateId(0), LocalDefId(42));
        let self_param = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let string_ty = Type::Struct {
            id: string_id,
            args: Vec::new(),
        };
        lowerer.current_trait = Some("Show".to_string());
        lowerer.current_trait_id = Some(trait_id);
        lowerer.generic_context = Some(crate::lower::body_context::GenericLoweringContext::new(
            trait_id,
            vec!["Self".to_string()],
        ));
        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Show".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "show".to_string(),
                HirFunctionSig {
                    id: invalid_signature_id,
                    name: "show".to_string(),
                    generic_params: Vec::new(),
                    params: vec![Type::Generic(self_param)],
                    ret: string_ty.clone(),
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(crate::types::ReceiverMode::Shared),
                    is_unsafe: false,
                },
            )]),
        });
        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Show".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "show".to_string(),
                HirFunctionSig {
                    id: signature_id,
                    name: "show".to_string(),
                    generic_params: Vec::new(),
                    params: vec![Type::Generic(self_param)],
                    ret: string_ty.clone(),
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(crate::types::ReceiverMode::Shared),
                    is_unsafe: false,
                },
            )]),
        });
        let field = HirExpr {
            kind: HirExprKind::FieldAccess(
                Box::new(receiver_expr(Type::Generic(self_param))),
                "show".to_string(),
                None,
            ),
            ty: lowerer.engine.fresh_type_var(),
            span: Span::test(),
        };

        let lowered = lowerer.apply_secondary(
            field,
            &SecondaryExpr::Arguments(Vec::new()),
            ExprUse::Value,
            false,
        );

        assert_eq!(lowered.ty, string_ty);
        match lowered.kind {
            HirExprKind::MethodCall(_, _, _, _, Some(target)) => {
                assert_eq!(target.trait_id(), Some(trait_id));
                assert_eq!(target.method_id(), Some(signature_id));
            }
            other => panic!("expected current-trait signature method call, got {other:?}"),
        }
    }

    #[test]
    fn type_var_method_value_uses_trait_bound_target_before_name_candidates() {
        let (mut lowerer, receiver_ty, trait_id, method_id) = lowerer_with_type_var_method_bound();

        let lowered = lowerer.apply_secondary(
            receiver_expr(receiver_ty),
            &SecondaryExpr::Dot(IdentOrNumber::Ident(Ident {
                name: "value".to_string(),
                span: Span::test(),
            })),
            ExprUse::Value,
            true,
        );

        assert!(matches!(lowered.ty, Type::Function { ret, .. } if matches!(*ret, Type::I64)));
        match lowered.kind {
            HirExprKind::Lambda { body, .. } => match body.stmts.as_slice() {
                [crate::hir::HirStmt::Expr(HirExpr {
                    kind: HirExprKind::MethodCall(_, _, _, _, Some(target)),
                    ..
                })] => {
                    assert_eq!(target.trait_id(), Some(trait_id));
                    assert_eq!(target.method_id(), Some(method_id));
                }
                other => panic!("expected targeted method call body, got {other:?}"),
            },
            other => panic!("expected method-value lambda, got {other:?}"),
        }
    }

    #[test]
    fn inherent_impl_method_call_carries_exact_target() {
        let (mut lowerer, impl_id, method_id) = lowerer_with_inherent_method();
        let receiver = receiver_expr(Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(10)),
            args: Vec::new(),
        });
        let field = HirExpr {
            kind: HirExprKind::FieldAccess(Box::new(receiver), "value".to_string(), None),
            ty: lowerer.engine.fresh_type_var(),
            span: Span::test(),
        };

        let lowered = lowerer.apply_secondary(
            field,
            &SecondaryExpr::Arguments(Vec::new()),
            ExprUse::Value,
            false,
        );

        match lowered.kind {
            HirExprKind::MethodCall(_, _, _, _, Some(target)) => {
                assert_eq!(target.impl_id(), Some(impl_id));
                assert_eq!(target.trait_id(), None);
                assert!(target.trait_args().is_empty());
                assert_eq!(target.method_id(), Some(method_id));
            }
            other => panic!("expected exact inherent method target, got {other:?}"),
        }
    }

    #[test]
    fn empty_bang_rejects_method_with_explicit_params() {
        let (mut lowerer, impl_id, _) = lowerer_with_inherent_method();
        {
            let method = lowerer
                .items
                .impl_def_mut(impl_id)
                .and_then(|imp| imp.methods.get_mut("value"))
                .expect("test helper should create value method");
            method.params.push(HirParam {
                name: "amount".to_string(),
                local_id: crate::ids::HirLocalId(1),
                ty: Type::I64,
                mutable: false,
                is_ref: false,
            });
            method.ret_type = Type::Unit;
            method.body.ty = Type::Unit;
        }
        let receiver = receiver_expr(Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(10)),
            args: Vec::new(),
        });
        assert!(
            lowerer
                .concrete_method_candidate(receiver.clone(), "value")
                .is_some(),
            "fixture should select the canonical inherent method"
        );
        let method_field = lowerer.apply_secondary(
            receiver,
            &SecondaryExpr::Dot(IdentOrNumber::Ident(Ident {
                name: "value".to_string(),
                span: Span::test(),
            })),
            ExprUse::Value,
            false,
        );
        let lowered = lowerer.apply_secondary(
            method_field,
            &SecondaryExpr::Arguments(Vec::new()),
            ExprUse::Value,
            false,
        );

        assert_eq!(lowered.ty, Type::Error);
        assert!(lowerer.diagnostics.errors().iter().any(|error| {
            error.message == "Type mismatch: method 'value' expected 1 args, got 0"
        }));
    }

    #[test]
    fn inherent_impl_method_value_carries_exact_target() {
        let (mut lowerer, impl_id, method_id) = lowerer_with_inherent_method();

        let lowered = lowerer.apply_secondary(
            receiver_expr(Type::Struct {
                id: DefId::new(CrateId(0), LocalDefId(10)),
                args: Vec::new(),
            }),
            &SecondaryExpr::Dot(IdentOrNumber::Ident(Ident {
                name: "value".to_string(),
                span: Span::test(),
            })),
            ExprUse::Value,
            true,
        );

        match lowered.kind {
            HirExprKind::Lambda { body, .. } => match body.stmts.as_slice() {
                [crate::hir::HirStmt::Expr(HirExpr {
                    kind: HirExprKind::MethodCall(_, _, _, _, Some(target)),
                    ..
                })] => {
                    assert_eq!(target.impl_id(), Some(impl_id));
                    assert_eq!(target.trait_id(), None);
                    assert_eq!(target.method_id(), Some(method_id));
                }
                other => panic!("expected targeted method-value body, got {other:?}"),
            },
            other => panic!("expected method-value lambda, got {other:?}"),
        }
    }

    #[test]
    fn curried_inherent_method_partial_application_carries_exact_target() {
        let (mut lowerer, impl_id, method_id) = lowerer_with_curried_inherent_method();
        let receiver = receiver_expr(Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(10)),
            args: Vec::new(),
        });
        let field = HirExpr {
            kind: HirExprKind::FieldAccess(Box::new(receiver), "value".to_string(), None),
            ty: lowerer.engine.fresh_type_var(),
            span: Span::test(),
        };
        let lowered = lowerer.apply_secondary(
            field,
            &SecondaryExpr::Arguments(vec![crate::ast::Argument {
                arg: crate::ast::Expression::UnaryExpr(crate::ast::UnaryExpr::PrimaryExpr(
                    crate::ast::PrimaryExpr {
                        operand: crate::ast::Operand::Literal(crate::ast::Literal {
                            kind: crate::ast::LiteralKind::Number(1),
                            span: Span::test(),
                        }),
                        secondaries: None,
                        type_annotation: None,
                    },
                )),
            }]),
            ExprUse::Value,
            false,
        );

        match lowered.kind {
            HirExprKind::MethodCall(_, _, args, _, Some(target)) => {
                assert_eq!(args.len(), 1);
                assert_eq!(target.impl_id(), Some(impl_id));
                assert_eq!(target.trait_id(), None);
                assert_eq!(target.method_id(), Some(method_id));
            }
            other => panic!("expected curried targeted method call, got {other:?}"),
        }
    }

    #[test]
    fn type_var_field_access_uses_canonical_struct_name() {
        let mut lowerer = Lowerer::new_for_test();
        let point_id = DefId::new(CrateId(0), LocalDefId(1));
        lowerer.current_def_ids.insert(point_id);
        lowerer.items.insert_structure(HirStruct {
            id: point_id,
            name: "Point".to_string(),
            generic_params: Vec::new(),
            fields: vec![HirField {
                id: FieldId(0),
                name: "x".to_string(),
                ty: Type::I64,
                public: true,
            }],
        });
        lowerer
            .resolver
            .item_names_by_id
            .insert(point_id, "Point".to_string());
        lowerer
            .resolver
            .item_paths
            .insert("Point".to_string(), point_id);

        let base = HirExpr {
            kind: HirExprKind::Var("point".to_string()),
            ty: lowerer.engine.fresh_type_var(),
            span: Span::test(),
        };
        let lowered = lowerer.apply_secondary(
            base,
            &SecondaryExpr::Dot(IdentOrNumber::Ident(Ident {
                name: "x".to_string(),
                span: Span::test(),
            })),
            ExprUse::Value,
            false,
        );

        assert_eq!(lowered.ty, Type::I64);
        assert!(lowerer.errors().is_empty());
    }

    #[test]
    fn type_var_field_access_remains_unresolved_when_multiple_structs_match() {
        let mut lowerer = Lowerer::new_for_test();
        lowerer.items.insert_structure(HirStruct {
            id: DefId::new(CrateId(0), LocalDefId(1)),
            name: "First".to_string(),
            generic_params: Vec::new(),
            fields: vec![HirField {
                id: FieldId(0),
                name: "value".to_string(),
                ty: Type::I64,
                public: true,
            }],
        });
        lowerer.items.insert_structure(HirStruct {
            id: DefId::new(CrateId(0), LocalDefId(2)),
            name: "Second".to_string(),
            generic_params: Vec::new(),
            fields: vec![HirField {
                id: FieldId(0),
                name: "value".to_string(),
                ty: Type::I64,
                public: true,
            }],
        });

        let base = HirExpr {
            kind: HirExprKind::Var("receiver".to_string()),
            ty: lowerer.engine.fresh_type_var(),
            span: Span::test(),
        };
        let lowered = lowerer.apply_secondary(
            base,
            &SecondaryExpr::Dot(IdentOrNumber::Ident(Ident {
                name: "value".to_string(),
                span: Span::test(),
            })),
            ExprUse::Value,
            false,
        );

        assert!(matches!(lowered.ty, Type::TypeVar(_)));
        assert!(lowerer.errors().is_empty());
        match lowered.kind {
            HirExprKind::FieldAccess(_, _, None) => {}
            other => panic!("expected resolved field access, got {other:?}"),
        }
    }

    #[test]
    fn test_borrowed_slice_type_name_match_rejects_concrete_element_mismatch() {
        assert!(!Lowerer::receiver_type_name_matches("&[U8]", "&[I64]"));
        assert!(!Lowerer::receiver_type_name_matches(
            "&mut [U8]",
            "&mut [I64]"
        ));
    }

    #[test]
    fn test_borrowed_slice_type_name_match_allows_generic_candidate_shape() {
        assert!(Lowerer::receiver_type_name_matches("&[T]", "&[U8]"));
        assert!(Lowerer::receiver_type_name_matches("&mut [T]", "&mut [U8]"));
    }
}

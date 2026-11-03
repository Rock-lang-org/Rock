//! Callable argument lowering and coercion.

use crate::ast;
use crate::hir::*;
use crate::lower::intrinsics::is_intrinsic_name;
use crate::lower::Lowerer;
use crate::types::{TraitBound, Type};

impl Lowerer {
    pub(super) fn lambda_from_expression(expr: &ast::Expression) -> Option<&ast::LambdaDecl> {
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

    pub(super) fn lower_selected_method_argument(
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

    pub(super) fn callable_argument_parameters(&self, expected: &Type) -> Option<Vec<Type>> {
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

    pub(super) fn propagate_call_argument_bounds(&mut self, expected: &Type, actual: &Type) {
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

    pub(crate) fn coerce_argument_to_expected(
        &mut self,
        mut arg: HirExpr,
        expected_ty: &Type,
    ) -> HirExpr {
        let resolved_expected = self.resolve_projection_type(&self.engine.resolve(expected_ty));
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
}

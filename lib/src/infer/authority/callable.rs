use crate::hir::{HirExpr, HirExprKind};
use crate::lower::ResolveError;
use crate::types::{FunctionSafety, Type};

use super::MethodAuthorityContext;

impl MethodAuthorityContext<'_> {
    pub(super) fn materialize_method_bindings(
        &self,
        args: &[HirExpr],
        result: &Type,
        target: &mut crate::hir::HirMethodCallTarget,
        span: &crate::lexer::Span,
        errors: &mut Vec<ResolveError>,
    ) {
        let Some(impl_id) = target.impl_id() else {
            return;
        };
        let Some(function) = self.impls.get(&impl_id).and_then(|imp| {
            imp.methods
                .values()
                .find(|method| Some(method.id) == target.method_id())
        }) else {
            return;
        };
        let owner_params = target
            .owner_substitution
            .iter()
            .map(|binding| binding.param)
            .collect::<std::collections::HashSet<_>>();
        let required = function
            .generic_params
            .iter()
            .map(|param| param.id)
            .filter(|id| !owner_params.contains(id) && Some(id.owner) != target.trait_id())
            .collect::<Vec<_>>();
        if required.iter().all(|id| {
            target
                .method_substitution
                .iter()
                .any(|binding| binding.param == *id)
        }) {
            return;
        }
        let mut substitution = target
            .owner_substitution
            .iter()
            .chain(&target.method_substitution)
            .map(|binding| (binding.param, self.resolved_type(&binding.ty)))
            .collect::<std::collections::HashMap<_, _>>();
        for (param, arg) in function
            .params
            .iter()
            .skip(usize::from(function.is_method))
            .zip(args)
        {
            crate::selection::type_pattern_matches(
                &param.ty,
                &self.resolved_type(&arg.ty),
                &mut substitution,
            );
        }
        crate::selection::type_pattern_matches(
            &function.ret_type,
            &self.resolved_type(result),
            &mut substitution,
        );
        for (subject, bounds) in &function.generic_bounds {
            let Some(actual) = substitution.get(subject).cloned() else {
                continue;
            };
            let mut native = &actual;
            while let Type::Reference { inner, .. } = native {
                native = inner;
            }
            if let Type::Function { params, ret, .. } = native {
                let args = match params.as_slice() {
                    [] => Type::Unit,
                    [ty] => ty.clone(),
                    params => Type::Tuple(params.to_vec()),
                };
                for bound in bounds {
                    if bound.type_args.len() == 2
                        && self
                            .language_items
                            .callable_protocols()
                            .any(|(_, id, _)| id == bound.trait_id)
                    {
                        crate::selection::type_pattern_matches(
                            &bound.type_args[0],
                            &args,
                            &mut substitution,
                        );
                        crate::selection::type_pattern_matches(
                            &bound.type_args[1],
                            ret,
                            &mut substitution,
                        );
                    }
                }
            }
        }
        for param in required {
            if target
                .method_substitution
                .iter()
                .any(|binding| binding.param == param)
            {
                continue;
            }
            if let Some(ty) = substitution.get(&param) {
                target.method_substitution.push(crate::hir::HirTypeBinding {
                    param,
                    ty: ty.clone(),
                });
            } else if self.strict {
                errors.push(ResolveError::with_span(
                    "cannot infer a generalized method type argument".into(),
                    span.clone(),
                ));
            }
        }
        target
            .method_substitution
            .sort_by_key(super::binding_sort_key);
    }

    pub(super) fn materialize_value_call(
        &mut self,
        expr: &mut HirExpr,
        errors: &mut Vec<ResolveError>,
    ) {
        let HirExprKind::Call(callee, args, _) = &mut expr.kind else {
            return;
        };
        let mut ty = self.resolved_type(&callee.ty);
        if matches!(ty, Type::TypeVar(_)) {
            if !self.strict {
                return;
            }
            let signature = Type::function(
                args.iter().map(|arg| arg.ty.clone()).collect(),
                expr.ty.clone(),
            );
            if self.engine.borrow_mut().unify(&ty, &signature).is_err() {
                return;
            }
            ty = self.resolved_type(&callee.ty);
        }
        let mut native_ty = &ty;
        while let Type::Reference { inner, .. } = native_ty {
            native_ty = inner;
        }
        if let Type::Function {
            params,
            ret,
            safety,
            ..
        } = native_ty
        {
            if params.len() != args.len() {
                errors.push(ResolveError::with_span(
                    format!(
                        "function expects {} arguments but received {}",
                        params.len(),
                        args.len()
                    ),
                    expr.span.clone(),
                ));
                return;
            }
            let mut native_callee = (**callee).clone();
            native_callee.ty = native_ty.clone();
            self.materialize_callable_arguments(&native_callee, args, errors);
            let check = self.engine.borrow_mut().unify(ret, &expr.ty);
            if let Err(error) = check {
                errors.push(ResolveError::with_span(
                    error.render(&self.engine.borrow()),
                    expr.span.clone(),
                ));
            }
            if *safety == FunctionSafety::Unsafe && !self.unsafe_context.get() {
                errors.push(ResolveError::with_span(
                    "Call to unsafe function value requires an unsafe block".into(),
                    expr.span.clone(),
                ));
            }
            callee.ty = ty;
            return;
        }
        let mut candidates = self.receiver_adjustment_candidates((**callee).clone());
        for candidate in &mut candidates {
            candidate.expr.ty = self.resolved_type(&candidate.expr.ty);
        }
        let selection = self.service().select_callable_method(
            &candidates,
            &self.language_items,
            |ty| match ty {
                Type::TypeVar(id) => self.engine.borrow().get_bounds(*id),
                Type::Generic(id) => self.bounds.get(id).cloned().unwrap_or_default(),
                _ => Vec::new(),
            },
            None,
        );
        let selected = match selection {
            Ok(Some(selected)) => selected,
            Ok(None) => {
                if self.strict {
                    errors.push(ResolveError::with_span(
                        format!("value of type {} is not callable", self.display_type(&ty)),
                        expr.span.clone(),
                    ));
                }
                return;
            }
            Err(error) => {
                if self.strict {
                    errors.push(ResolveError::with_span(
                        self.display_selection_error(&error),
                        expr.span.clone(),
                    ));
                }
                return;
            }
        };
        let packed = match args.as_slice() {
            [] => HirExpr {
                ty: Type::Unit,
                kind: HirExprKind::Unit,
                span: expr.span.clone(),
            },
            [arg] => arg.clone(),
            args => HirExpr {
                ty: Type::Tuple(args.iter().map(|arg| arg.ty.clone()).collect()),
                kind: HirExprKind::TupleLiteral(args.to_vec()),
                span: expr.span.clone(),
            },
        };
        let Some(function) = &selected.function else {
            return;
        };
        if function.is_unsafe && !self.unsafe_context.get() {
            errors.push(ResolveError::with_span(
                "Call to unsafe callable requires an unsafe block".into(),
                expr.span.clone(),
            ));
            return;
        }
        let mut subst = selected.owner_substitution.clone();
        for param in &function.generic_params {
            subst.entry(param.id).or_insert_with(|| {
                self.engine
                    .borrow_mut()
                    .fresh_type_var_at_kind(expr.span.clone(), param.kind.clone())
            });
        }
        let Some(param) = selected.substituted_params.first() else {
            return;
        };
        let expected = param.ty.substitute_generics(&subst);
        let argument_result = self.engine.borrow_mut().unify(&expected, &packed.ty);
        if let Err(error) = argument_result {
            errors.push(ResolveError::with_span(
                error.render(&self.engine.borrow()),
                packed.span.clone(),
            ));
            return;
        }
        let result = selected.return_type.substitute_generics(&subst);
        let result_check = self.engine.borrow_mut().unify(&result, &expr.ty);
        if let Err(error) = result_check {
            errors.push(ResolveError::with_span(
                error.render(&self.engine.borrow()),
                expr.span.clone(),
            ));
            return;
        }
        self.record_selected_bounds(&selected, &subst, "function call");
        let target = selected.target_with_substitution(&subst, |ty| self.resolved_type(ty));
        let receiver =
            self.apply_receiver_adjustment(selected.receiver.clone(), selected.receiver_adjustment);
        expr.kind = HirExprKind::MethodCall(
            Box::new(receiver),
            function.name.clone(),
            vec![packed],
            function.self_receiver,
            Some(target),
        );
        expr.ty = self.resolved_type(&result);
    }
}

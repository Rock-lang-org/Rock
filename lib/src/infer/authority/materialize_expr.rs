use crate::hir::{HirExpr, HirExprKind, HirTypeBinding};
use crate::lower::ResolveError;
use crate::selection::{type_pattern_matches, ReceiverAdjustment};
use crate::types::{FunctionSafety, Type};

use super::type_predicates::{contains_inference_type, contains_recovery_type};
use super::{binding_sort_key, native_call_type, AuthorityObligationKind, MethodAuthorityContext};

impl MethodAuthorityContext<'_> {
    pub(super) fn materialize_expr(&mut self, expr: &mut HirExpr, errors: &mut Vec<ResolveError>) {
        if matches!(&expr.kind, HirExprKind::Call(callee, _, _) if !native_call_type(&callee.ty) && !matches!(callee.kind, HirExprKind::FieldAccess(_, _, None)))
            && (self.visit_authority_slot(AuthorityObligationKind::CallableCall, &expr.span)
                || self.visit_authority_slot(AuthorityObligationKind::DeferredCall, &expr.span))
        {
            self.materialize_value_call(expr, errors);
        }
        match &mut expr.kind {
            HirExprKind::Call(callee, args, target) => {
                let deferred_member = matches!(callee.kind, HirExprKind::FieldAccess(_, _, _));
                let selected_site = deferred_member.then(|| {
                    self.visit_authority_slot(AuthorityObligationKind::DeferredCall, &expr.span)
                });
                if target.is_some() {
                    if deferred_member {
                        if let HirExprKind::FieldAccess(receiver, _, _) = &mut callee.kind {
                            self.materialize_expr(receiver, errors);
                        }
                    } else {
                        self.materialize_expr(callee, errors);
                    }
                    for arg in args.iter_mut() {
                        self.materialize_expr(arg, errors);
                    }
                    self.materialize_callable_arguments(callee, args, errors);
                    if let Some(crate::hir::HirCallTarget::Function(function_id)) = target.as_ref()
                    {
                        self.materialize_direct_function_call(&expr.ty, callee, args, *function_id);
                    }
                    return;
                }
                let (mut receiver, method_name) = match &callee.kind {
                    HirExprKind::FieldAccess(receiver, method_name, None) => {
                        (receiver.as_ref().clone(), method_name.clone())
                    }
                    _ => {
                        self.materialize_expr(callee, errors);
                        for arg in args.iter_mut() {
                            self.materialize_expr(arg, errors);
                        }
                        self.materialize_callable_arguments(callee, args, errors);
                        if matches!(callee.kind, HirExprKind::FieldAccess(_, _, Some(_))) {
                            self.materialize_value_call(expr, errors);
                        }
                        return;
                    }
                };
                if selected_site == Some(false) {
                    self.materialize_expr(&mut receiver, errors);
                    for arg in args.iter_mut() {
                        self.materialize_expr(arg, errors);
                    }
                    return;
                }
                self.materialize_expr(&mut receiver, errors);
                for arg in args.iter_mut() {
                    self.materialize_expr(arg, errors);
                }
                let Some(mut selected) = self.select_deferred_method(
                    &receiver,
                    &method_name,
                    args,
                    Some(&expr.ty),
                    errors,
                ) else {
                    if !self.strict
                        && (contains_recovery_type(&self.resolved_type(&receiver.ty))
                            || args
                                .iter()
                                .any(|arg| contains_recovery_type(&self.resolved_type(&arg.ty))))
                    {
                        return;
                    }
                    self.materialize_field_access(callee, errors);
                    if matches!(callee.kind, HirExprKind::FieldAccess(_, _, Some(_))) {
                        self.materialize_value_call(expr, errors);
                    }
                    return;
                };
                selected.receiver = receiver.clone();
                let adjusted_receiver = match selected.receiver_adjustment {
                    ReceiverAdjustment::TraitDeref => self
                        .trait_deref_candidate(receiver.clone())
                        .unwrap_or_else(|| receiver.clone()),
                    adjustment => self.apply_receiver_adjustment(receiver.clone(), adjustment),
                };

                let Some(function) = selected.function.as_ref() else {
                    errors.push(ResolveError::with_span(
                        format!("selected method '{}' has no executable body", method_name),
                        expr.span.clone(),
                    ));
                    return;
                };
                if selected.substituted_params.len() != args.len() {
                    errors.push(ResolveError::with_span(
                        format!(
                            "selected method '{}' expects {} arguments but received {}",
                            method_name,
                            selected.substituted_params.len(),
                            args.len()
                        ),
                        expr.span.clone(),
                    ));
                    return;
                }

                let mut substitution = selected.owner_substitution.clone();
                for (param, arg) in selected.substituted_params.iter().zip(args.iter_mut()) {
                    let mut actual = self.engine.borrow().resolve(&arg.ty);
                    let expected = param.ty.substitute_generics(&substitution);
                    let borrow_argument = match &expected {
                        Type::Reference {
                            mutable: false,
                            inner,
                        } => {
                            !matches!(actual, Type::Reference { .. })
                                || (!type_pattern_matches(
                                    &expected,
                                    &actual,
                                    &mut substitution.clone(),
                                ) && type_pattern_matches(
                                    inner,
                                    &actual,
                                    &mut substitution.clone(),
                                ))
                        }
                        _ => false,
                    };
                    if borrow_argument {
                        let borrowed = HirExpr {
                            ty: Type::Reference {
                                mutable: false,
                                inner: Box::new(actual.clone()),
                            },
                            kind: HirExprKind::Ref(false, Box::new(arg.clone())),
                            span: arg.span.clone(),
                        };
                        *arg = borrowed;
                        actual = self.engine.borrow().resolve(&arg.ty);
                    }
                    let mut matches = type_pattern_matches(&param.ty, &actual, &mut substitution);
                    if !matches && contains_inference_type(&expected) {
                        // A trait argument still free (`?r` in `x: Tag ?r`) is fixed by
                        // this call's argument rather than rejected.
                        let mut engine = self.engine.borrow_mut();
                        let mut probe = engine.clone_for_probe();
                        if probe.unify(&arg.ty, &expected).is_ok() {
                            engine.commit_probe(probe);
                            matches = true;
                        }
                    }
                    if !matches {
                        if !contains_inference_type(&expected) {
                            if let (
                                Type::Reference {
                                    mutable: expected_mutable,
                                    inner: expected_inner,
                                },
                                Type::Reference {
                                    mutable: actual_mutable,
                                    inner: actual_inner,
                                },
                            ) = (&expected, &actual)
                            {
                                if (!expected_mutable || *actual_mutable)
                                    && matches!(expected_inner.as_ref(), Type::Slice(_))
                                    && matches!(actual_inner.as_ref(), Type::Array(_, _))
                                {
                                    if let Some(coerced) =
                                        self.array_ref_to_slice_ref(arg.clone(), *expected_mutable)
                                    {
                                        if self
                                            .engine
                                            .borrow_mut()
                                            .unify(&coerced.ty, &expected)
                                            .is_ok()
                                        {
                                            *arg = coerced;
                                        }
                                    }
                                }
                            }
                            let _ = self.engine.borrow_mut().unify(&arg.ty, &expected);
                            actual = self.engine.borrow().resolve(&arg.ty);
                            matches = type_pattern_matches(&param.ty, &actual, &mut substitution);
                        }
                    }
                    if !matches {
                        if !self.strict {
                            return;
                        }
                        errors.push(ResolveError::with_span(
                            format!(
                                "selected method '{}' argument type {} does not match {}",
                                method_name,
                                self.display_type(&arg.ty),
                                self.display_type(&param.ty)
                            ),
                            arg.span.clone(),
                        ));
                        return;
                    }
                }
                self.record_selected_bounds(&selected, &substitution, "deferred method call");
                let actual_return = self.engine.borrow().resolve(&expr.ty);
                let _ =
                    type_pattern_matches(&selected.return_type, &actual_return, &mut substitution);

                let owner_params = selected
                    .target
                    .owner_substitution
                    .iter()
                    .map(|binding| binding.param)
                    .collect::<std::collections::HashSet<_>>();
                let mut method_substitution = Vec::new();
                for param in &function.generic_params {
                    if owner_params.contains(&param.id)
                        || selected
                            .target
                            .trait_id()
                            .is_some_and(|id| param.id.owner == id)
                    {
                        continue;
                    }
                    let Some(ty) = substitution.get(&param.id) else {
                        if !self.strict {
                            return;
                        }
                        errors.push(ResolveError::with_span(
                            format!(
                                "selected method '{}' leaves a generic parameter unresolved",
                                method_name
                            ),
                            expr.span.clone(),
                        ));
                        return;
                    };
                    method_substitution.push(HirTypeBinding {
                        param: param.id,
                        ty: ty.clone(),
                    });
                }
                method_substitution.sort_by_key(binding_sort_key);

                // Owner parameters fixed only by the expected return type (e.g. `U` in
                // `impl Into U for T`) are bound here, after return-type matching.
                let mut authority =
                    selected.target_with_substitution(&substitution, |ty| self.resolved_type(ty));
                authority.method_substitution = method_substitution;
                let selected_return = selected.return_type.substitute_generics(&substitution);
                let selected_callable = Type::function_with_safety(
                    selected
                        .substituted_params
                        .iter()
                        .map(|param| param.ty.substitute_generics(&substitution))
                        .collect(),
                    selected_return.clone(),
                    FunctionSafety::from_is_unsafe(function.is_unsafe),
                );
                {
                    let mut engine = self.engine.borrow_mut();
                    if let Err(error) = engine.unify(&callee.ty, &selected_callable) {
                        if self.strict {
                            errors.push(ResolveError::with_span(
                                format!(
                                    "selected method '{}' callable type does not match its deferred call: {}",
                                    method_name,
                                    error.render(&engine)
                                ),
                                expr.span.clone(),
                            ));
                            return;
                        }
                        self.ambiguous = true;
                    }
                    let _ = engine.unify(&expr.ty, &selected_return);
                    expr.ty = engine.resolve(&expr.ty);
                }
                expr.kind = HirExprKind::MethodCall(
                    Box::new(adjusted_receiver),
                    method_name,
                    std::mem::take(args),
                    function.self_receiver,
                    Some(authority),
                );
            }
            HirExprKind::MethodCall(receiver, _, args, _, target) => {
                let selected_site =
                    self.visit_authority_slot(AuthorityObligationKind::MethodCall, &expr.span);
                self.materialize_expr(receiver, errors);
                for arg in args.iter_mut() {
                    self.materialize_expr(arg, errors);
                }
                if let Some(target) = target {
                    self.materialize_method_bindings(args, &expr.ty, target, &expr.span, errors);
                }
                if selected_site && self.strict && target.is_none() {
                    errors.push(ResolveError::with_span(
                        "accepted HIR method call has no selected authority".to_string(),
                        expr.span.clone(),
                    ));
                }
            }
            HirExprKind::FieldAccess(_, _, Some(_)) => {
                self.materialize_field_access(expr, errors);
            }
            HirExprKind::FieldAccess(receiver, _, None) => {
                if self.visit_authority_slot(AuthorityObligationKind::Field, &expr.span) {
                    self.materialize_field_access(expr, errors);
                } else {
                    self.materialize_expr(receiver, errors);
                }
            }
            HirExprKind::Deref(receiver)
            | HirExprKind::Ref(_, receiver)
            | HirExprKind::Cast(receiver, _)
            | HirExprKind::TupleIndex(receiver, _) => self.materialize_expr(receiver, errors),
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.materialize_expr(condition, errors);
                self.materialize_block(then_branch, errors);
                if let Some(else_branch) = else_branch {
                    self.materialize_block(else_branch, errors);
                }
            }
            HirExprKind::Block(block) | HirExprKind::Loop(block) => {
                self.materialize_block(block, errors)
            }
            HirExprKind::UnsafeBlock(block) => {
                let previous = self.unsafe_context.replace(true);
                self.materialize_block(block, errors);
                self.unsafe_context.set(previous);
            }
            HirExprKind::While { condition, body } => {
                self.materialize_expr(condition, errors);
                self.materialize_block(body, errors);
            }
            HirExprKind::For { iter, body, .. } => {
                self.materialize_expr(iter, errors);
                self.materialize_block(body, errors);
            }
            HirExprKind::Lambda { body, .. } => {
                let previous = self.unsafe_context.replace(false);
                self.materialize_block(body, errors);
                self.unsafe_context.set(previous);
            }
            HirExprKind::Assign(left, right) | HirExprKind::BinOp(_, left, right) => {
                self.materialize_expr(left, errors);
                self.materialize_expr(right, errors);
            }
            HirExprKind::UnaryOp(_, inner) => self.materialize_expr(inner, errors),
            HirExprKind::ArrayRepeat(value, _) => self.materialize_expr(value, errors),
            HirExprKind::Intrinsic { args, .. }
            | HirExprKind::TupleLiteral(args)
            | HirExprKind::ArrayLiteral(args) => {
                for arg in args {
                    self.materialize_expr(arg, errors);
                }
            }
            HirExprKind::StructLiteral(_, _, fields) => {
                for field in fields {
                    self.materialize_expr(&mut field.value, errors);
                }
            }
            HirExprKind::EnumVariant(_, _, args, _) => {
                for arg in args {
                    self.materialize_expr(arg, errors);
                }
            }
            HirExprKind::Match { scrutinee, arms } => {
                self.materialize_expr(scrutinee, errors);
                for arm in arms {
                    if let Some(guard) = &mut arm.guard {
                        self.materialize_expr(guard, errors);
                    }
                    self.materialize_block(&mut arm.body, errors);
                }
            }
            HirExprKind::Try { .. } => {
                let materialize_branch =
                    self.visit_authority_slot(AuthorityObligationKind::TryBranch, &expr.span);
                let materialize_residual =
                    self.visit_authority_slot(AuthorityObligationKind::FromResidual, &expr.span);
                if materialize_branch || materialize_residual {
                    self.materialize_try(expr, errors, materialize_branch, materialize_residual);
                } else if let HirExprKind::Try { expr: operand, .. } = &mut expr.kind {
                    self.materialize_expr(operand, errors);
                }
            }
            HirExprKind::Var(_)
            | HirExprKind::ResolvedVar(_)
            | HirExprKind::IntLiteral(_) | HirExprKind::ConstParam(_)
            | HirExprKind::FloatLiteral(_)
            | HirExprKind::BoolLiteral(_)
            | HirExprKind::StringLiteral(_)
            | HirExprKind::CharLiteral(_)
            | HirExprKind::Unit => {}
        }
    }

    pub(super) fn materialize_callable_arguments(
        &mut self,
        callee: &HirExpr,
        args: &mut [HirExpr],
        errors: &mut Vec<ResolveError>,
    ) {
        let Type::Function { params, .. } = self.resolved_type(&callee.ty) else {
            return;
        };
        if params.len() != args.len() {
            if self.strict {
                errors.push(ResolveError::with_span(
                    format!(
                        "callable expects {} arguments but received {}",
                        params.len(),
                        args.len()
                    ),
                    callee.span.clone(),
                ));
            }
            return;
        }

        for (index, (arg, expected)) in args.iter_mut().zip(params).enumerate() {
            let actual = self.resolved_type(&arg.ty);
            let expected = self.resolved_type(&expected);
            if let (
                Type::Reference {
                    mutable: actual_mutable,
                    inner: actual_inner,
                },
                Type::Reference {
                    mutable: expected_mutable,
                    inner: expected_inner,
                },
            ) = (&actual, &expected)
            {
                if (!expected_mutable || *actual_mutable)
                    && matches!(actual_inner.as_ref(), Type::Array(_, _))
                    && matches!(expected_inner.as_ref(), Type::Slice(_))
                {
                    if let Some(coerced) =
                        self.array_ref_to_slice_ref(arg.clone(), *expected_mutable)
                    {
                        let mut probe = self.engine.borrow().clone_for_probe();
                        if probe.unify(&coerced.ty, &expected).is_ok() {
                            self.engine.borrow_mut().commit_probe(probe);
                            *arg = coerced;
                            continue;
                        }
                    }
                }
            }

            let mut probe = self.engine.borrow().clone_for_probe();
            match probe.unify(&actual, &expected) {
                Ok(()) => self.engine.borrow_mut().commit_probe(probe),
                Err(error) if self.strict => {
                    let message = self
                        .constructor_argument_mismatch_message(callee, index, &actual, &error)
                        .unwrap_or_else(|| {
                            format!(
                                "callable argument type {} does not match {}: {}",
                                self.display_type(&actual),
                                self.display_type(&expected),
                                error.render(&probe)
                            )
                        });
                    errors.push(ResolveError::with_span(message, arg.span.clone()));
                }
                Err(_) => self.ambiguous = true,
            }
        }
    }
}

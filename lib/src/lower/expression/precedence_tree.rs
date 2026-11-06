//! Operator precedence tree construction.

use super::operand::{ast_expr_operand_span, AstExprOperand, ExprUse};
use crate::hir::*;
use crate::lower::Lowerer;
use crate::selection::SelectionDiagnostic;
use crate::types::Type;

impl Lowerer {
    pub(super) fn build_precedence_tree_with_use(
        &mut self,
        operands: &[AstExprOperand<'_>],
        operators: &[String],
        start: usize,
        end: usize,
        use_kind: ExprUse,
    ) -> HirExpr {
        if start == end {
            return self.lower_ast_expr_operand_with_use(&operands[start], use_kind);
        }

        let mut min_prec = u8::MAX;
        let mut split_at = start;
        for i in start..end {
            let prec = match self.op_precedence(&operators[i]) {
                Ok(prec) => prec,
                Err(error) => {
                    let Some(span) = ast_expr_operand_span(&operands[start]) else {
                        self.diagnostics.push_toolchain(error);
                        return self.lower_ast_expr_operand_with_use(&operands[start], use_kind);
                    };
                    self.diagnostics.push_with_span(error, span.clone());
                    return self.error_expression_at(span);
                }
            };
            if prec <= min_prec {
                min_prec = prec;
                split_at = i;
            }
        }

        let op = &operators[split_at];
        let (left_use, right_use) = if op == "=" {
            (ExprUse::AssignmentPlace, ExprUse::Value)
        } else {
            (ExprUse::Value, ExprUse::Value)
        };
        let left =
            self.build_precedence_tree_with_use(operands, operators, start, split_at, left_use);
        let right =
            self.build_precedence_tree_with_use(operands, operators, split_at + 1, end, right_use);

        self.build_precedence_tree(&[left, right], std::slice::from_ref(op), 0, 1)
    }

    /// Build a correctly-precedenced tree from a flat list using
    /// a simple algorithm: find the lowest-precedence operator and split there
    pub(crate) fn build_precedence_tree(
        &mut self,
        operands: &[HirExpr],
        operators: &[String],
        start: usize,
        end: usize,
    ) -> HirExpr {
        if start == end {
            return operands[start].clone();
        }

        // Find the lowest precedence operator in range [start..end-1]
        // For left-associativity, take the RIGHTMOST occurrence of the lowest precedence
        let op_start = start;
        let op_end = end; // operators[i] is between operands[i] and operands[i+1]

        let mut min_prec = u8::MAX;
        let mut split_at = op_start;

        for i in op_start..op_end {
            let prec = match self.op_precedence(&operators[i]) {
                Ok(p) => p,
                Err(e) => {
                    let span = operands[start].span.clone();
                    self.diagnostics.push_with_span(e, span.clone());
                    return self.error_expression_at(span);
                }
            };
            if prec <= min_prec {
                min_prec = prec;
                split_at = i;
            }
        }

        let left = self.build_precedence_tree(operands, operators, start, split_at);
        let right = self.build_precedence_tree(operands, operators, split_at + 1, end);

        let op_str = &operators[split_at];

        // Assignment operator: special handling
        if op_str == "=" {
            if let Err(error) = self.engine.unify(&right.ty, &left.ty) {
                self.diagnostics.push_type_with_span(
                    format!("Assignment type mismatch: {}", error.render(&self.engine)),
                    right.span.clone(),
                );
            }
            let operation_span = right.span.clone();
            return HirExpr {
                ty: Type::Unit,
                kind: HirExprKind::Assign(Box::new(left), Box::new(right)),
                span: operation_span,
            };
        }

        // Short-circuit operators: keep as BinOp for special evaluation semantics
        if op_str == "&&" || op_str == "||" {
            // Comparison/logical ops: operands should be compatible
            if let Err(e) = self.engine.unify(&left.ty, &right.ty) {
                self.diagnostics.push_type_with_span(
                    format!(
                        "Binary operator '{}': operand type mismatch: {}",
                        op_str,
                        e.render(&self.engine)
                    ),
                    right.span.clone(),
                );
            }
            let operation_span = right.span.clone();
            let bin_op = if op_str == "&&" {
                BinOp::And
            } else {
                BinOp::Or
            };
            return HirExpr {
                ty: Type::Bool,
                kind: HirExprKind::BinOp(bin_op, Box::new(left), Box::new(right)),
                span: operation_span,
            };
        }

        let resolved_left_ty = self.resolve_projection_type(&self.engine.resolve(&left.ty));

        if op_str == "&" {
            let resolved_fn_ty = self.resolve_projection_type(&resolved_left_ty);
            if let Type::Function {
                params,
                ret,
                safety,
                ..
            } = &resolved_fn_ty
            {
                let right_span = right.span.clone();
                let borrowed_right = HirExpr {
                    ty: Type::Reference {
                        mutable: false,
                        inner: Box::new(right.ty.clone()),
                    },
                    kind: HirExprKind::Ref(false, Box::new(right)),
                    span: right_span.clone(),
                };
                let arg = params
                    .first()
                    .map(|param| self.coerce_argument_to_expected(borrowed_right.clone(), param))
                    .unwrap_or(borrowed_right);
                let ret_ty = if params.len() == 1 {
                    ret.as_ref().clone()
                } else {
                    self.engine.fresh_type_var_at(right_span)
                };
                let expected_fn_ty =
                    Type::function_with_safety(vec![arg.ty.clone()], ret_ty.clone(), *safety);
                let _ = self.engine.unify(&left.ty, &expected_fn_ty);
                self.report_unsafe_function_value_call_from_type(&left);

                let left_is_unsafe_function = matches!(
                    self.resolve_projection_type(&self.engine.resolve(&left.ty)),
                    Type::Function {
                        safety: crate::types::FunctionSafety::Unsafe,
                        ..
                    }
                );
                if !left_is_unsafe_function {
                    match &left.kind {
                        HirExprKind::ResolvedVar(reference) => self
                            .report_unsafe_operator_function_call_if_needed(
                                Some(&reference.target),
                                &reference.name,
                                left.span.clone(),
                            ),
                        HirExprKind::Var(function_name) => self
                            .report_unsafe_operator_function_call_if_needed(
                                None,
                                function_name,
                                left.span.clone(),
                            ),
                        _ => {}
                    }
                }

                let target = self.call_target_for_callee(&left);
                let operation_span = left.span.clone();
                return HirExpr {
                    ty: self.resolve_projection_type(&self.engine.resolve(&ret_ty)),
                    kind: HirExprKind::Call(Box::new(left), vec![arg], target),
                    span: operation_span,
                };
            }
        }

        let mut resolved_ty = self.resolve_projection_type(&self.engine.resolve(&left.ty));

        if let Type::Generic(gen_param) = &resolved_ty {
            let bounds = self
                .current_impl_bounds()
                .get(gen_param)
                .cloned()
                .unwrap_or_default();
            let receiver_candidates = self.receiver_adjustment_candidates(left.clone());

            let result = self.selection_service().select_bound_method_matching(
                &receiver_candidates,
                &bounds,
                op_str,
                resolved_ty.clone(),
                matches!(&left.kind, HirExprKind::Call(_, _, _)),
                |selected| {
                    self.operator_method_accepts_args(selected, std::slice::from_ref(&right))
                },
            );
            let span = left.span.clone();
            if let Some(selected) = self.handle_optional_selection(result, span) {
                let adjusted_recv = self.apply_receiver_adjustment(
                    selected.receiver.clone(),
                    selected.receiver_adjustment,
                );
                let right = self.autoref_operator_arg_for_selected_method(right, &selected, 0);
                let Some((method_func, ret_ty, coerced_args, target)) =
                    self.selected_method_call_types(&selected, vec![right])
                else {
                    return self.error_expression_at(left.span.clone());
                };
                self.report_unsafe_operator_method_call_if_needed(
                    &method_func,
                    op_str,
                    left.span.clone(),
                );

                return HirExpr {
                    ty: ret_ty,
                    kind: HirExprKind::MethodCall(
                        Box::new(adjusted_recv),
                        op_str.clone(),
                        coerced_args,
                        method_func.self_receiver,
                        Some(target),
                    ),
                    span: left.span.clone(),
                };
            }
        }

        if let Type::TypeVar(id) = &resolved_ty {
            let bounds = self.engine.get_bounds(*id);
            let receiver_candidates = self.receiver_adjustment_candidates(left.clone());
            let result = self.selection_service().select_bound_method_matching(
                &receiver_candidates,
                &bounds,
                op_str,
                resolved_ty.clone(),
                matches!(&left.kind, HirExprKind::Call(_, _, _)),
                |selected| {
                    self.operator_method_accepts_args(selected, std::slice::from_ref(&right))
                },
            );
            let span = left.span.clone();
            if let Some(selected) = self.handle_optional_selection(result, span) {
                let adjusted_recv = self.apply_receiver_adjustment(
                    selected.receiver.clone(),
                    selected.receiver_adjustment,
                );
                let right = self.autoref_operator_arg_for_selected_method(right, &selected, 0);
                let Some((method_func, ret_ty, coerced_args, target)) =
                    self.selected_method_call_types(&selected, vec![right])
                else {
                    return self.error_expression_at(left.span.clone());
                };
                self.report_unsafe_operator_method_call_if_needed(
                    &method_func,
                    op_str,
                    left.span.clone(),
                );

                return HirExpr {
                    ty: ret_ty,
                    kind: HirExprKind::MethodCall(
                        Box::new(adjusted_recv),
                        op_str.clone(),
                        coerced_args,
                        method_func.self_receiver,
                        Some(target),
                    ),
                    span: left.span.clone(),
                };
            }
        }

        let func_binding = crate::lower::resolution::LowerResolutionContext::new(self)
            .resolve_identifier_value(op_str.as_str())
            .map(|resolved| {
                let func_ty = self.instantiate_resolved_value_type(&resolved, left.span.clone());
                (func_ty, resolved.name, resolved.target)
            });

        if let Some((func_ty, hir_name, target)) = func_binding {
            self.infer_bounded_constructor_heads(&func_ty, &[&left, &right]);
            let resolved_func_ty = self.resolve_projection_type(&self.engine.resolve(&func_ty));

            // A curried operator (`>> = f, g ~> ...`) takes its operands one at
            // a time, like an ordinary call to a curried function.
            if matches!(&resolved_func_ty, Type::Function { params, .. } if params.len() == 1) {
                self.report_unsafe_operator_function_call_if_needed(
                    target.as_ref(),
                    &hir_name,
                    left.span.clone(),
                );
                let operation_span = left.span.clone();
                let callee = HirExpr {
                    ty: resolved_func_ty,
                    kind: match target {
                        Some(target) => HirExprKind::ResolvedVar(HirVarRef {
                            name: hir_name,
                            target,
                        }),
                        None => HirExprKind::Var(hir_name),
                    },
                    span: operation_span.clone(),
                };
                return self.fold_unary_calls(callee, vec![left, right], operation_span);
            }

            let (mut expected_ret_ty, safety) = match resolved_func_ty {
                Type::Function { ret, safety, .. } => (*ret, safety),
                _ => (
                    self.engine.fresh_type_var_at(left.span.clone()),
                    crate::types::FunctionSafety::Safe,
                ),
            };
            let expected_fn_ty = Type::function_with_safety(
                vec![left.ty.clone(), right.ty.clone()],
                expected_ret_ty.clone(),
                safety,
            );
            if let Err(e) = self.engine.unify(&func_ty, &expected_fn_ty) {
                if matches!(
                    &e,
                    crate::infer::UnifyError::ConstructorInference { .. }
                        | crate::infer::UnifyError::AmbiguousConstructorHeads { .. }
                ) && (Self::type_has_unresolved_parameter(&left.ty)
                    || Self::type_has_unresolved_parameter(&right.ty))
                {
                    let span = left.span.clone();
                    self.constraint_store.add_equality(
                        func_ty.clone(),
                        expected_fn_ty.clone(),
                        span,
                        &format!("Operator '{op_str}' function type mismatch"),
                    );
                } else {
                    self.diagnostics.push_type_with_span(
                        format!(
                            "Operator '{}': function type mismatch: {}",
                            op_str,
                            e.render(&self.engine)
                        ),
                        left.span.clone(),
                    );
                }
            }

            expected_ret_ty = match self.engine.normalize_resolved_type(&expected_ret_ty) {
                Ok(ty) => ty,
                Err(error) => {
                    self.diagnostics
                        .push_type_with_span(error, left.span.clone());
                    Type::Error
                }
            };
            self.report_unsafe_operator_function_call_if_needed(
                target.as_ref(),
                &hir_name,
                left.span.clone(),
            );

            let operation_span = left.span.clone();
            return HirExpr {
                ty: expected_ret_ty.clone(),
                kind: {
                    let callee = HirExpr {
                        ty: self.engine.resolve(&func_ty),
                        kind: match target {
                            Some(target) => HirExprKind::ResolvedVar(HirVarRef {
                                name: hir_name,
                                target,
                            }),
                            None => HirExprKind::Var(hir_name),
                        },
                        span: operation_span.clone(),
                    };
                    let target = self.call_target_for_callee(&callee);
                    HirExprKind::Call(Box::new(callee), vec![left, right], target)
                },
                span: operation_span,
            };
        }

        // Use operator symbol directly as method name (e.g. "+" -> "+", ">>=" -> ">>=")
        let method_name = op_str.clone();

        if matches!(resolved_ty, Type::TypeVar(_)) {
            match self.infer_unresolved_binary_operator_receiver(&left, &right, &method_name) {
                Ok(true) => {
                    resolved_ty = self.resolve_projection_type(&self.engine.resolve(&left.ty));
                }
                Ok(false) => {}
                Err(SelectionDiagnostic::AmbiguousCandidates { .. }) => {
                    let result_ty = self.engine.fresh_type_var_at(left.span.clone());
                    let operation_span = left.span.clone();
                    let callee = HirExpr {
                        ty: Type::function(vec![right.ty.clone()], result_ty.clone()),
                        kind: HirExprKind::FieldAccess(Box::new(left), method_name, None),
                        span: operation_span.clone(),
                    };
                    return HirExpr {
                        ty: result_ty,
                        kind: HirExprKind::Call(Box::new(callee), vec![right], None),
                        span: operation_span,
                    };
                }
                Err(error) => {
                    let message = self.display_selection_error(&error);
                    self.diagnostics
                        .push_selection_with_span(message, left.span.clone());
                    return self.error_expression_at(left.span.clone());
                }
            }
        }

        let receiver_candidates = self.receiver_adjustment_candidates(left.clone());
        let found_method = match self.select_operator_method(
            &receiver_candidates,
            &method_name,
            std::slice::from_ref(&right),
        ) {
            Ok(selected) => selected,
            Err(error) => {
                let message = self.display_selection_error(&error);
                self.diagnostics
                    .push_selection_with_span(message, left.span.clone());
                return self.error_expression_at(left.span.clone());
            }
        };

        if let Some(selected) = found_method {
            let right = self.autoref_operator_arg_for_selected_method(right, &selected, 0);
            let Some((method_func, result_ty, mut coerced_args, target)) =
                self.selected_method_call_types(&selected, vec![right])
            else {
                return self.error_expression_at(left.span.clone());
            };
            let right = coerced_args.remove(0);
            self.report_unsafe_operator_method_call_if_needed(
                &method_func,
                &method_name,
                left.span.clone(),
            );

            return HirExpr {
                ty: result_ty,
                kind: HirExprKind::MethodCall(
                    Box::new(self.apply_receiver_adjustment(
                        selected.receiver,
                        selected.receiver_adjustment,
                    )),
                    method_name,
                    vec![right],
                    method_func.self_receiver,
                    Some(target),
                ),
                span: left.span.clone(),
            };
        }

        let receiver_type = self.display_type(&resolved_ty);
        self.diagnostics.push_selection_with_span(
            format!(
                "No implementation found for operator '{}' on type {}",
                op_str, receiver_type
            ),
            left.span.clone(),
        );

        self.error_expression_at(left.span.clone())
    }
}

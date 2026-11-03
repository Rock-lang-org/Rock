//! Unary expression lowering.

use super::operand::ExprUse;
use crate::ast;
use crate::hir::*;
use crate::lexer::Span;
use crate::lower::Lowerer;
use crate::selection::SelectionDiagnostic;
use crate::types::Type;

impl Lowerer {
    pub(crate) fn error_expression_at(&self, span: Span) -> HirExpr {
        HirExpr {
            ty: Type::Error,
            kind: HirExprKind::Var("<error>".to_string()),
            span,
        }
    }

    pub(crate) fn lower_unary_expr(&mut self, expr: &ast::UnaryExpr) -> HirExpr {
        self.lower_unary_expr_with_use(expr, ExprUse::Value)
    }

    pub(crate) fn lower_unary_expr_with_use(
        &mut self,
        expr: &ast::UnaryExpr,
        use_kind: ExprUse,
    ) -> HirExpr {
        match expr {
            ast::UnaryExpr::PrimaryExpr(primary) => {
                if use_kind == ExprUse::Value {
                    self.lower_primary_expr(primary)
                } else {
                    self.lower_primary_expr_with_use(primary, use_kind)
                }
            }
            ast::UnaryExpr::UnaryExpr(op, inner) => {
                // Special handling for & / &mut (reference operator)
                if op.value == "&" || op.value == "&mut" {
                    let inner_use = if op.value == "&mut" {
                        ExprUse::AssignmentPlace
                    } else {
                        ExprUse::Value
                    };
                    let inner_hir = self.lower_unary_expr_with_use(inner, inner_use);

                    if op.value == "&mut" {
                        if let HirExprKind::Var(name) = &inner_hir.kind {
                            if let Some(binding) = self.scope.lookup(name) {
                                if !binding.mutable {
                                    self.diagnostics.push_with_span(
                                        format!(
                                            "Cannot take a mutable reference to immutable binding '{}'",
                                            name
                                        ),
                                        op.span.clone(),
                                    );
                                }
                            }
                        }
                    }

                    let ref_ty = Type::Reference {
                        mutable: op.value == "&mut",
                        inner: Box::new(inner_hir.ty.clone()),
                    };
                    return HirExpr {
                        ty: ref_ty,
                        kind: HirExprKind::Ref(op.value == "&mut", Box::new(inner_hir)),
                        span: op.span.clone(),
                    };
                }

                let inner_hir = self.lower_unary_expr_with_use(inner, ExprUse::Value);

                // Special handling for * (dereference operator)
                if op.value == "*" {
                    let resolved_inner_ty = self.engine.resolve(&inner_hir.ty);
                    let inner_span = inner_hir.span.clone();
                    if let Type::Reference { inner, .. } = &resolved_inner_ty {
                        return HirExpr {
                            ty: (*inner.clone()),
                            kind: HirExprKind::Deref(Box::new(inner_hir)),
                            span: inner_span.clone(),
                        };
                    }

                    if let Type::Pointer(inner) = &resolved_inner_ty {
                        if !self.is_in_unsafe() {
                            self.diagnostics.push_selection_with_span(
                                "Dereference of raw pointer requires an unsafe block".to_string(),
                                op.span.clone(),
                            );
                        }

                        return HirExpr {
                            ty: (*inner.clone()),
                            kind: HirExprKind::Deref(Box::new(inner_hir)),
                            span: inner_span,
                        };
                    }

                    if let Some(next) = self
                        .autoderef_candidates(inner_hir.clone())
                        .into_iter()
                        .nth(1)
                    {
                        return next;
                    }

                    let result_ty = resolved_inner_ty.clone();
                    let receiver_type = self.display_type(&resolved_inner_ty);
                    self.diagnostics.push_type_with_span(
                        format!("Cannot dereference non-pointer type: {}", receiver_type),
                        op.span.clone(),
                    );
                    return HirExpr {
                        ty: result_ty,
                        kind: HirExprKind::Deref(Box::new(inner_hir)),
                        span: op.span.clone(),
                    };
                }

                if op.value == "-" || op.value == "!" {
                    let method_name = op.value.as_str();
                    let mut resolved_ty =
                        self.resolve_projection_type(&self.engine.resolve(&inner_hir.ty));

                    if matches!(resolved_ty, Type::TypeVar(_)) {
                        match self.infer_unresolved_unary_operator_receiver(&inner_hir, method_name)
                        {
                            Ok(true) => {
                                resolved_ty = self
                                    .resolve_projection_type(&self.engine.resolve(&inner_hir.ty));
                            }
                            Ok(false) => {}
                            Err(SelectionDiagnostic::AmbiguousCandidates { .. }) => {
                                let result_ty = self.engine.fresh_type_var_at(op.span.clone());
                                let callee = HirExpr {
                                    ty: Type::function(Vec::new(), result_ty.clone()),
                                    kind: HirExprKind::FieldAccess(
                                        Box::new(inner_hir),
                                        method_name.to_string(),
                                        None,
                                    ),
                                    span: op.span.clone(),
                                };
                                return HirExpr {
                                    ty: result_ty,
                                    kind: HirExprKind::Call(Box::new(callee), Vec::new(), None),
                                    span: op.span.clone(),
                                };
                            }
                            Err(error) => {
                                let message = self.display_selection_error(&error);
                                self.diagnostics
                                    .push_selection_with_span(message, op.span.clone());
                                return self.error_expression_at(op.span.clone());
                            }
                        }
                    }

                    let bound_method_result = match &resolved_ty {
                        Type::TypeVar(id) => {
                            let bounds = self.engine.get_bounds(*id);
                            let receiver_candidates =
                                self.receiver_adjustment_candidates(inner_hir.clone());
                            Some(
                                self.selection_service()
                                    .select_bound_method_preferring_non_ref_receiver(
                                        &receiver_candidates,
                                        &bounds,
                                        method_name,
                                        resolved_ty.clone(),
                                        matches!(&inner_hir.kind, HirExprKind::Call(_, _, _)),
                                    ),
                            )
                        }
                        Type::Generic(gen_param) => {
                            let bounds = self
                                .current_impl_bounds()
                                .get(gen_param)
                                .cloned()
                                .unwrap_or_default();
                            let receiver_candidates =
                                self.receiver_adjustment_candidates(inner_hir.clone());
                            Some(
                                self.selection_service()
                                    .select_bound_method_preferring_non_ref_receiver(
                                        &receiver_candidates,
                                        &bounds,
                                        method_name,
                                        resolved_ty.clone(),
                                        matches!(&inner_hir.kind, HirExprKind::Call(_, _, _)),
                                    ),
                            )
                        }
                        _ => None,
                    };
                    let bound_method = bound_method_result
                        .and_then(|result| self.handle_optional_selection(result, op.span.clone()));

                    if let Some(selected) = bound_method {
                        let Some((method_func, result_ty, args, target)) =
                            self.selected_method_call_types(&selected, Vec::new())
                        else {
                            return self.error_expression_at(op.span.clone());
                        };
                        self.report_unsafe_operator_method_call_if_needed(
                            &method_func,
                            method_name,
                            inner_hir.span.clone(),
                        );

                        return HirExpr {
                            ty: result_ty,
                            kind: HirExprKind::MethodCall(
                                Box::new(self.apply_receiver_adjustment(
                                    selected.receiver,
                                    selected.receiver_adjustment,
                                )),
                                method_name.to_string(),
                                args,
                                method_func.self_receiver,
                                Some(target),
                            ),
                            span: inner_hir.span.clone(),
                        };
                    }

                    let receiver_candidates =
                        self.receiver_adjustment_candidates(inner_hir.clone());
                    let selected =
                        match self.select_operator_method(&receiver_candidates, method_name, &[]) {
                            Ok(selected) => selected,
                            Err(error) => {
                                let message = self.display_selection_error(&error);
                                self.diagnostics
                                    .push_selection_with_span(message, op.span.clone());
                                return self.error_expression_at(op.span.clone());
                            }
                        };
                    if let Some(selected) = selected {
                        let Some((method_func, result_ty, args, target)) =
                            self.selected_method_call_types(&selected, Vec::new())
                        else {
                            return self.error_expression_at(op.span.clone());
                        };
                        self.report_unsafe_operator_method_call_if_needed(
                            &method_func,
                            method_name,
                            op.span.clone(),
                        );

                        return HirExpr {
                            ty: result_ty,
                            kind: HirExprKind::MethodCall(
                                Box::new(self.apply_receiver_adjustment(
                                    selected.receiver,
                                    selected.receiver_adjustment,
                                )),
                                method_name.to_string(),
                                args,
                                method_func.self_receiver,
                                Some(target),
                            ),
                            span: inner_hir.span.clone(),
                        };
                    }

                    let receiver_type = self.display_type(&resolved_ty);
                    self.diagnostics.push_selection_with_span(
                        format!(
                            "No implementation found for operator '{}' on type {}",
                            op.value, receiver_type
                        ),
                        op.span.clone(),
                    );
                    return self.error_expression_at(op.span.clone());
                }

                let unary_op = match op.value.as_str() {
                    "~" => UnaryOp::BitNot,
                    _ => {
                        self.diagnostics.push_with_span(
                            format!("Unknown unary operator: {}", op.value),
                            op.span.clone(),
                        );
                        UnaryOp::BitNot
                    }
                };
                let ty = inner_hir.ty.clone();
                let inner_span = inner_hir.span.clone();
                HirExpr {
                    ty,
                    kind: HirExprKind::UnaryOp(unary_op, Box::new(inner_hir)),
                    span: inner_span,
                }
            }
        }
    }
}

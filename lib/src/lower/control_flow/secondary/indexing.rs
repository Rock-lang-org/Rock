//! Index application for secondary expressions.

use crate::ast;
use crate::hir::*;
use crate::lexer::Span;
use crate::lower::expression::ExprUse;
use crate::lower::Lowerer;
use crate::type_services::facts::TypeFacts;
use crate::types::{TraitBound, Type};
use std::collections::HashMap;

impl Lowerer {
    pub(super) fn apply_indice_secondary(
        &mut self,
        expr: HirExpr,
        index_expr: &ast::Expression,
        span: Span,
        use_kind: ExprUse,
    ) -> HirExpr {
        let index = self.lower_expression(index_expr);
        let resolved_expr_ty = self.resolve_projection_type(&self.engine.resolve(&expr.ty));
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
                        self.resolve_projection_type(&self.engine.resolve(&candidate.expr.ty)),
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
                                self.resolve_projection_type(&self.engine.resolve(&expr.ty)),
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
                        crate::type_services::display::short_item_name(&selected_method_name)
                    ),
                    span.clone(),
                );
            }
            self.record_pending_impl_bounds(&selected, &HashMap::new(), "index operator");
            let selected_return_type = &selected.return_type;
            let expected_output = Type::Projection {
                ty: Box::new(
                    self.resolve_projection_type(&self.engine.resolve(&selected.receiver.ty)),
                ),
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
                    let actual_inner = self.resolve_projection_type(&self.engine.resolve(inner));
                    let expected_inner =
                        self.resolve_projection_type(&self.engine.resolve(&expected_output));
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
}

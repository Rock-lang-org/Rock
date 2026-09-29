//! Trait-owned loops lower to ordinary short-circuiting fold calls.

use crate::ast;
use crate::hir::*;
use crate::lexer::Span;
use crate::lower::Lowerer;
use crate::types::{FunctionSafety, TraitBound, Type};

#[derive(Clone)]
pub(crate) struct FoldLoopContext {
    name: String,
    stop_variant: HirVariantLocation,
    next_variant: HirVariantLocation,
    exit_ty: Type,
    callback_ty: Type,
    span: Span,
}

impl FoldLoopContext {
    pub(crate) fn unit(&self) -> HirExpr {
        HirExpr {
            kind: HirExprKind::Unit,
            ty: Type::Unit,
            span: self.span.clone(),
        }
    }

    fn variant(&self, stop: bool, value: HirExpr, ty: Type) -> HirExpr {
        let location = if stop {
            &self.stop_variant
        } else {
            &self.next_variant
        };
        HirExpr {
            kind: HirExprKind::EnumVariant(
                self.name.clone(),
                location.name.clone(),
                vec![value],
                Some(location.clone()),
            ),
            ty,
            span: self.span.clone(),
        }
    }

    pub(crate) fn next(&self) -> HirExpr {
        self.variant(false, self.unit(), self.callback_ty.clone())
    }

    pub(crate) fn stop(&self) -> HirExpr {
        self.variant(
            true,
            self.variant(false, self.unit(), self.exit_ty.clone()),
            self.callback_ty.clone(),
        )
    }

    fn returned(&self, value: HirExpr) -> HirExpr {
        self.variant(
            true,
            self.variant(true, value, self.exit_ty.clone()),
            self.callback_ty.clone(),
        )
    }

    fn break_pattern(&self, pattern: HirPattern) -> HirPattern {
        HirPattern::Enum(
            self.name.clone(),
            self.stop_variant.name.clone(),
            Some(self.stop_variant.clone()),
            vec![pattern],
        )
    }
}

impl Lowerer {
    fn fold_pattern_is_irrefutable(&self, pattern: &HirPattern) -> bool {
        match pattern {
            HirPattern::Wildcard | HirPattern::Binding { .. } => true,
            HirPattern::Tuple(patterns) => patterns
                .iter()
                .all(|pattern| self.fold_pattern_is_irrefutable(pattern)),
            HirPattern::Struct(_, _, _, fields) => fields
                .iter()
                .all(|field| self.fold_pattern_is_irrefutable(&field.pattern)),
            HirPattern::Enum(_, _, Some(location), patterns) => {
                self.items
                    .enumeration(location.owner)
                    .is_some_and(|enumeration| enumeration.variants.len() == 1)
                    && patterns
                        .iter()
                        .all(|pattern| self.fold_pattern_is_irrefutable(pattern))
            }
            HirPattern::Or(patterns) => patterns
                .iter()
                .any(|pattern| self.fold_pattern_is_irrefutable(pattern)),
            HirPattern::Literal(_) | HirPattern::Enum(_, _, None, _) => false,
        }
    }

    pub(crate) fn wrap_fold_return(&self, mut value: HirExpr) -> HirExpr {
        for context in self.fold_loop_stack.iter().flatten() {
            value = context.returned(value);
        }
        value
    }

    pub(crate) fn lower_fold_loop(
        &mut self,
        pattern: &ast::Pattern,
        iter: HirExpr,
        body: &ast::Block,
        span: Span,
    ) -> HirExpr {
        let (Some(fold), Some(flow)) = (
            self.language_items.fold.clone(),
            self.language_items.try_protocol.clone(),
        ) else {
            self.diagnostics.push_with_span(
                "for loops over this source require the fold and control-flow language protocols"
                    .to_string(),
                span.clone(),
            );
            return self.error_expression_at(span);
        };
        let iter_ty = self.engine.resolve(&iter.ty);
        let member_name = self.trait_by_id(fold.trait_id).and_then(|trait_def| {
            trait_def
                .signatures
                .values()
                .find(|signature| signature.id == fold.method_id)
                .map(|signature| signature.name.clone())
                .or_else(|| {
                    trait_def
                        .methods
                        .values()
                        .find(|method| method.id == fold.method_id)
                        .map(|method| method.name.clone())
                })
        });
        let Some(member_name) = member_name else {
            self.diagnostics.push_with_span(
                "fold protocol member is unavailable".to_string(),
                span.clone(),
            );
            return self.error_expression_at(span);
        };
        let bounds = match &iter_ty {
            Type::TypeVar(id) => {
                let mut bounds = self
                    .engine
                    .get_bounds(*id)
                    .into_iter()
                    .filter(|bound| bound.trait_id == fold.trait_id)
                    .collect::<Vec<_>>();
                if bounds.is_empty() {
                    let item = self.engine.fresh_type_var_at(span.clone());
                    let bound = TraitBound {
                        trait_id: fold.trait_id,
                        type_args: vec![item],
                    };
                    self.engine.add_bound(*id, bound.clone());
                    self.constraint_store.add_trait(
                        iter_ty.clone(),
                        bound.clone(),
                        span.clone(),
                        "for loop",
                    );
                    bounds.push(bound);
                }
                bounds
            }
            Type::Generic(id) => self
                .current_impl_bounds()
                .get(id)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|bound| bound.trait_id == fold.trait_id)
                .collect(),
            _ => Vec::new(),
        };
        let selection = if bounds.is_empty() {
            let candidates = vec![crate::selection::ReceiverCandidate {
                expr: iter.clone(),
                adjustment: crate::selection::ReceiverAdjustment::None,
                can_autoref_mut: false,
            }];
            self.selection_service().select_concrete_method_matching(
                &candidates,
                &member_name,
                |ty| self.resolve_projection_type(&self.engine.resolve(ty)),
                |candidate| {
                    candidate.authority().trait_id() == Some(fold.trait_id)
                        && candidate.receiver_adjustment
                            == crate::selection::ReceiverAdjustment::None
                },
            )
        } else {
            let candidates = self.receiver_adjustment_candidates(iter.clone());
            self.selection_service().select_bound_method(
                &candidates,
                &bounds,
                &member_name,
                iter_ty.clone(),
            )
        };
        let selected = match selection {
            Ok(selected) => selected,
            Err(error) => {
                let message = self.display_selection_error(&error);
                self.diagnostics
                    .push_selection_with_span(message, span.clone());
                return self.error_expression_at(span);
            }
        };
        let Some(element_ty) = selected.authority().trait_args().first().cloned() else {
            self.diagnostics.push_with_span(
                "fold protocol must identify one element type".to_string(),
                span.clone(),
            );
            return self.error_expression_at(span);
        };
        let return_ty = self.current_body_return_type().unwrap_or(Type::Unit);
        let exit_ty = Type::Enum {
            id: flow.control_flow_enum_id,
            args: vec![return_ty.clone(), Type::Unit],
        };
        let callback_ty = Type::Enum {
            id: flow.control_flow_enum_id,
            args: vec![exit_ty.clone(), Type::Unit],
        };
        let Some(enumeration) = self.items.enumeration(flow.control_flow_enum_id) else {
            self.diagnostics.push_with_span(
                "for loop control-flow enum is unavailable".to_string(),
                span.clone(),
            );
            return self.error_expression_at(span);
        };
        let location = |id| {
            enumeration
                .variants
                .iter()
                .find(|v| v.id == id)
                .map(|v| HirVariantLocation {
                    owner: enumeration.id,
                    variant_id: id,
                    name: v.name.clone(),
                })
        };
        let (Some(stop_variant), Some(next_variant)) = (
            location(flow.break_variant_id),
            location(flow.continue_variant_id),
        ) else {
            self.diagnostics.push_with_span(
                "for loop control-flow variants are unavailable".to_string(),
                span.clone(),
            );
            return self.error_expression_at(span);
        };
        let context = FoldLoopContext {
            name: enumeration.name.clone(),
            stop_variant,
            next_variant,
            exit_ty,
            callback_ty: callback_ty.clone(),
            span: span.clone(),
        };

        self.push_scope();
        let param_id = self.fresh_local_id();
        if let Some(owner) = self.current_body_def_id() {
            self.source_map.insert_local_in_scope(
                owner,
                param_id,
                span.clone(),
                self.source_scope_stack.last().copied(),
            );
        }
        let param_ty = Type::Tuple(vec![Type::Unit, element_ty.clone()]);
        let param = HirParam {
            name: "$fold_pair".to_string(),
            local_id: param_id,
            ty: param_ty.clone(),
            mutable: false,
            is_ref: false,
        };
        let binding = self.lower_pattern(pattern, &element_ty);
        if !self.fold_pattern_is_irrefutable(&binding) {
            self.pop_scope();
            self.diagnostics.push_with_span(
                "for loop pattern must be irrefutable".to_string(),
                Self::pattern_binding_span(pattern).unwrap_or_else(|| span.clone()),
            );
            return self.error_expression_at(span);
        }
        self.fold_loop_stack.push(Some(context.clone()));
        let mut body =
            self.with_body_return_type(callback_ty.clone(), |lowerer| lowerer.lower_block(body));
        self.fold_loop_stack.pop();
        body.stmts.push(HirStmt::Expr(context.next()));
        body.ty = callback_ty.clone();
        let destructure = HirExpr {
            ty: callback_ty.clone(),
            span: span.clone(),
            kind: HirExprKind::Match {
                scrutinee: Box::new(HirExpr {
                    ty: param_ty.clone(),
                    span: span.clone(),
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: param.name.clone(),
                        target: HirVarTarget::Local(param_id),
                    }),
                }),
                arms: vec![HirMatchArm {
                    pattern: HirPattern::Tuple(vec![HirPattern::Wildcard, binding]),
                    guard: None,
                    body,
                }],
            },
        };
        let body = HirBlock {
            ty: callback_ty.clone(),
            stmts: vec![HirStmt::Expr(destructure)],
        };
        let params = vec![param];
        let captures = self.collect_lambda_captures(&body, &params);
        self.pop_scope();
        let callback = HirExpr {
            ty: Self::lambda_function_type(
                vec![param_ty],
                callback_ty.clone(),
                FunctionSafety::Safe,
                &captures,
            ),
            kind: HirExprKind::Lambda {
                params,
                body,
                captures,
            },
            span: span.clone(),
        };
        let Some((method, ret_ty, args, target)) =
            self.selected_method_call_types(&selected, vec![callback, context.unit()])
        else {
            self.diagnostics.push_with_span(
                "fold protocol method is unavailable".to_string(),
                span.clone(),
            );
            return self.error_expression_at(span);
        };
        if method.is_unsafe && !self.is_in_unsafe() {
            self.diagnostics.push_with_span(
                "for loop fold implementation requires an unsafe block".to_string(),
                span.clone(),
            );
        }
        if let Err(error) = self.engine.unify(&ret_ty, &callback_ty) {
            self.diagnostics.push_type_with_span(
                format!(
                    "for loop fold result mismatch: {}",
                    error.render(&self.engine)
                ),
                span.clone(),
            );
        }
        let receiver =
            self.apply_receiver_adjustment(selected.receiver.clone(), selected.receiver_adjustment);
        let call = HirExpr {
            ty: ret_ty,
            span: span.clone(),
            kind: HirExprKind::MethodCall(
                Box::new(receiver),
                method.name,
                args,
                method.self_receiver,
                Some(target),
            ),
        };
        let return_id = self.fresh_local_id();
        if let Some(owner) = self.current_body_def_id() {
            self.source_map.insert_local_in_scope(
                owner,
                return_id,
                span.clone(),
                self.source_scope_stack.last().copied(),
            );
        }
        let return_name = "$fold_return".to_string();
        let returned = HirExpr {
            ty: return_ty,
            span: span.clone(),
            kind: HirExprKind::ResolvedVar(HirVarRef {
                name: return_name.clone(),
                target: HirVarTarget::Local(return_id),
            }),
        };
        HirExpr {
            ty: Type::Unit,
            span,
            kind: HirExprKind::Match {
                scrutinee: Box::new(call),
                arms: vec![
                    HirMatchArm {
                        pattern: context.break_pattern(context.break_pattern(
                            HirPattern::Binding {
                                name: return_name,
                                local_id: return_id,
                                mutable: false,
                            },
                        )),
                        guard: None,
                        body: HirBlock {
                            ty: Type::Never,
                            stmts: vec![HirStmt::Return(Some(returned))],
                        },
                    },
                    HirMatchArm {
                        pattern: HirPattern::Wildcard,
                        guard: None,
                        body: HirBlock {
                            ty: Type::Unit,
                            stmts: vec![HirStmt::Expr(context.unit())],
                        },
                    },
                ],
            },
        }
    }
}

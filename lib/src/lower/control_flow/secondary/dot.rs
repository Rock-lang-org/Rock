//! Field and method access application for secondary expressions.

use crate::ast;
use crate::hir::*;
use crate::lexer::Span;
use crate::lower::Lowerer;
use crate::types::Type;

impl Lowerer {
    pub(super) fn apply_dot_secondary(
        &mut self,
        expr: HirExpr,
        ident_or_num: &ast::IdentOrNumber,
        span: Span,
        method_as_value: bool,
    ) -> HirExpr {
        match ident_or_num {
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
                        let receiver_candidates = self.receiver_adjustment_candidates(expr.clone());
                        let result = self
                            .selection_service()
                            .select_bound_method_preferring_non_ref_receiver(
                                &receiver_candidates,
                                &bounds,
                                &ident.name,
                                recv_ty.clone(),
                                matches!(&expr.kind, HirExprKind::Call(_, _, _)),
                            );
                        if let Some(selected) = self.handle_optional_selection(result, span.clone())
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
                        let receiver_candidates = self.receiver_adjustment_candidates(expr.clone());
                        let result = self
                            .selection_service()
                            .select_bound_method_preferring_non_ref_receiver(
                                &receiver_candidates,
                                &bounds,
                                &ident.name,
                                recv_ty.clone(),
                                matches!(&expr.kind, HirExprKind::Call(_, _, _)),
                            );
                        if let Some(selected) = self.handle_optional_selection(result, span.clone())
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
                        self.record_pending_impl_bounds(&selected, &method_subst, "method value");
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
                                    crate::type_services::display::short_item_name(&ident.name)
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
                                crate::types::FunctionSafety::from_is_unsafe(method_func.is_unsafe),
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
                        .select_concrete_method_candidates(&receiver_candidates, field_name, |ty| {
                            self.resolve_projection_type(&self.engine.resolve(ty))
                        })
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

                let (base_expr, inner_ty) = if let Type::Reference { inner, .. } = &resolved_ty {
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
                    let method_subst = self.fresh_method_generic_subst(method_func, span.clone());
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
                                crate::type_services::display::short_item_name(&ident.name)
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
                    Type::Tuple(elems) if (*n as usize) < elems.len() => elems[*n as usize].clone(),
                    _ => self.engine.fresh_type_var_at(span.clone()),
                };

                HirExpr {
                    ty: elem_ty,
                    kind: HirExprKind::TupleIndex(Box::new(expr), *n as u32),
                    span,
                }
            }
        }
    }
}

use std::collections::{HashMap, HashSet};

use crate::hir::{HirCallTarget, HirExpr, HirExprKind, HirFunction, HirStaticMethodTarget};
use crate::lower::ResolveError;
use crate::selection::SelectionService;
use crate::types::Type;

use super::type_predicates::{try_carrier_has_unknown_head, try_type_head_is_unresolved};
use super::MethodAuthorityContext;

impl MethodAuthorityContext<'_> {
    pub(super) fn materialize_try(
        &mut self,
        expr: &mut HirExpr,
        errors: &mut Vec<ResolveError>,
        materialize_branch: bool,
        materialize_residual: bool,
    ) {
        let HirExprKind::Try {
            expr: operand,
            branch_method,
            branch_self_receiver,
            from_residual_target,
            output_ty,
            residual_ty,
            return_ty,
            ..
        } = &mut expr.kind
        else {
            unreachable!("Try materialization called for a non-Try expression");
        };

        self.materialize_expr(operand, errors);
        {
            let mut engine = self.engine.borrow_mut();
            let _ = engine.unify(&expr.ty, output_ty);
            expr.ty = engine.resolve(&expr.ty);
        }
        let Some(protocol) = self.try_protocol.as_ref() else {
            if self.try_strict && self.strict {
                errors.push(ResolveError::with_span(
                    "Cannot use '?' because the Try language-item protocol is unavailable"
                        .to_string(),
                    expr.span.clone(),
                ));
            }
            return;
        };
        let Some(try_trait) = self.traits.get(&protocol.try_trait_id) else {
            if self.try_strict && self.strict {
                errors.push(ResolveError::with_span(
                    "Cannot use '?' because the Try language-item protocol is unavailable"
                        .to_string(),
                    expr.span.clone(),
                ));
            }
            return;
        };

        if materialize_branch && branch_method.is_none() {
            let carrier_ty = self.resolved_type(&operand.ty);
            if try_carrier_has_unknown_head(&carrier_ty) {
                if self.try_strict {
                    errors.push(ResolveError::with_span(
                        "Cannot use '?' because its carrier type is unresolved or generic"
                            .to_string(),
                        expr.span.clone(),
                    ));
                }
                return;
            }

            let receiver = operand.as_ref().clone();
            let selected = match SelectionService::new(
                self.traits,
                self.impls,
                self.sized_trait_id,
                None,
                &self.bounds,
            )
            .with_effective_trait_methods(self.effective_trait_methods)
            .select_required_trait_member(
                &receiver,
                &carrier_ty,
                protocol.try_trait_id,
                protocol.branch_method_id,
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
                    if self.try_strict && self.strict {
                        errors.push(ResolveError::with_span(message, expr.span.clone()));
                    }
                    return;
                }
            };

            if let Some(function) = selected.function.as_ref() {
                if function.is_unsafe && !self.unsafe_context.get() {
                    if self.try_strict && self.strict {
                        errors.push(ResolveError::with_span(
                            format!(
                                "Call to unsafe function '{}' requires an unsafe block",
                                crate::type_services::display::short_item_name(&function.name)
                            ),
                            expr.span.clone(),
                        ));
                    }
                    return;
                }
            }

            let Some(selected_output) =
                self.selected_try_associated_type(&selected, try_trait, protocol.output_id)
            else {
                if self.try_strict && self.strict {
                    errors.push(ResolveError::with_span(
                        "Cannot use '?' because the Try language-item output type is unavailable"
                            .to_string(),
                        expr.span.clone(),
                    ));
                }
                return;
            };
            let Some(selected_residual) =
                self.selected_try_associated_type(&selected, try_trait, protocol.residual_id)
            else {
                if self.try_strict && self.strict {
                    errors.push(ResolveError::with_span(
                        "Cannot use '?' because the Try language-item residual type is unavailable"
                            .to_string(),
                        expr.span.clone(),
                    ));
                }
                return;
            };

            {
                let mut engine = self.engine.borrow_mut();
                if let Err(error) = engine.unify(output_ty, &selected_output) {
                    if self.try_strict && self.strict {
                        errors.push(ResolveError::with_span(
                            format!(
                                "Cannot use '?' because its output type could not be resolved: {}",
                                error.render(&engine)
                            ),
                            expr.span.clone(),
                        ));
                    }
                    return;
                }
                expr.ty = engine.resolve(output_ty);
                if let Err(error) = engine.unify(residual_ty, &selected_residual) {
                    if self.try_strict && self.strict {
                        errors.push(ResolveError::with_span(
                            format!(
                                "Cannot use '?' because its residual type could not be resolved: {}",
                                error.render(&engine)
                            ),
                            expr.span.clone(),
                        ));
                    }
                    return;
                }
            }
            let selected_residual = self.resolved_type(&selected_residual);
            if try_type_head_is_unresolved(&selected_residual) {
                if self.try_strict && self.strict {
                    errors.push(ResolveError::with_span(
                        "Cannot use '?' because its Try residual type is unresolved or generic"
                            .to_string(),
                        expr.span.clone(),
                    ));
                }
                return;
            }

            let method_substitution = selected
                .function
                .as_ref()
                .map(|function| {
                    self.infer_try_method_substitution(&selected, &carrier_ty, function, &[])
                })
                .unwrap_or_default();
            let method = selected.target_with_substitution(&method_substitution, |ty| {
                self.engine.borrow().resolve(ty)
            });
            *branch_method = Some(method);
            *branch_self_receiver = selected
                .function
                .as_ref()
                .and_then(|function| function.self_receiver);
        }

        if materialize_residual && from_residual_target.is_none() {
            let resolved_return_ty = self.resolved_type(return_ty);
            if try_type_head_is_unresolved(&resolved_return_ty) {
                if self.try_strict && self.strict {
                    errors.push(ResolveError::with_span(
                        "Cannot use '?' because its enclosing return type is unresolved or generic"
                            .to_string(),
                        expr.span.clone(),
                    ));
                }
                return;
            }
            let resolved_residual_ty = self.resolved_type(residual_ty);
            let return_owner = HirExpr {
                ty: resolved_return_ty.clone(),
                kind: HirExprKind::Unit,
                span: expr.span.clone(),
            };
            let Some(from_residual_trait) = self.traits.get(&protocol.from_residual_trait_id)
            else {
                if self.try_strict && self.strict {
                    errors.push(ResolveError::with_span(
                        "Cannot use '?' because the FromResidual language-item trait is unavailable"
                            .to_string(),
                        expr.span.clone(),
                    ));
                }
                return;
            };
            let residual_arg = HirExpr {
                ty: resolved_residual_ty.clone(),
                kind: HirExprKind::Unit,
                span: expr.span.clone(),
            };
            let selected = match SelectionService::new(
                self.traits,
                self.impls,
                self.sized_trait_id,
                None,
                &self.bounds,
            )
            .with_effective_trait_methods(self.effective_trait_methods)
            .select_static_trait_member(
                &return_owner,
                &resolved_return_ty,
                from_residual_trait.id,
                std::slice::from_ref(&resolved_residual_ty),
                protocol.from_residual_method_id,
            ) {
                Ok(selected) => selected,
                Err(error) => {
                    let message = match error {
                        crate::selection::SelectionDiagnostic::NoImplementation { .. } => {
                            let return_type = self.display_type(&resolved_return_ty);
                            let residual_type = self.display_type(&resolved_residual_ty);
                            format!(
                                "Cannot use '?' because {return_type} does not implement FromResidual {residual_type}"
                            )
                        }
                        _ => self.display_selection_error(&error),
                    };
                    if self.try_strict && self.strict {
                        errors.push(ResolveError::with_span(message, expr.span.clone()));
                    }
                    return;
                }
            };
            if let Some(function) = selected.function.as_ref() {
                if function.is_unsafe && !self.unsafe_context.get() {
                    if self.try_strict && self.strict {
                        errors.push(ResolveError::with_span(
                            format!(
                                "Call to unsafe function '{}' requires an unsafe block",
                                crate::type_services::display::short_item_name(&function.name)
                            ),
                            expr.span.clone(),
                        ));
                    }
                    return;
                }
            }
            let method_substitution = selected
                .function
                .as_ref()
                .map(|function| {
                    self.infer_try_method_substitution(
                        &selected,
                        &resolved_return_ty,
                        function,
                        &[residual_arg],
                    )
                })
                .unwrap_or_default();
            let method = selected.target_with_substitution(&method_substitution, |ty| {
                self.engine.borrow().resolve(ty)
            });
            *from_residual_target = Some(HirCallTarget::StaticMethod(HirStaticMethodTarget {
                owner_ty: resolved_return_ty,
                method,
            }));
        }

        let engine = self.engine.borrow();
        *output_ty = engine.resolve(output_ty);
        *residual_ty = engine.resolve(residual_ty);
        *return_ty = engine.resolve(return_ty);
        expr.ty = engine.resolve(&expr.ty);
    }

    fn selected_try_associated_type(
        &self,
        selected: &crate::selection::SelectedMethod,
        trait_def: &crate::hir::HirTrait,
        assoc_type_id: crate::ids::AssocTypeId,
    ) -> Option<Type> {
        trait_def
            .associated_types
            .iter()
            .find(|associated| associated.id == assoc_type_id)?;
        let ty = selected
            .associated_types
            .iter()
            .find(|associated| associated.id == assoc_type_id)
            .map(|associated| associated.ty.clone())?;
        Some(self.engine.borrow().resolve(&ty))
    }

    fn infer_try_method_substitution(
        &self,
        selected: &crate::selection::SelectedMethod,
        receiver_ty: &Type,
        function: &HirFunction,
        args: &[HirExpr],
    ) -> HashMap<crate::types::GenericParamId, Type> {
        let inferable = function
            .generic_params
            .iter()
            .map(|param| param.id)
            .collect::<HashSet<_>>();
        let mut engine = self.engine.borrow_mut();
        let mut substitution = function
            .generic_params
            .iter()
            .map(|param| {
                (
                    param.id,
                    engine
                        .fresh_type_var_at_kind(selected.receiver.span.clone(), param.kind.clone()),
                )
            })
            .collect::<HashMap<_, _>>();
        if function.is_method {
            if let Some(self_param) = function.params.first() {
                let expected = self_param.ty.substitute_generics(&substitution);
                let _ = engine.unify(&expected, receiver_ty);
                crate::selection::infer_generic_subst_from_types(
                    &self_param.ty,
                    receiver_ty,
                    &mut substitution,
                );
            }
        }
        let param_start = usize::from(function.is_method);
        for (param, arg) in function.params[param_start..].iter().zip(args) {
            let actual = engine.resolve(&arg.ty);
            crate::selection::infer_generic_subst_from_types(&param.ty, &actual, &mut substitution);
            let expected = param.ty.substitute_generics(&substitution);
            let _ = engine.unify(&expected, &arg.ty);
        }
        substitution.retain(|param, _| inferable.contains(param));
        for ty in substitution.values_mut() {
            *ty = engine.resolve(ty);
        }
        for (param, ty) in &selected.owner_substitution {
            substitution.insert(*param, ty.clone());
        }
        substitution
    }
}

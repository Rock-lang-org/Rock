//! Try (`?`) operator lowering.

use crate::hir::*;
use crate::ids::DefId;
use crate::lexer::Span;
use crate::lower::Lowerer;
use crate::types::{TraitBound, Type};
use std::collections::HashMap;

impl Lowerer {
    pub(super) fn lower_try_interogation(&mut self, mut expr: HirExpr, span: Span) -> HirExpr {
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
                        crate::type_services::display::short_item_name(&func.name)
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
                        crate::type_services::display::short_item_name(&func.name)
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
}

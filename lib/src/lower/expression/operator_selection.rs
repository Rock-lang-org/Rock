//! Operator method selection and unsafe-call reporting.

use crate::hir::*;
use crate::lexer::Span;
use crate::lower::Lowerer;
use crate::selection::{ReceiverCandidate, SelectedMethod, SelectedOrigin, SelectionDiagnostic};
use crate::types::Type;
use std::collections::HashMap;

impl Lowerer {
    pub(super) fn autoref_operator_arg_for_selected_method(
        &mut self,
        arg: HirExpr,
        selected: &crate::selection::SelectedMethod,
        index: usize,
    ) -> HirExpr {
        let Some(param) = selected.substituted_params.get(index) else {
            return arg;
        };
        let expected_ty = self.resolve_projection_type(&param.ty);
        let resolved_arg_ty = self.resolve_projection_type(&self.engine.resolve(&arg.ty));

        if matches!(expected_ty, Type::Reference { mutable: false, .. })
            && expected_ty != resolved_arg_ty
        {
            let span = arg.span.clone();
            return HirExpr {
                ty: Type::Reference {
                    mutable: false,
                    inner: Box::new(resolved_arg_ty),
                },
                kind: HirExprKind::Ref(false, Box::new(arg)),
                span,
            };
        }

        arg
    }

    pub(super) fn operator_method_accepts_args(
        &self,
        selected: &crate::selection::SelectedMethod,
        args: &[HirExpr],
    ) -> bool {
        if selected.substituted_params.len() != args.len() {
            return false;
        }

        // Bound arguments belong to the caller: U and V are distinct rigid
        // types even when both instantiate the same trait member.
        let rigid_generics = matches!(selected.origin, SelectedOrigin::TraitBound { .. });
        if !rigid_generics
            && selected
                .substituted_params
                .iter()
                .any(|param| Self::type_has_unresolved_parameter(&param.ty))
        {
            return true;
        }

        let mut subst = HashMap::new();
        selected
            .substituted_params
            .iter()
            .zip(args)
            .all(|(param, arg)| {
                let expected = self.resolve_projection_type(&param.ty);
                let actual = self.resolve_projection_type(&self.engine.resolve(&arg.ty));
                if !rigid_generics
                    && crate::selection::type_pattern_matches(&expected, &actual, &mut subst)
                {
                    return true;
                }
                if Self::type_accepts_inference_vars(&expected, &actual) {
                    return true;
                }
                if let Type::Reference {
                    inner,
                    mutable: false,
                } = expected
                {
                    return (!rigid_generics
                        && crate::selection::type_pattern_matches(&inner, &actual, &mut subst))
                        || Self::type_accepts_inference_vars(&inner, &actual);
                }
                false
            })
    }

    pub(super) fn report_unsafe_operator_method_call_if_needed(
        &mut self,
        method_func: &HirFunction,
        method_name: &str,
        span: Span,
    ) {
        if method_func.is_unsafe && !self.is_in_unsafe() {
            self.diagnostics.push_selection_with_span(
                format!(
                    "Call to unsafe function '{}' requires an unsafe block",
                    crate::type_services::display::short_item_name(method_name)
                ),
                span,
            );
        }
    }

    pub(super) fn report_unsafe_operator_function_call_if_needed(
        &mut self,
        target: Option<&HirVarTarget>,
        function_name: &str,
        span: Span,
    ) {
        let is_unsafe = match target {
            Some(HirVarTarget::Function(function_id)) => self
                .items
                .function(*function_id)
                .is_some_and(|func| func.is_unsafe),
            _ => self
                .resolve_module_alias_or_item_def_id(function_name)
                .and_then(|id| self.items.function(id))
                .is_some_and(|func| func.is_unsafe),
        };

        if is_unsafe && !self.is_in_unsafe() {
            self.diagnostics.push_selection_with_span(
                format!(
                    "Call to unsafe function '{}' requires an unsafe block",
                    crate::type_services::display::short_item_name(function_name)
                ),
                span,
            );
        }
    }

    pub(super) fn ambiguous_operator_candidates(
        engine: &crate::infer::InferenceEngine,
        receiver: &Type,
        operation: &str,
        candidates: &[(crate::ids::DefId, Type, Type)],
    ) -> Result<bool, SelectionDiagnostic> {
        if candidates.is_empty() {
            return Ok(false);
        }
        let mut candidate_ids = candidates
            .iter()
            .map(|(impl_id, _, _)| *impl_id)
            .collect::<Vec<_>>();
        candidate_ids.sort();
        candidate_ids.dedup();
        Err(SelectionDiagnostic::AmbiguousCandidates {
            operation: operation.to_string(),
            receiver: engine.resolve(receiver),
            candidates: candidate_ids,
        })
    }

    pub(super) fn ambiguous_unary_operator_candidates(
        engine: &crate::infer::InferenceEngine,
        receiver: &Type,
        operation: &str,
        candidates: &[(crate::ids::DefId, Type)],
    ) -> Result<bool, SelectionDiagnostic> {
        if candidates.is_empty() {
            return Ok(false);
        }
        let mut candidate_ids = candidates
            .iter()
            .map(|(impl_id, _)| *impl_id)
            .collect::<Vec<_>>();
        candidate_ids.sort();
        candidate_ids.dedup();
        Err(SelectionDiagnostic::AmbiguousCandidates {
            operation: operation.to_string(),
            receiver: engine.resolve(receiver),
            candidates: candidate_ids,
        })
    }

    pub(crate) fn select_operator_method(
        &mut self,
        receiver_candidates: &[ReceiverCandidate],
        method_name: &str,
        args: &[HirExpr],
    ) -> Result<Option<SelectedMethod>, SelectionDiagnostic> {
        let result = self.selection_service().select_concrete_method_matching(
            receiver_candidates,
            method_name,
            |ty| self.resolve_projection_type(&self.engine.resolve(ty)),
            |selected| self.operator_method_accepts_args(selected, args),
        );
        match result {
            Ok(selected) => Ok(Some(selected)),
            Err(SelectionDiagnostic::NoImplementation { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

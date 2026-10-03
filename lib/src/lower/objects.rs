use crate::hir::{HirExpr, HirExprKind, HirObjectCoercion};
use crate::types::{TraitBound, Type};

use super::Lowerer;

impl Lowerer {
    pub(crate) fn lower_expression_expected(
        &mut self,
        expression: &crate::ast::Expression,
        expected: &Type,
    ) -> HirExpr {
        if let crate::ast::Expression::UnaryExpr(crate::ast::UnaryExpr::PrimaryExpr(primary)) =
            expression
        {
            if primary
                .secondaries
                .as_ref()
                .is_none_or(|items| items.is_empty())
                && primary.type_annotation.is_none()
            {
                match &primary.operand {
                    crate::ast::Operand::Expression(inner) => {
                        return self.lower_expression_expected(inner, expected)
                    }
                    crate::ast::Operand::If(branch) => {
                        return self.lower_if_expected(branch, Some(expected))
                    }
                    crate::ast::Operand::Open(open) => {
                        return self.lower_open_expected(open, Some(expected))
                    }
                    crate::ast::Operand::Match(branch) => {
                        return self.lower_match_expected(branch, Some(expected))
                    }
                    _ => {}
                }
            }
        }
        let value = self.lower_expression(expression);
        self.coerce_argument_to_expected(value, expected)
    }

    pub(crate) fn coerce_borrowed_object(
        &mut self,
        arg: HirExpr,
        target: &Type,
    ) -> Option<HirExpr> {
        let source = self.engine.resolve(&arg.ty);
        if source == *target {
            return None;
        }
        let target_object = matches!(target, Type::Reference { inner, .. } | Type::Pointer(inner) if matches!(inner.as_ref(), Type::Object(_)));
        let unknown_handle = matches!(target, Type::TypeVar(_))
            || matches!(target, Type::Reference { inner, .. } if matches!(inner.as_ref(), Type::TypeVar(_)));
        let potential_source_handle = matches!(source, Type::Reference { .. })
            || (matches!(source, Type::TypeVar(_)) && matches!(target, Type::Reference { .. }));
        if !target_object && !(unknown_handle && potential_source_handle) {
            return None;
        }
        Some(HirExpr {
            span: arg.span.clone(),
            ty: target.clone(),
            kind: HirExprKind::ObjectCoercion(
                Box::new(arg),
                HirObjectCoercion {
                    target: target.clone(),
                    evidence: None,
                },
            ),
        })
    }

    pub(crate) fn object_method_candidate(
        &mut self,
        receiver: HirExpr,
        name: &str,
    ) -> Option<crate::selection::SelectedMethod> {
        let ty = self.engine.resolve(&receiver.ty);
        let object_ty = match &ty {
            Type::Reference { inner, .. } => inner.as_ref(),
            ty => ty,
        };
        let (object, witness) = match object_ty {
            Type::Object(object) => (object.as_ref().clone(), None),
            Type::Witness(witness) => (self.opened_object_schema(*witness)?, Some(*witness)),
            _ => return None,
        };
        let bound_object = if let Some(witness) = witness {
            object
                .try_map_types(|ty| {
                    crate::type_services::substitution::instantiate_object_self(
                        ty,
                        &Type::Witness(witness),
                    )
                })
                .ok()?
        } else {
            object.clone()
        };
        let bounds: Vec<TraitBound> = std::iter::once(bound_object.principal.clone())
            .chain(bound_object.guarantees.iter().cloned())
            .collect();
        let candidates = self.receiver_adjustment_candidates(receiver.clone());
        let result = self.selection_service().select_bound_method(
            &candidates,
            &bounds,
            name,
            object_ty.clone(),
        );
        let mut selected = self.handle_optional_selection(result, receiver.span.clone())?;
        let adapted = if let Some(witness) = witness {
            crate::hir::object_methods::adapt_opened(
                &mut selected,
                self.items.traits_for_selection(),
                &object,
                witness,
            )
        } else {
            crate::hir::object_methods::adapt(
                &mut selected,
                self.items.traits_for_selection(),
                &object,
            )
        };
        if let Err(message) = adapted {
            self.diagnostics.push_type_with_span(message, receiver.span);
            return None;
        }
        Some(selected)
    }

    pub(crate) fn shared_package_member_requires_mutable_access(
        &mut self,
        receiver: &HirExpr,
        name: &str,
    ) -> bool {
        let Type::Reference {
            mutable: false,
            inner,
        } = self.engine.resolve(&receiver.ty)
        else {
            return false;
        };
        let Type::Witness(witness) = inner.as_ref() else {
            return false;
        };
        let Some((_, binding)) = self
            .opened_packages
            .iter()
            .rev()
            .find(|(_, binding)| binding.witness == *witness)
        else {
            return false;
        };
        let Ok(object) = binding.object_schema().and_then(|object| {
            object
                .try_map_types(|ty| {
                    crate::type_services::substitution::instantiate_object_self(ty, inner.as_ref())
                })
                .map_err(|error| error.to_string())
        }) else {
            return false;
        };
        let bounds = std::iter::once(object.principal)
            .chain(object.guarantees)
            .collect::<Vec<_>>();
        let mut candidates = self.receiver_adjustment_candidates(receiver.clone());
        let service = self.selection_service();
        if !matches!(
            service.select_bound_method(&candidates, &bounds, name, inner.as_ref().clone()),
            Err(crate::selection::SelectionDiagnostic::NoImplementation { .. })
        ) {
            return false;
        }
        // Diagnosis-only probe of the same bound member, never executable
        // selection authority or a grant of mutable receiver capability.
        for candidate in &mut candidates {
            candidate.can_autoref_mut = true;
        }
        service
            .select_bound_method(&candidates, &bounds, name, inner.as_ref().clone())
            .is_ok_and(|selected| {
                selected.function.as_ref().is_some_and(|function| {
                    function.self_receiver == Some(crate::types::ReceiverMode::Mut)
                })
            })
    }
}

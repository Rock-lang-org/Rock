use crate::hir::{HirExpr, HirExprKind, HirFieldLocation};
use crate::lower::ResolveError;
use crate::type_services::projection::ProjectionNormalizer;
use crate::types::Type;

use super::type_predicates::contains_inference_type;
use super::MethodAuthorityContext;

impl MethodAuthorityContext<'_> {
    /// When `receiver.method_name` names no field but an impl provides the
    /// method under unmet `where` bounds, report the unmet bound instead of
    /// letting the call fall back to an "unknown field" error.
    pub(super) fn unsatisfied_method_error(
        &mut self,
        receiver: &HirExpr,
        method_name: &str,
        span: &crate::Span,
    ) -> Option<ResolveError> {
        let receiver_ty = self.resolved_type(&receiver.ty);
        let struct_ty = match &receiver_ty {
            Type::Reference { inner, .. } => inner.as_ref(),
            ty => ty,
        };
        if let Type::Struct { id, .. } = struct_ty {
            let has_field = self
                .structs
                .get(id)
                .is_some_and(|structure| structure.fields.iter().any(|f| f.name == method_name));
            if has_field {
                return None;
            }
        }
        let candidates = self.receiver_adjustment_candidates(receiver.clone());
        let explained =
            self.service()
                .explain_unsatisfied_method(&candidates, method_name, |ty| {
                    self.resolved_type(ty)
                })?;

        let provider = explained
            .method_trait
            .as_deref()
            .map(|name| format!(" from trait `{}`", name.rsplit("::").next().unwrap_or(name)))
            .unwrap_or_default();
        let mut message = format!(
            "method `{method_name}`{provider} exists for `{}`, but its requirements are not met",
            self.display_type(&explained.receiver)
        );
        for (index, (ty, trait_name)) in explained.chain.iter().enumerate() {
            let trait_name = trait_name.rsplit("::").next().unwrap_or(trait_name);
            let lead = if index == 0 {
                ": requires"
            } else {
                ", which requires"
            };
            let shown = self.display_type(ty);
            // `Result A, E: Show` would read as two bounds; group applied types.
            let shown = if shown.contains(' ') {
                format!("({shown})")
            } else {
                shown
            };
            message.push_str(&format!("{lead} `{shown}: {trait_name}`"));
        }
        if let Some((ty, trait_name)) = explained.chain.last() {
            let trait_name = trait_name.rsplit("::").next().unwrap_or(trait_name);
            message.push_str(&format!(
                "; `{}` does not implement `{trait_name}`",
                self.display_type(ty)
            ));
        }
        Some(ResolveError::with_span_code(
            message,
            span.clone(),
            crate::diagnostic::DiagnosticCode::Selection,
        ))
    }

    /// Binds a deferred dereference (`*x` whose operand was unresolved while
    /// lowering) to the pointee once callers have seeded the operand.
    pub(super) fn materialize_deref(&mut self, expr: &mut HirExpr, errors: &mut Vec<ResolveError>) {
        let HirExprKind::Deref(operand) = &mut expr.kind else {
            return;
        };
        self.materialize_expr(operand, errors);
        let placeholder = self.engine.borrow().resolve(&expr.ty);
        if !matches!(placeholder, Type::TypeVar(_)) {
            return;
        }
        let operand_ty = self.resolved_type(&operand.ty);
        let pointee = match operand_ty {
            Type::Reference { inner, .. } => *inner,
            Type::Pointer(inner) => {
                if !self.unsafe_context.get() {
                    errors.push(ResolveError::with_span_code(
                        "Dereference of raw pointer requires an unsafe block".to_string(),
                        expr.span.clone(),
                        crate::diagnostic::DiagnosticCode::Selection,
                    ));
                }
                *inner
            }
            ty if contains_inference_type(&ty) => return,
            ty => {
                errors.push(ResolveError::with_span(
                    format!(
                        "Cannot dereference non-pointer type: {}",
                        self.display_type(&ty)
                    ),
                    expr.span.clone(),
                ));
                return;
            }
        };
        let _ = self.engine.borrow_mut().unify(&placeholder, &pointee);
        expr.ty = pointee;
    }

    pub(super) fn materialize_field_access(
        &mut self,
        expr: &mut HirExpr,
        errors: &mut Vec<ResolveError>,
    ) {
        let HirExprKind::FieldAccess(receiver, field_name, location) = &mut expr.kind else {
            return;
        };
        self.materialize_expr(receiver, errors);
        let normalized = self.engine.borrow().normalize_resolved_type(&receiver.ty);
        receiver.ty = match normalized {
            Ok(ty) => ProjectionNormalizer::normalize(&self.service(), &ty),
            Err(message) => {
                errors.push(ResolveError::with_span(message, receiver.span.clone()));
                return;
            }
        };
        if let Type::Reference { inner, .. } = &receiver.ty {
            let dereferenced = HirExpr {
                ty: inner.as_ref().clone(),
                kind: HirExprKind::Deref(Box::new(receiver.as_ref().clone())),
                span: receiver.span.clone(),
            };
            **receiver = dereferenced;
        }
        let receiver_ty = &receiver.ty;
        let Type::Struct { id, args } = receiver_ty else {
            if !contains_inference_type(receiver_ty) {
                errors.push(ResolveError::with_span(
                    format!(
                        "Unknown field '{}' on type '{}'",
                        field_name,
                        self.display_type(receiver_ty)
                    ),
                    expr.span.clone(),
                ));
            }
            return;
        };
        let Some(structure) = self.structs.get(id) else {
            return;
        };
        let Some(field) = structure
            .fields
            .iter()
            .find(|field| match location.as_ref() {
                Some(location) => location.owner == *id && location.field_id == field.id,
                None => field.name == *field_name,
            })
        else {
            errors.push(ResolveError::with_span(
                format!(
                    "Unknown field '{}' on struct '{}'",
                    field_name, structure.name
                ),
                expr.span.clone(),
            ));
            return;
        };
        if !field.public && location.is_none() {
            if !self.strict {
                return;
            }
            errors.push(ResolveError::with_span(
                format!(
                    "field '{}' of struct '{}' is private",
                    field_name, structure.name
                ),
                expr.span.clone(),
            ));
            return;
        }
        *location = Some(HirFieldLocation {
            owner: *id,
            field_id: field.id,
            name: field.name.clone(),
        });
        let substitution = structure
            .generic_params
            .iter()
            .enumerate()
            .filter_map(|(index, _)| {
                args.get(index).cloned().map(|ty| {
                    (
                        crate::types::GenericParamId {
                            owner: *id,
                            index: index as u32,
                        },
                        ty,
                    )
                })
            })
            .collect();
        let field_ty = field.ty.substitute_generics(&substitution);
        // An access typed with a bare placeholder variable may share it with
        // other sites (a method receiver, a binding); bind it, don't just
        // replace it. A callee-shaped placeholder (`() -> _`) is left alone:
        // the field may be a callable struct rather than a function.
        let placeholder = self.engine.borrow().resolve(&expr.ty);
        if matches!(placeholder, Type::TypeVar(_)) {
            let _ = self.engine.borrow_mut().unify(&placeholder, &field_ty);
        }
        expr.ty = field_ty;
    }
}

use crate::hir::{HirExpr, HirExprKind, HirFieldLocation};
use crate::lower::ResolveError;
use crate::types::Type;

use super::type_predicates::contains_inference_type;
use super::MethodAuthorityContext;

impl MethodAuthorityContext<'_> {
    pub(super) fn materialize_field_access(
        &mut self,
        expr: &mut HirExpr,
        errors: &mut Vec<ResolveError>,
    ) {
        let HirExprKind::FieldAccess(receiver, field_name, location) = &mut expr.kind else {
            return;
        };
        self.materialize_expr(receiver, errors);
        receiver.ty = match self.engine.borrow().normalize_resolved_type(&receiver.ty) {
            Ok(ty) => ty,
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
        expr.ty = field.ty.substitute_generics(&substitution);
    }
}

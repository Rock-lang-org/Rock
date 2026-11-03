use std::collections::HashSet;

use crate::hir::{
    HirBlock, HirClosureCapture, HirClosureCaptureKind, HirExpr, HirExprKind, HirFieldLocation,
    HirStmt, HirVarRef, HirVarTarget,
};
use crate::ids::HirLocalId;
use crate::lower::ResolveError;
use crate::selection::SelectedMethod;
use crate::types::{CallableKind, CaptureKind, FunctionCapture, FunctionSafety, Type};

use super::type_predicates::contains_inference_type;
use super::MethodAuthorityContext;

impl MethodAuthorityContext<'_> {
    #[allow(dead_code)]
    fn fresh_local_id(&mut self) -> HirLocalId {
        let id = HirLocalId(self.next_local_id);
        self.next_local_id = self.next_local_id.saturating_add(1);
        id
    }

    #[allow(dead_code)]
    fn lambda_function_type(
        &self,
        params: Vec<Type>,
        ret: Type,
        safety: FunctionSafety,
        captures: &[HirClosureCapture],
    ) -> Type {
        let captures = captures
            .iter()
            .map(|capture| FunctionCapture {
                kind: match capture.kind {
                    HirClosureCaptureKind::SharedBorrow => CaptureKind::SharedBorrow,
                    HirClosureCaptureKind::MutableBorrow => CaptureKind::MutableBorrow,
                    HirClosureCaptureKind::Move => CaptureKind::Move,
                },
                ty: capture.ty.clone(),
            })
            .collect::<Vec<_>>();
        Type::function_with_metadata(
            params,
            ret,
            safety,
            CallableKind::from_captures(&captures),
            captures,
        )
    }

    #[allow(dead_code)]
    fn collect_method_value_captures(
        &self,
        expr: &HirExpr,
        receiver_mode: Option<crate::types::ReceiverMode>,
        captures: &mut Vec<HirClosureCapture>,
        seen: &mut HashSet<HirLocalId>,
    ) {
        match &expr.kind {
            HirExprKind::ResolvedVar(HirVarRef {
                target: HirVarTarget::Local(local_id),
                ..
            }) => {
                let Some((name, ty, mutable)) = self.local_bindings.get(local_id) else {
                    return;
                };
                if !seen.insert(*local_id) {
                    return;
                }
                let kind = match receiver_mode {
                    Some(crate::types::ReceiverMode::Move) => HirClosureCaptureKind::Move,
                    Some(crate::types::ReceiverMode::Mut) => HirClosureCaptureKind::MutableBorrow,
                    Some(crate::types::ReceiverMode::Shared) | None => {
                        HirClosureCaptureKind::SharedBorrow
                    }
                };
                captures.push(HirClosureCapture {
                    name: name.clone(),
                    local_id: *local_id,
                    kind,
                    mutable: *mutable,
                    ty: ty.clone(),
                });
            }
            HirExprKind::FieldAccess(base, _, _)
            | HirExprKind::TupleIndex(base, _)
            | HirExprKind::Deref(base)
            | HirExprKind::Ref(_, base)
            | HirExprKind::Cast(base, _) => {
                self.collect_method_value_captures(base, receiver_mode, captures, seen)
            }
            _ => {}
        }
    }

    #[allow(dead_code)]
    fn materialize_method_value(
        &mut self,
        expr: &mut HirExpr,
        receiver: HirExpr,
        method_name: String,
        selected: SelectedMethod,
        errors: &mut Vec<ResolveError>,
    ) {
        let Some(function) = selected.function.clone() else {
            errors.push(ResolveError::with_span(
                format!("selected method '{}' has no executable body", method_name),
                expr.span.clone(),
            ));
            return;
        };
        let mut substitution = selected.owner_substitution.clone();
        for param in &function.generic_params {
            substitution.entry(param.id).or_insert_with(|| {
                self.engine
                    .borrow_mut()
                    .fresh_type_var_at_kind(expr.span.clone(), param.kind.clone())
            });
        }
        let mut lambda_params = Vec::new();
        for param in &selected.substituted_params {
            let mut param = param.clone();
            param.local_id = self.fresh_local_id();
            param.ty = param.ty.substitute_generics(&substitution);
            lambda_params.push(param);
        }
        let call_args = lambda_params
            .iter()
            .map(|param| HirExpr {
                ty: param.ty.clone(),
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: param.name.clone(),
                    target: HirVarTarget::Local(param.local_id),
                }),
                span: expr.span.clone(),
            })
            .collect::<Vec<_>>();
        let call_ret = self.resolved_type(&selected.return_type.substitute_generics(&substitution));
        self.record_selected_bounds(&selected, &substitution, "deferred method value");
        let target = selected.target_with_substitution(&substitution, |ty| self.resolved_type(ty));
        let adjusted_receiver =
            self.apply_receiver_adjustment(selected.receiver.clone(), selected.receiver_adjustment);
        let call = HirExpr {
            ty: call_ret.clone(),
            kind: HirExprKind::MethodCall(
                Box::new(adjusted_receiver),
                method_name,
                call_args,
                function.self_receiver,
                Some(target),
            ),
            span: expr.span.clone(),
        };
        let body = HirBlock {
            ty: call_ret.clone(),
            stmts: vec![HirStmt::Expr(call)],
        };
        let mut captures = Vec::new();
        self.collect_method_value_captures(
            &receiver,
            function.self_receiver,
            &mut captures,
            &mut HashSet::new(),
        );
        let params = lambda_params.iter().map(|param| param.ty.clone()).collect();
        expr.ty = self.lambda_function_type(
            params,
            call_ret.clone(),
            FunctionSafety::from_is_unsafe(function.is_unsafe),
            &captures,
        );
        expr.kind = HirExprKind::Lambda {
            params: lambda_params,
            body,
            captures,
        };
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

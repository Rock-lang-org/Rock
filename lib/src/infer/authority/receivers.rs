use std::collections::{HashMap, HashSet};

use crate::hir::{HirExpr, HirExprKind, HirVarRef, HirVarTarget};
use crate::selection::{ReceiverAdjustment, ReceiverCandidate};
use crate::types::Type;

use super::type_predicates::contains_recovery_type;
use super::MethodAuthorityContext;

impl MethodAuthorityContext<'_> {
    pub(super) fn resolved_type(&self, ty: &Type) -> Type {
        let engine = self.engine.borrow();
        engine
            .normalize_resolved_type(ty)
            .unwrap_or_else(|_| engine.resolve(ty))
    }

    fn expr_can_autoref_mut_receiver(&self, expr: &HirExpr) -> bool {
        match &expr.kind {
            HirExprKind::ResolvedVar(HirVarRef {
                target: HirVarTarget::Local(local_id),
                ..
            }) => {
                self.mutable_locals.contains(local_id)
                    && !matches!(self.resolved_type(&expr.ty), Type::Reference { .. })
            }
            HirExprKind::FieldAccess(base, _, _) | HirExprKind::TupleIndex(base, _) => {
                self.expr_can_autoref_mut_receiver(base)
            }
            HirExprKind::Deref(inner) => matches!(
                self.resolved_type(&inner.ty),
                Type::Reference { mutable: true, .. }
            ),
            _ => false,
        }
    }

    fn is_borrowable_local_expr(expr: &HirExpr) -> bool {
        matches!(
            expr.kind,
            HirExprKind::FieldAccess(_, _, _)
                | HirExprKind::TupleIndex(_, _)
                // Place produced by dereferencing a reference, such as the
                // result of an index operation on a nested array.
                | HirExprKind::Deref(_)
                | HirExprKind::ResolvedVar(HirVarRef {
                    target: HirVarTarget::Local(_),
                    ..
                })
        )
    }

    fn push_receiver_candidate(
        &self,
        candidates: &mut Vec<ReceiverCandidate>,
        seen: &mut HashSet<Type>,
        expr: HirExpr,
        adjustment: ReceiverAdjustment,
        can_autoref_mut: bool,
    ) {
        let resolved = self.resolved_type(&expr.ty);
        if seen.insert(resolved) {
            candidates.push(ReceiverCandidate {
                expr,
                adjustment,
                can_autoref_mut,
            });
        }
    }

    fn builtin_shared_ref_deref(&self, expr: HirExpr) -> Option<HirExpr> {
        let Type::Reference {
            mutable: false,
            inner,
        } = self.resolved_type(&expr.ty)
        else {
            return None;
        };
        Some(HirExpr {
            ty: *inner,
            kind: HirExprKind::Deref(Box::new(expr.clone())),
            span: expr.span,
        })
    }

    fn builtin_mut_ref_deref(&self, expr: HirExpr) -> Option<HirExpr> {
        let Type::Reference {
            mutable: true,
            inner,
        } = self.resolved_type(&expr.ty)
        else {
            return None;
        };
        Some(HirExpr {
            ty: *inner,
            kind: HirExprKind::Deref(Box::new(expr.clone())),
            span: expr.span,
        })
    }

    fn mut_ref_to_shared_ref(&self, expr: HirExpr) -> Option<HirExpr> {
        let Type::Reference {
            mutable: true,
            inner,
        } = self.resolved_type(&expr.ty)
        else {
            return None;
        };
        Some(HirExpr {
            ty: Type::Reference {
                mutable: false,
                inner,
            },
            kind: expr.kind,
            span: expr.span,
        })
    }

    pub(super) fn array_ref_to_slice_ref(&self, expr: HirExpr, mutable: bool) -> Option<HirExpr> {
        let Type::Reference { inner, .. } = self.resolved_type(&expr.ty) else {
            return None;
        };
        let Type::Array(element, _) = inner.as_ref() else {
            return None;
        };
        let span = expr.span.clone();
        Some(HirExpr {
            ty: Type::Reference {
                mutable,
                inner: Box::new(Type::Slice(element.clone())),
            },
            kind: HirExprKind::Intrinsic {
                name: "ArrayRefToSlice".to_string(),
                args: vec![expr],
            },
            span,
        })
    }

    fn array_value_to_slice_ref(&self, expr: HirExpr, mutable: bool) -> Option<HirExpr> {
        if !Self::is_borrowable_local_expr(&expr) {
            return None;
        }
        let Type::Array(element, _) = self.resolved_type(&expr.ty) else {
            return None;
        };
        let span = expr.span.clone();
        let borrowed = HirExpr {
            ty: Type::Reference {
                mutable,
                inner: Box::new(expr.ty.clone()),
            },
            kind: HirExprKind::Ref(mutable, Box::new(expr)),
            span: span.clone(),
        };
        Some(HirExpr {
            ty: Type::Reference {
                mutable,
                inner: Box::new(Type::Slice(element)),
            },
            kind: HirExprKind::Intrinsic {
                name: "ArrayRefToSlice".to_string(),
                args: vec![borrowed],
            },
            span,
        })
    }

    pub(super) fn apply_receiver_adjustment(
        &self,
        expr: HirExpr,
        adjustment: ReceiverAdjustment,
    ) -> HirExpr {
        match adjustment {
            ReceiverAdjustment::AutorefShared | ReceiverAdjustment::AutorefMut => {
                let resolved = self.resolved_type(&expr.ty);
                if !matches!(resolved, Type::Str | Type::Slice(_)) {
                    return expr;
                }
                let mutable = matches!(adjustment, ReceiverAdjustment::AutorefMut);
                let span = expr.span.clone();
                HirExpr {
                    ty: Type::Reference {
                        mutable,
                        inner: Box::new(expr.ty.clone()),
                    },
                    kind: HirExprKind::Ref(mutable, Box::new(expr)),
                    span,
                }
            }
            ReceiverAdjustment::MutToSharedRef => {
                self.mut_ref_to_shared_ref(expr.clone()).unwrap_or(expr)
            }
            ReceiverAdjustment::None
            | ReceiverAdjustment::BuiltinDeref
            | ReceiverAdjustment::TraitDeref
            | ReceiverAdjustment::ArrayRefToSliceRef
            | ReceiverAdjustment::ArrayValueToMutSliceRef
            | ReceiverAdjustment::ArrayValueToSliceRef
            | ReceiverAdjustment::ArrayValueToSliceValue => expr,
        }
    }

    pub(super) fn trait_deref_candidate(&mut self, expr: HirExpr) -> Option<HirExpr> {
        let can_autoref_mut = self.expr_can_autoref_mut_receiver(&expr);
        let candidate = ReceiverCandidate {
            expr: HirExpr {
                ty: self.resolved_type(&expr.ty),
                ..expr.clone()
            },
            adjustment: ReceiverAdjustment::None,
            can_autoref_mut,
        };
        let mut selected = self.service().select_concrete_method_candidates(
            std::slice::from_ref(&candidate),
            "*",
            |ty| self.resolved_type(ty),
        );
        if can_autoref_mut {
            let has_mutable = selected.iter().any(|candidate| {
                candidate.function.as_ref().is_some_and(|function| {
                    function.self_receiver == Some(crate::types::ReceiverMode::Mut)
                })
            });
            if has_mutable {
                selected.retain(|candidate| {
                    candidate.function.as_ref().is_some_and(|function| {
                        function.self_receiver == Some(crate::types::ReceiverMode::Mut)
                    })
                });
            }
        }
        if selected.len() != 1 {
            return None;
        }
        let selected = selected.pop().expect("one deref candidate");
        let function = selected.function.clone()?;
        let return_type = self.resolved_type(&selected.return_type);
        let Type::Reference { inner, .. } = &return_type else {
            return None;
        };
        self.record_selected_bounds(&selected, &HashMap::new(), "trait deref");
        let receiver =
            self.apply_receiver_adjustment(selected.receiver.clone(), selected.receiver_adjustment);
        let target =
            selected.target_with_substitution(&HashMap::new(), |ty| self.resolved_type(ty));
        Some(HirExpr {
            ty: *inner.clone(),
            kind: HirExprKind::Deref(Box::new(HirExpr {
                ty: return_type,
                kind: HirExprKind::MethodCall(
                    Box::new(receiver),
                    function.name,
                    Vec::new(),
                    function.self_receiver,
                    Some(target),
                ),
                span: expr.span.clone(),
            })),
            span: expr.span,
        })
    }

    pub(super) fn receiver_adjustment_candidates(
        &mut self,
        expr: HirExpr,
    ) -> Vec<ReceiverCandidate> {
        let mut candidates = Vec::new();
        let mut seen = HashSet::new();
        let can_autoref_mut = self.expr_can_autoref_mut_receiver(&expr);
        self.push_receiver_candidate(
            &mut candidates,
            &mut seen,
            expr.clone(),
            ReceiverAdjustment::None,
            can_autoref_mut,
        );
        if let Some(mut_slice) = self.array_value_to_slice_ref(expr.clone(), true) {
            self.push_receiver_candidate(
                &mut candidates,
                &mut seen,
                mut_slice,
                ReceiverAdjustment::ArrayValueToMutSliceRef,
                false,
            );
        }
        if let Some(deref) = self.builtin_shared_ref_deref(expr.clone()) {
            self.push_receiver_candidate(
                &mut candidates,
                &mut seen,
                deref,
                ReceiverAdjustment::BuiltinDeref,
                false,
            );
        }
        if let Some(deref) = self.builtin_mut_ref_deref(expr.clone()) {
            self.push_receiver_candidate(
                &mut candidates,
                &mut seen,
                deref,
                ReceiverAdjustment::BuiltinDeref,
                true,
            );
        }
        if let Some(shared_ref) = self.mut_ref_to_shared_ref(expr.clone()) {
            self.push_receiver_candidate(
                &mut candidates,
                &mut seen,
                shared_ref.clone(),
                ReceiverAdjustment::MutToSharedRef,
                false,
            );
            if let Some(deref) = self.builtin_shared_ref_deref(shared_ref.clone()) {
                self.push_receiver_candidate(
                    &mut candidates,
                    &mut seen,
                    deref,
                    ReceiverAdjustment::BuiltinDeref,
                    false,
                );
            }
            if let Some(slice_ref) = self.array_ref_to_slice_ref(shared_ref, false) {
                self.push_receiver_candidate(
                    &mut candidates,
                    &mut seen,
                    slice_ref.clone(),
                    ReceiverAdjustment::ArrayRefToSliceRef,
                    false,
                );
                if let Some(slice_value) = self.builtin_shared_ref_deref(slice_ref) {
                    self.push_receiver_candidate(
                        &mut candidates,
                        &mut seen,
                        slice_value,
                        ReceiverAdjustment::BuiltinDeref,
                        false,
                    );
                }
            }
        }
        if !contains_recovery_type(&self.resolved_type(&expr.ty)) {
            if let Some(deref) = self.trait_deref_candidate(expr.clone()) {
                self.push_receiver_candidate(
                    &mut candidates,
                    &mut seen,
                    deref,
                    ReceiverAdjustment::TraitDeref,
                    can_autoref_mut,
                );
            }
        }
        if let Some(slice_ref) = self.array_ref_to_slice_ref(expr.clone(), false) {
            self.push_receiver_candidate(
                &mut candidates,
                &mut seen,
                slice_ref,
                ReceiverAdjustment::ArrayRefToSliceRef,
                false,
            );
        }
        if let Some(slice_ref) = self.array_value_to_slice_ref(expr.clone(), false) {
            self.push_receiver_candidate(
                &mut candidates,
                &mut seen,
                slice_ref,
                ReceiverAdjustment::ArrayValueToSliceRef,
                false,
            );
        }
        if let Some(slice_ref) = self.array_value_to_slice_ref(expr, false) {
            let span = slice_ref.span.clone();
            let slice_value = HirExpr {
                ty: match self.resolved_type(&slice_ref.ty) {
                    Type::Reference { inner, .. } => *inner,
                    ty => ty,
                },
                kind: HirExprKind::Deref(Box::new(slice_ref)),
                span,
            };
            self.push_receiver_candidate(
                &mut candidates,
                &mut seen,
                slice_value,
                ReceiverAdjustment::ArrayValueToSliceValue,
                false,
            );
        }
        candidates
    }
}

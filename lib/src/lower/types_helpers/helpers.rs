//! Type helper methods for Lowerer

use std::collections::{HashMap, HashSet};

use crate::hir::{HirExpr, HirExprKind, HirFunction, HirVarRef, HirVarTarget};
use crate::selection::{ReceiverAdjustment, ReceiverCandidate};
use crate::type_services::projection::{
    ProjectionAssociatedType, ProjectionImpl, ProjectionNormalizer, ProjectionProvider,
};
use crate::types::{GenericParamId, Type};

use crate::lower::Lowerer;

impl Lowerer {
    fn is_borrowable_local_expr(kind: &HirExprKind) -> bool {
        matches!(
            kind,
            HirExprKind::Var(_)
                | HirExprKind::FieldAccess(_, _, _)
                | HirExprKind::ResolvedVar(HirVarRef {
                    target: HirVarTarget::Local(_),
                    ..
                })
        )
    }

    pub(crate) fn expr_is_mutable_lvalue(&self, expr: &HirExpr) -> bool {
        match &expr.kind {
            HirExprKind::Var(name) => self
                .scope
                .lookup(name)
                .is_some_and(|binding| binding.mutable),
            HirExprKind::ResolvedVar(HirVarRef {
                name,
                target: HirVarTarget::Local(_),
            }) => self
                .scope
                .lookup(name)
                .is_some_and(|binding| binding.mutable),
            HirExprKind::FieldAccess(base, _, _) | HirExprKind::TupleIndex(base, _) => {
                self.expr_is_mutable_lvalue(base)
            }
            HirExprKind::Deref(inner) => matches!(
                self.resolve_projection_type(&self.engine.resolve(&inner.ty)),
                Type::Reference { mutable: true, .. }
            ),
            _ => false,
        }
    }

    pub(crate) fn expr_can_autoref_mut_receiver(&self, expr: &HirExpr) -> bool {
        match &expr.kind {
            HirExprKind::Var(name) => {
                self.scope
                    .lookup(name)
                    .is_some_and(|binding| binding.mutable)
                    && !matches!(
                        self.resolve_projection_type(&self.engine.resolve(&expr.ty)),
                        Type::Reference { .. }
                    )
            }
            HirExprKind::ResolvedVar(HirVarRef {
                name,
                target: HirVarTarget::Local(_),
            }) => {
                self.scope
                    .lookup(name)
                    .is_some_and(|binding| binding.mutable)
                    && !matches!(
                        self.resolve_projection_type(&self.engine.resolve(&expr.ty)),
                        Type::Reference { .. }
                    )
            }
            HirExprKind::ResolvedVar(_) => false,
            HirExprKind::FieldAccess(base, _, _) | HirExprKind::TupleIndex(base, _) => {
                self.expr_can_autoref_mut_receiver(base)
            }
            HirExprKind::Deref(inner) => matches!(
                self.resolve_projection_type(&self.engine.resolve(&inner.ty)),
                Type::Reference { mutable: true, .. }
            ),
            _ => false,
        }
    }

    #[cfg(test)]
    pub(crate) fn get_type_name_for_method_lookup_in_context(&self, ty: &Type) -> Option<String> {
        crate::lower::resolution::LowerResolutionContext::new(self)
            .canonical_type_name_for_method_lookup(ty)
    }

    #[cfg(test)]
    fn type_pattern_matches_after_subst(pattern: &Type, actual: &Type) -> bool {
        match (pattern, actual) {
            (
                Type::Struct {
                    id: expected_id,
                    args: expected_args,
                },
                Type::Struct {
                    id: actual_id,
                    args: actual_args,
                },
            )
            | (
                Type::Enum {
                    id: expected_id,
                    args: expected_args,
                },
                Type::Enum {
                    id: actual_id,
                    args: actual_args,
                },
            ) => {
                expected_args.len() == actual_args.len()
                    && expected_id == actual_id
                    && expected_args
                        .iter()
                        .zip(actual_args.iter())
                        .all(|(expected, actual)| {
                            Self::type_pattern_matches_after_subst(expected, actual)
                        })
            }
            (
                Type::Reference {
                    mutable: left_mut,
                    inner: left,
                },
                Type::Reference {
                    mutable: right_mut,
                    inner: right,
                },
            ) => left_mut == right_mut && Self::type_pattern_matches_after_subst(left, right),
            (Type::Slice(left), Type::Slice(right))
            | (Type::Pointer(left), Type::Pointer(right)) => {
                Self::type_pattern_matches_after_subst(left, right)
            }
            (Type::Array(left, left_len), Type::Array(right, right_len)) => {
                left_len == right_len && Self::type_pattern_matches_after_subst(left, right)
            }
            _ => pattern == actual,
        }
    }

    pub(crate) fn apply_trait_deref(&mut self, expr: HirExpr) -> Option<HirExpr> {
        let resolved_ty = self.resolve_projection_type(&self.engine.resolve(&expr.ty));
        let can_autoref_mut = self.expr_can_autoref_mut_receiver(&expr);
        let receiver = ReceiverCandidate {
            expr: HirExpr {
                ty: resolved_ty,
                ..expr.clone()
            },
            adjustment: ReceiverAdjustment::None,
            can_autoref_mut,
        };
        let mut candidates = self.selection_service().select_concrete_method_candidates(
            std::slice::from_ref(&receiver),
            "*",
            |ty| self.resolve_projection_type(&self.engine.resolve(ty)),
        );
        if can_autoref_mut {
            let has_mutable = candidates.iter().any(|candidate| {
                candidate.function.as_ref().is_some_and(|function| {
                    function.self_receiver == Some(crate::types::ReceiverMode::Mut)
                })
            });
            if has_mutable {
                candidates.retain(|candidate| {
                    candidate.function.as_ref().is_some_and(|function| {
                        function.self_receiver == Some(crate::types::ReceiverMode::Mut)
                    })
                });
            }
        }
        let selected = match candidates.len() {
            0 => return None,
            1 => candidates.pop().unwrap(),
            _ => {
                let receiver_type = self.display_type(&expr.ty);
                self.diagnostics.push_selection_with_span(
                    format!("Ambiguous auto-deref for type {}", receiver_type),
                    expr.span.clone(),
                );
                return None;
            }
        };
        let method_func = selected.function.clone()?;
        let return_ty = self.resolve_projection_type(&selected.return_type);
        let Type::Reference { inner, .. } = &return_ty else {
            return None;
        };
        self.record_pending_impl_bounds(&selected, &HashMap::new(), "trait deref");
        let receiver =
            self.apply_receiver_adjustment(selected.receiver, selected.receiver_adjustment);

        Some(HirExpr {
            ty: *inner.clone(),
            kind: HirExprKind::Deref(Box::new(HirExpr {
                ty: return_ty,
                kind: HirExprKind::MethodCall(
                    Box::new(receiver),
                    method_func.name.clone(),
                    vec![],
                    method_func.self_receiver,
                    Some(selected.target),
                ),
                span: expr.span.clone(),
            })),
            span: expr.span.clone(),
        })
    }

    #[cfg(test)]
    pub(crate) fn seed_receiver_substitution_from_impl(
        imp: &crate::hir::HirImpl,
        recv_ty: &Type,
        subst: &mut HashMap<GenericParamId, Type>,
    ) {
        crate::selection::seed_receiver_substitution_from_impl(imp, recv_ty, subst);
    }

    fn apply_builtin_shared_ref_deref(&self, expr: HirExpr) -> Option<HirExpr> {
        let resolved_ty = self.resolve_projection_type(&self.engine.resolve(&expr.ty));
        let Type::Reference {
            mutable: false,
            inner,
        } = resolved_ty
        else {
            return None;
        };

        Some(HirExpr {
            ty: *inner,
            kind: HirExprKind::Deref(Box::new(expr.clone())),
            span: expr.span,
        })
    }

    fn coerce_mut_ref_to_shared_ref(&self, expr: HirExpr) -> Option<HirExpr> {
        let resolved_ty = self.resolve_projection_type(&self.engine.resolve(&expr.ty));
        let Type::Reference {
            mutable: true,
            inner,
        } = resolved_ty
        else {
            return None;
        };

        if matches!(inner.as_ref(), Type::Object(_)) {
            let span = expr.span.clone();
            let place = HirExpr {
                ty: inner.as_ref().clone(),
                kind: HirExprKind::Deref(Box::new(expr)),
                span: span.clone(),
            };
            return Some(HirExpr {
                ty: Type::Reference {
                    mutable: false,
                    inner,
                },
                kind: HirExprKind::Ref(false, Box::new(place)),
                span,
            });
        }

        Some(HirExpr {
            ty: Type::Reference {
                mutable: false,
                inner,
            },
            kind: expr.kind,
            span: expr.span,
        })
    }

    fn apply_builtin_mut_ref_deref(&self, expr: HirExpr) -> Option<HirExpr> {
        let resolved_ty = self.resolve_projection_type(&self.engine.resolve(&expr.ty));
        let Type::Reference {
            mutable: true,
            inner,
        } = resolved_ty
        else {
            return None;
        };

        Some(HirExpr {
            ty: *inner,
            kind: HirExprKind::Deref(Box::new(expr.clone())),
            span: expr.span,
        })
    }

    pub(crate) fn coerce_array_ref_to_slice_ref(&mut self, expr: HirExpr) -> Option<HirExpr> {
        let resolved_ty = self.resolve_projection_type(&self.engine.resolve(&expr.ty));
        let Type::Reference { mutable, .. } = resolved_ty else {
            return None;
        };

        self.coerce_array_ref_to_slice_ref_with_mutability(expr, mutable)
    }

    pub(crate) fn coerce_array_ref_to_slice_ref_with_mutability(
        &mut self,
        expr: HirExpr,
        mutable: bool,
    ) -> Option<HirExpr> {
        let span = expr.span.clone();
        let resolved_ty = self.resolve_projection_type(&self.engine.resolve(&expr.ty));
        let Type::Reference { inner, .. } = resolved_ty else {
            return None;
        };

        let Type::Array(elem, _) = inner.as_ref() else {
            return None;
        };

        let slice_ref_ty = Type::Reference {
            mutable,
            inner: Box::new(Type::Slice(elem.clone())),
        };

        Some(HirExpr {
            ty: slice_ref_ty,
            kind: HirExprKind::Intrinsic {
                name: "ArrayRefToSlice".to_string(),
                args: vec![expr],
            },
            span,
        })
    }

    pub(crate) fn coerce_array_value_to_slice_value(&mut self, expr: HirExpr) -> Option<HirExpr> {
        let span = expr.span.clone();
        let resolved_ty = self.resolve_projection_type(&self.engine.resolve(&expr.ty));
        let Type::Array(elem, _) = resolved_ty else {
            return None;
        };

        if !Self::is_borrowable_local_expr(&expr.kind) {
            return None;
        }

        let borrowed_array = HirExpr {
            ty: Type::Reference {
                mutable: false,
                inner: Box::new(expr.ty.clone()),
            },
            kind: HirExprKind::Ref(false, Box::new(expr)),
            span: span.clone(),
        };
        let borrowed_slice = self.coerce_array_ref_to_slice_ref(borrowed_array)?;

        Some(HirExpr {
            ty: Type::Slice(elem),
            kind: HirExprKind::Deref(Box::new(borrowed_slice)),
            span,
        })
    }

    fn coerce_array_value_to_slice_ref(&mut self, expr: HirExpr) -> Option<HirExpr> {
        let span = expr.span.clone();
        let resolved_ty = self.resolve_projection_type(&self.engine.resolve(&expr.ty));
        if !matches!(resolved_ty, Type::Array(_, _)) {
            return None;
        }

        if !Self::is_borrowable_local_expr(&expr.kind) {
            return None;
        }

        let borrowed_array = HirExpr {
            ty: Type::Reference {
                mutable: false,
                inner: Box::new(expr.ty.clone()),
            },
            kind: HirExprKind::Ref(false, Box::new(expr)),
            span,
        };

        self.coerce_array_ref_to_slice_ref(borrowed_array)
    }

    fn coerce_array_value_to_mut_slice_ref(&mut self, expr: HirExpr) -> Option<HirExpr> {
        let span = expr.span.clone();
        let resolved_ty = self.resolve_projection_type(&self.engine.resolve(&expr.ty));
        if !matches!(resolved_ty, Type::Array(_, _)) {
            return None;
        }

        if !self.expr_is_mutable_lvalue(&expr) {
            return None;
        }

        let borrowed_array = HirExpr {
            ty: Type::Reference {
                mutable: true,
                inner: Box::new(expr.ty.clone()),
            },
            kind: HirExprKind::Ref(true, Box::new(expr)),
            span,
        };

        self.coerce_array_ref_to_slice_ref_with_mutability(borrowed_array, true)
    }

    pub(crate) fn receiver_adjustment_candidates(
        &mut self,
        expr: HirExpr,
    ) -> Vec<ReceiverCandidate> {
        let mut candidates = Vec::new();
        let mut seen = HashSet::new();

        let push_candidate = |this: &mut Self,
                              candidates: &mut Vec<ReceiverCandidate>,
                              seen: &mut HashSet<Type>,
                              expr: HirExpr,
                              adjustment: ReceiverAdjustment,
                              can_autoref_mut: bool| {
            let candidate = ReceiverCandidate {
                expr,
                adjustment,
                can_autoref_mut,
            };
            let resolved = this.resolve_projection_type(&this.engine.resolve(&candidate.expr.ty));
            if seen.insert(resolved) {
                candidates.push(candidate);
            }
        };

        let can_autoref_mut = self.expr_can_autoref_mut_receiver(&expr);
        push_candidate(
            self,
            &mut candidates,
            &mut seen,
            expr.clone(),
            ReceiverAdjustment::None,
            can_autoref_mut,
        );

        if let Some(mut_slice_ref) = self.coerce_array_value_to_mut_slice_ref(expr.clone()) {
            push_candidate(
                self,
                &mut candidates,
                &mut seen,
                mut_slice_ref,
                ReceiverAdjustment::ArrayValueToMutSliceRef,
                false,
            );
        }

        if let Some(shared_ref_peeled) = self.apply_builtin_shared_ref_deref(expr.clone()) {
            push_candidate(
                self,
                &mut candidates,
                &mut seen,
                shared_ref_peeled,
                ReceiverAdjustment::BuiltinDeref,
                false,
            );
        }

        if let Some(mut_ref_peeled) = self.apply_builtin_mut_ref_deref(expr.clone()) {
            push_candidate(
                self,
                &mut candidates,
                &mut seen,
                mut_ref_peeled,
                ReceiverAdjustment::BuiltinDeref,
                true,
            );
        }

        if let Some(shared_ref) = self.coerce_mut_ref_to_shared_ref(expr.clone()) {
            push_candidate(
                self,
                &mut candidates,
                &mut seen,
                shared_ref.clone(),
                ReceiverAdjustment::MutToSharedRef,
                false,
            );

            if let Some(shared_ref_peeled) = self.apply_builtin_shared_ref_deref(shared_ref.clone())
            {
                push_candidate(
                    self,
                    &mut candidates,
                    &mut seen,
                    shared_ref_peeled,
                    ReceiverAdjustment::BuiltinDeref,
                    false,
                );
            }

            if let Some(shared_slice_ref) = self.coerce_array_ref_to_slice_ref(shared_ref) {
                push_candidate(
                    self,
                    &mut candidates,
                    &mut seen,
                    shared_slice_ref.clone(),
                    ReceiverAdjustment::ArrayRefToSliceRef,
                    false,
                );

                if let Some(shared_slice_value) =
                    self.apply_builtin_shared_ref_deref(shared_slice_ref)
                {
                    push_candidate(
                        self,
                        &mut candidates,
                        &mut seen,
                        shared_slice_value,
                        ReceiverAdjustment::BuiltinDeref,
                        false,
                    );
                }
            }
        }

        if let Some(deref_target) = self.apply_trait_deref(expr.clone()) {
            push_candidate(
                self,
                &mut candidates,
                &mut seen,
                deref_target,
                ReceiverAdjustment::TraitDeref,
                can_autoref_mut,
            );
        }

        if let Some(slice_ref) = self.coerce_array_ref_to_slice_ref(expr.clone()) {
            push_candidate(
                self,
                &mut candidates,
                &mut seen,
                slice_ref,
                ReceiverAdjustment::ArrayRefToSliceRef,
                false,
            );
        }

        if let Some(slice_ref) = self.coerce_array_value_to_slice_ref(expr.clone()) {
            push_candidate(
                self,
                &mut candidates,
                &mut seen,
                slice_ref,
                ReceiverAdjustment::ArrayValueToSliceRef,
                false,
            );
        }

        if let Some(slice_value) = self.coerce_array_value_to_slice_value(expr) {
            push_candidate(
                self,
                &mut candidates,
                &mut seen,
                slice_value,
                ReceiverAdjustment::ArrayValueToSliceValue,
                false,
            );
        }

        candidates
    }

    pub(crate) fn apply_receiver_adjustment(
        &mut self,
        expr: HirExpr,
        adjustment: ReceiverAdjustment,
    ) -> HirExpr {
        match adjustment {
            ReceiverAdjustment::AutorefShared | ReceiverAdjustment::AutorefMut => {
                let span = expr.span.clone();
                let resolved_ty = self.resolve_projection_type(&self.engine.resolve(&expr.ty));
                if !matches!(resolved_ty, Type::Str | Type::Slice(_) | Type::Object(_)) {
                    return expr;
                }

                let mutable = matches!(adjustment, ReceiverAdjustment::AutorefMut);
                HirExpr {
                    ty: Type::Reference {
                        mutable,
                        inner: Box::new(expr.ty.clone()),
                    },
                    kind: HirExprKind::Ref(mutable, Box::new(expr)),
                    span,
                }
            }
            ReceiverAdjustment::MutToSharedRef => self
                .coerce_mut_ref_to_shared_ref(expr.clone())
                .unwrap_or(expr),
            ReceiverAdjustment::None
            | ReceiverAdjustment::BuiltinDeref
            | ReceiverAdjustment::TraitDeref
            | ReceiverAdjustment::ArrayRefToSliceRef
            | ReceiverAdjustment::ArrayValueToMutSliceRef
            | ReceiverAdjustment::ArrayValueToSliceRef
            | ReceiverAdjustment::ArrayValueToSliceValue => expr,
        }
    }

    pub(crate) fn autoderef_candidates(&mut self, expr: HirExpr) -> Vec<HirExpr> {
        let mut candidates = vec![expr.clone()];
        let mut seen = HashSet::new();
        let mut current = expr;

        seen.insert(self.resolve_projection_type(&self.engine.resolve(&current.ty)));

        for _ in 0..8 {
            let next = self
                .apply_builtin_shared_ref_deref(current.clone())
                .or_else(|| self.apply_trait_deref(current.clone()));
            let Some(next) = next else {
                break;
            };

            let resolved_next = self.resolve_projection_type(&self.engine.resolve(&next.ty));
            if !seen.insert(resolved_next) {
                break;
            }

            candidates.push(next.clone());
            current = next;
        }

        candidates
    }

    pub(crate) fn resolve_projection_type(&self, ty: &Type) -> Type {
        ProjectionNormalizer::normalize(self, ty)
    }

    /// Get the type name for method lookup, supporting both struct/enum and primitive types
    #[cfg(test)]
    pub(crate) fn get_type_name_for_method_lookup(ty: &Type) -> Option<String> {
        match ty {
            Type::Struct { .. } | Type::Enum { .. } => Some(ty.to_string()),
            // Support primitive types for trait implementations
            Type::I64 => Some("I64".to_string()),
            Type::I32 => Some("I32".to_string()),
            Type::I16 => Some("I16".to_string()),
            Type::I8 => Some("I8".to_string()),
            Type::U64 => Some("U64".to_string()),
            Type::U32 => Some("U32".to_string()),
            Type::U16 => Some("U16".to_string()),
            Type::U8 => Some("U8".to_string()),
            Type::F64 => Some("F64".to_string()),
            Type::F32 => Some("F32".to_string()),
            Type::Bool => Some("Bool".to_string()),
            Type::Str => Some("Str".to_string()),
            Type::Slice(_) => Some("Array".to_string()),
            Type::Array(_, _) => Some(ty.to_string()),
            Type::Pointer(_) => Some(ty.to_string()),
            Type::Reference { inner, .. }
                if matches!(inner.as_ref(), Type::Slice(_) | Type::Str) =>
            {
                Some(ty.to_string())
            }
            Type::Reference { inner, .. } => Self::get_type_name_for_method_lookup(inner),
            _ => None,
        }
    }

    pub(crate) fn infer_generic_subst_from_types(
        expected: &Type,
        actual: &Type,
        subst: &mut HashMap<GenericParamId, Type>,
    ) {
        crate::selection::infer_generic_subst_from_types(expected, actual, subst);
    }

    pub(crate) fn infer_method_substitution(
        &mut self,
        recv_ty: &Type,
        method_func: &HirFunction,
        args: &[HirExpr],
        operation_span: crate::lexer::Span,
    ) -> HashMap<GenericParamId, Type> {
        let inferable_generics: HashSet<_> = method_func
            .generic_params
            .iter()
            .map(|param| param.id)
            .collect();
        let mut subst: HashMap<_, _> = method_func
            .generic_params
            .iter()
            .map(|param| {
                (
                    param.id,
                    self.engine
                        .fresh_type_var_at_kind(operation_span.clone(), param.kind.clone()),
                )
            })
            .collect();

        if method_func.is_method {
            if let Some(self_param) = method_func.params.first() {
                let expected = self_param.ty.substitute_generics(&subst);
                let _ = self.engine.unify(&expected, recv_ty);
                Self::infer_generic_subst_from_types(&self_param.ty, recv_ty, &mut subst);
            }
        }

        let param_start = if method_func.is_method { 1 } else { 0 };
        for (param, arg) in method_func.params[param_start..].iter().zip(args.iter()) {
            let actual = self.engine.resolve(&arg.ty);
            Self::infer_generic_subst_from_types(&param.ty, &actual, &mut subst);
            let expected = param.ty.substitute_generics(&subst);
            let _ = self.engine.unify(&expected, &arg.ty);

            let expected = param.ty.substitute_generics(&subst);
            let actual = self.engine.resolve(&arg.ty);
            Self::infer_generic_subst_from_types(&expected, &actual, &mut subst);
        }

        subst.retain(|param, _| inferable_generics.contains(param));
        for ty in subst.values_mut() {
            *ty = self.engine.resolve(ty);
        }
        subst
    }
}

impl ProjectionProvider for Lowerer {
    fn find_projection_impl(
        &self,
        base_ty: &Type,
        trait_id: crate::ids::DefId,
        trait_args: &[Type],
    ) -> Option<ProjectionImpl> {
        self.selection_service()
            .select_trait_impl_strict(base_ty, trait_id, trait_args)
            .ok()
            .map(|imp| ProjectionImpl {
                impl_id: imp.id,
                receiver_pattern: imp.receiver_pattern.clone(),
                trait_arg_types: imp.trait_arg_types.clone(),
                associated_types: imp
                    .associated_types
                    .iter()
                    .map(|assoc| ProjectionAssociatedType {
                        id: assoc.id,
                        name: assoc.name.clone(),
                        ty: assoc.ty.clone(),
                    })
                    .collect(),
            })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::hir::{
        HirAssociatedTypeDecl, HirAssociatedTypeDef, HirBlock, HirExpr, HirExprKind, HirFunction,
        HirFunctionSig, HirImpl, HirImplOwner, HirParam, HirTrait, HirVarRef, HirVarTarget,
    };
    use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId};
    use crate::lower::Lowerer;
    use crate::selection::ReceiverAdjustment;
    use crate::types::{AssociatedTypeKey, GenericParamDecl, GenericParamId, Type};

    fn def_id(index: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(index))
    }

    #[test]
    fn lowerer_matching_wrappers_match_selection_helpers() {
        let impl_id = DefId::new(CrateId(0), LocalDefId(80));
        let generic = GenericParamId {
            owner: impl_id,
            index: 0,
        };
        let imp = HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Vec".to_string()),
            type_name: "Vec".to_string(),
            type_generics: vec![GenericParamDecl::type_param(generic, "T")],
            receiver_pattern: vec![Type::Generic(generic)].into(),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        };
        let recv_ty = Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(81)),
            args: vec![Type::I64],
        };
        let mut lowerer_subst = HashMap::new();
        let mut selection_subst = HashMap::new();

        Lowerer::seed_receiver_substitution_from_impl(&imp, &recv_ty, &mut lowerer_subst);
        crate::selection::seed_receiver_substitution_from_impl(
            &imp,
            &recv_ty,
            &mut selection_subst,
        );

        assert_eq!(lowerer_subst, selection_subst);
        assert!(crate::selection::type_pattern_matches(
            &Type::Generic(generic),
            &Type::I64,
            &mut HashMap::new()
        ));
    }

    #[test]
    fn generated_operator_deref_persists_exact_selected_authority() {
        let owner_id = def_id(90);
        let trait_id = def_id(91);
        let member_id = def_id(92);
        let impl_id = def_id(93);
        let method_id = def_id(94);
        let receiver_ty = Type::Struct {
            id: owner_id,
            args: Vec::new(),
        };
        let return_ty = Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        };
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .resolver
            .item_names_by_id
            .insert(owner_id, "Box".to_string());
        lowerer
            .resolver
            .item_names_by_id
            .insert(trait_id, "PointerLike".to_string());
        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "PointerLike".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "*".to_string(),
                HirFunctionSig {
                    id: member_id,
                    name: "*".to_string(),
                    generic_params: Vec::new(),
                    params: vec![receiver_ty.clone()],
                    ret: return_ty.clone(),
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(crate::types::ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
        });
        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(receiver_ty.clone()),
                trait_name: Some("PointerLike".to_string()),
                trait_id: Some(trait_id),
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::from([(
                    "*".to_string(),
                    HirFunction {
                        id: method_id,
                        name: "*".to_string(),
                        generic_params: Vec::new(),
                        generic_bounds: HashMap::new().into(),
                        params: vec![HirParam {
                            name: "self".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: receiver_ty.clone(),
                            mutable: false,
                            is_ref: false,
                        }],
                        ret_type: return_ty.clone(),
                        body: HirBlock {
                            stmts: Vec::new(),
                            ty: return_ty,
                        },
                        is_curried: false,
                        is_method: true,
                        self_receiver: Some(crate::types::ReceiverMode::Move),
                        is_unsafe: false,
                    },
                )]),
            })
            .unwrap();
        lowerer
            .imported_effective_trait_methods
            .insert((impl_id, member_id), method_id);
        let expr = HirExpr {
            ty: receiver_ty,
            kind: HirExprKind::Var("box".to_string()),
            span: crate::lexer::Span::test(),
        };
        let candidates = lowerer
            .selection_service()
            .select_concrete_method_candidates(
                &[crate::selection::ReceiverCandidate {
                    expr: expr.clone(),
                    adjustment: crate::selection::ReceiverAdjustment::None,
                    can_autoref_mut: false,
                }],
                "*",
                Type::clone,
            );
        assert_eq!(candidates.len(), 1, "expected one operator candidate");

        let deref = lowerer
            .apply_trait_deref(expr)
            .expect("operator protocol should select the exact impl");
        let HirExprKind::Deref(method_call) = deref.kind else {
            panic!("expected generated dereference");
        };
        let HirExprKind::MethodCall(_, method_name, _, _, Some(target)) = method_call.kind else {
            panic!("expected authority-bearing protocol method call");
        };
        assert_eq!(method_name, "*");
        assert_eq!(target.impl_id(), Some(impl_id));
        assert_eq!(target.trait_id(), Some(trait_id));
        assert_eq!(target.method_id(), Some(method_id));
    }

    #[test]
    fn generic_nominal_pattern_matching_requires_same_def_id() {
        let pattern = Type::Struct {
            id: def_id(1),
            args: vec![Type::I64],
        };
        let actual = Type::Struct {
            id: def_id(2),
            args: vec![Type::I64],
        };

        assert!(!Lowerer::type_pattern_matches_after_subst(
            &pattern, &actual
        ));
    }

    #[test]
    fn generic_substitution_does_not_cross_nominal_def_ids() {
        let expected = Type::Struct {
            id: def_id(1),
            args: vec![Type::Generic(GenericParamId {
                owner: def_id(0),
                index: 0,
            })],
        };
        let actual = Type::Struct {
            id: def_id(2),
            args: vec![Type::I64],
        };
        let mut subst = HashMap::new();

        Lowerer::infer_generic_subst_from_types(&expected, &actual, &mut subst);

        assert!(subst.is_empty());
    }

    #[test]
    fn receiver_adjustment_coerces_resolved_local_array_to_slice_ref() {
        let mut lowerer = Lowerer::new_for_test();
        let array_ty = Type::Array(Box::new(Type::I64), 3);
        let expr = HirExpr {
            ty: array_ty,
            kind: HirExprKind::ResolvedVar(HirVarRef {
                name: "arr".to_string(),
                target: HirVarTarget::Local(crate::ids::HirLocalId(0)),
            }),
            span: crate::lexer::Span::test(),
        };

        let candidates = lowerer.receiver_adjustment_candidates(expr);

        assert!(candidates.iter().any(|candidate| matches!(
            &candidate.expr.ty,
            Type::Reference { mutable: false, inner }
                if matches!(inner.as_ref(), Type::Slice(elem) if elem.as_ref() == &Type::I64)
        )));
    }

    #[test]
    fn receiver_adjustment_candidates_include_mutable_array_slice() {
        let owner_id = def_id(90);
        let trait_id = def_id(91);
        let member_id = def_id(92);
        let impl_id = def_id(93);
        let method_id = def_id(94);
        let mut lowerer = Lowerer::new_for_test();
        let array_ty = Type::Array(Box::new(Type::I64), 3);
        let return_ty = Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        };
        lowerer
            .resolver
            .item_names_by_id
            .insert(owner_id, "[I64; 3]".to_string());
        lowerer
            .resolver
            .item_names_by_id
            .insert(trait_id, "PointerLike".to_string());
        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "PointerLike".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "*".to_string(),
                HirFunctionSig {
                    id: member_id,
                    name: "*".to_string(),
                    generic_params: Vec::new(),
                    params: vec![array_ty.clone()],
                    ret: return_ty.clone(),
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(crate::types::ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
        });
        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("[I64; 3]".to_string()),
                type_name: "[I64; 3]".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(array_ty.clone()),
                trait_name: Some("PointerLike".to_string()),
                trait_id: Some(trait_id),
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::from([(
                    "*".to_string(),
                    HirFunction {
                        id: method_id,
                        name: "*".to_string(),
                        generic_params: Vec::new(),
                        generic_bounds: HashMap::new().into(),
                        params: vec![HirParam {
                            name: "self".to_string(),
                            local_id: crate::ids::HirLocalId(0),
                            ty: array_ty.clone(),
                            mutable: false,
                            is_ref: false,
                        }],
                        ret_type: return_ty.clone(),
                        body: HirBlock {
                            stmts: Vec::new(),
                            ty: return_ty,
                        },
                        is_curried: false,
                        is_method: true,
                        self_receiver: Some(crate::types::ReceiverMode::Move),
                        is_unsafe: false,
                    },
                )]),
            })
            .unwrap();
        lowerer
            .imported_effective_trait_methods
            .insert((impl_id, member_id), method_id);
        lowerer.scope.define_local(
            "arr".to_string(),
            array_ty.clone(),
            true,
            crate::ids::HirLocalId(0),
        );
        let expr = HirExpr {
            ty: array_ty,
            kind: HirExprKind::ResolvedVar(HirVarRef {
                name: "arr".to_string(),
                target: HirVarTarget::Local(crate::ids::HirLocalId(0)),
            }),
            span: crate::lexer::Span::test(),
        };

        let candidates = lowerer.receiver_adjustment_candidates(expr);

        assert_eq!(candidates[0].adjustment, ReceiverAdjustment::None);
        assert!(matches!(
            (&candidates[0].expr.ty, &candidates[1].expr.ty, candidates[1].adjustment),
            (
                Type::Array(elem, 3),
                Type::Reference { mutable: true, inner },
                ReceiverAdjustment::ArrayValueToMutSliceRef
            ) if elem.as_ref() == &Type::I64
                && matches!(inner.as_ref(), Type::Slice(slice_elem) if slice_elem.as_ref() == &Type::I64)
        ));
    }

    #[test]
    fn projection_resolution_rejects_assoc_type_owned_by_different_trait() {
        let show_id = def_id(30);
        let other_id = def_id(31);
        let impl_id = def_id(32);
        let box_id = def_id(33);
        let assoc_type_id = AssocTypeId(0);
        let base_ty = Type::Struct {
            id: box_id,
            args: Vec::new(),
        };

        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .resolver
            .item_names_by_id
            .insert(box_id, "Box".to_string());
        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: show_id,
            name: "Show".to_string(),
            generic_params: Vec::new(),
            associated_types: vec![HirAssociatedTypeDecl {
                id: assoc_type_id,
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
            }],
            methods: HashMap::new(),
            signatures: HashMap::new(),
        });
        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: other_id,
            name: "Other".to_string(),
            generic_params: Vec::new(),
            associated_types: vec![HirAssociatedTypeDecl {
                id: assoc_type_id,
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
            }],
            methods: HashMap::new(),
            signatures: HashMap::new(),
        });
        lowerer
            .resolver
            .item_paths
            .insert("Show".to_string(), show_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(show_id, "Show".to_string());
        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(base_ty.clone()),
                trait_name: Some("Show".to_string()),
                trait_id: Some(show_id),
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: vec![HirAssociatedTypeDef {
                    id: assoc_type_id,
                    name: "Output".to_string(),
                    kind: crate::type_services::kind::Kind::Type,
                    ty: Type::I64,
                }],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            })
            .unwrap();

        let projection = Type::Projection {
            ty: Box::new(base_ty),
            trait_id: show_id,
            assoc_type: AssociatedTypeKey {
                owner: other_id,
                assoc_type_id,
            },
            trait_args: Vec::new(),
        };

        assert!(matches!(
            lowerer.resolve_projection_type(&projection),
            Type::Projection { .. }
        ));
    }

    #[test]
    fn nominal_method_lookup_in_context_preserves_canonical_owner_name() {
        let id = def_id(42);
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .resolver
            .item_names_by_id
            .insert(id, "dep::module::Foo".to_string());

        assert_eq!(
            lowerer.get_type_name_for_method_lookup_in_context(&Type::Struct {
                id,
                args: Vec::new(),
            }),
            Some("dep::module::Foo".to_string())
        );
    }

    #[test]
    fn trait_impl_selection_uses_typed_receiver_pattern() {
        let id = def_id(42);
        let trait_id = def_id(44);
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .resolver
            .item_names_by_id
            .insert(id, "dep::module::Foo".to_string());
        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Show".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        });
        lowerer
            .resolver
            .item_paths
            .insert("Show".to_string(), trait_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(trait_id, "Show".to_string());
        let impl_id = def_id(43);
        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("dep::module::Foo".to_string()),
                type_name: "Foo".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                    id,
                    args: Vec::new(),
                }),
                trait_name: Some("Show".to_string()),
                trait_id: Some(trait_id),
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::new(),
            })
            .unwrap();
        assert!(lowerer
            .selection_service()
            .select_trait_impl(
                &Type::Struct {
                    id,
                    args: Vec::new(),
                },
                trait_id,
                &[],
            )
            .is_ok());
    }
}

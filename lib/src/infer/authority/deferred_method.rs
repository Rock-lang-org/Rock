use std::collections::{HashMap, HashSet};

use crate::hir::{HirExpr, HirExprKind};
use crate::lower::ResolveError;
use crate::selection::{type_pattern_matches, ReceiverAdjustment, SelectedMethod, SelectedOrigin};
use crate::types::{GenericParamId, TraitBound, Type};

use super::type_predicates::contains_recovery_type;
use super::MethodAuthorityContext;

impl MethodAuthorityContext<'_> {
    pub(super) fn record_selected_bounds(
        &mut self,
        selected: &SelectedMethod,
        substitution: &HashMap<GenericParamId, Type>,
        context: &str,
    ) {
        for (ty, bound) in &selected.pending_impl_bounds {
            self.record_trait_bound(
                ty.substitute_generics(substitution),
                TraitBound {
                    trait_id: bound.trait_id,
                    type_args: bound
                        .type_args
                        .iter()
                        .map(|arg| arg.substitute_generics(substitution))
                        .collect(),
                },
                selected.receiver.span.clone(),
                context,
            );
        }
        if let Some(function) = &selected.function {
            for (generic_param, bounds) in &function.generic_bounds {
                let subject = substitution
                    .get(generic_param)
                    .cloned()
                    .unwrap_or(Type::Generic(*generic_param));
                for bound in bounds {
                    self.record_trait_bound(
                        subject.clone(),
                        TraitBound {
                            trait_id: bound.trait_id,
                            type_args: bound
                                .type_args
                                .iter()
                                .map(|arg| arg.substitute_generics(substitution))
                                .collect(),
                        },
                        selected.receiver.span.clone(),
                        context,
                    );
                }
            }
        }
    }

    fn record_trait_bound(
        &mut self,
        ty: Type,
        bound: TraitBound,
        span: crate::lexer::Span,
        context: &str,
    ) {
        let ty = self.resolved_type(&ty);
        if let Type::TypeVar(id) = ty {
            self.engine.borrow_mut().add_bound(id, bound.clone());
            self.constraint_store
                .add_trait(Type::TypeVar(id), bound, span, context);
        } else {
            self.constraint_store.add_trait(ty, bound, span, context);
        }
    }

    /// A free receiver whose candidates all implement one trait member (the
    /// trait default and its overrides) is constrained by that trait instead of
    /// being ambiguous: `x.println!` makes `x: Show`, which generalizes.
    fn select_shared_trait_member(
        &mut self,
        receiver: &HirExpr,
        receiver_candidates: &[crate::selection::ReceiverCandidate],
        inferred: &[SelectedMethod],
        method_name: &str,
        args: &[HirExpr],
    ) -> Option<SelectedMethod> {
        let Type::TypeVar(id) = self.resolved_type(&receiver.ty) else {
            return None;
        };
        let mut members = inferred.iter().map(|candidate| match &candidate.target.target {
            crate::hir::HirSelectedMethodTarget::ImplMethod {
                selected_trait: Some(selected),
                ..
            } => Some((selected.trait_id, selected.member_id, selected.trait_args.clone())),
            _ => None,
        });
        let (trait_id, member_id, sample_args) = members.next()??;
        if !members.all(|member| {
            member.is_some_and(|(other_trait, other_member, other_args)| {
                other_trait == trait_id
                    && other_member == member_id
                    && other_args.len() == sample_args.len()
            })
        }) {
            return None;
        }
        // Trait arguments (`Rhs` in `Add Rhs`) are left to the call's arguments
        // and later generalization, never to one arbitrary impl.
        let type_args = sample_args
            .iter()
            .map(|arg| {
                let mut engine = self.engine.borrow_mut();
                let kind = engine
                    .kind_of(arg)
                    .unwrap_or(crate::type_services::kind::Kind::Type);
                engine.fresh_type_var_of_kind(kind)
            })
            .collect();
        let bound = TraitBound {
            trait_id,
            type_args,
        };
        self.record_trait_bound(
            Type::TypeVar(id),
            bound.clone(),
            receiver.span.clone(),
            "method call",
        );
        self.service()
            .select_bound_method_matching(
                receiver_candidates,
                &[bound],
                method_name,
                Type::TypeVar(id),
                matches!(receiver.kind, HirExprKind::Call(_, _, _)),
                |candidate| self.deferred_method_accepts_args(candidate, args),
            )
            .ok()
    }

    fn deferred_method_accepts_args(&self, candidate: &SelectedMethod, args: &[HirExpr]) -> bool {
        if candidate.substituted_params.len() != args.len() {
            return false;
        }
        let mut substitution = candidate.owner_substitution.clone();
        if matches!(candidate.origin, SelectedOrigin::TraitBound { .. }) {
            // Caller-owned generics in a bound are rigid, not impl parameters
            // that can be rebound to make a different RHS fit.
            let mut generics = HashSet::new();
            for param in &candidate.substituted_params {
                param.ty.collect_generic_params(&mut generics);
            }
            for generic in generics {
                substitution
                    .entry(generic)
                    .or_insert(Type::Generic(generic));
            }
        }
        candidate
            .substituted_params
            .iter()
            .zip(args)
            .all(|(param, arg)| {
                let expected = self.resolved_type(&param.ty);
                let actual = self.resolved_type(&arg.ty);
                if contains_recovery_type(&actual)
                    || contains_recovery_type(&expected)
                    || type_pattern_matches(&expected, &actual, &mut substitution)
                {
                    return true;
                }
                if let Type::Reference {
                    mutable: false,
                    inner,
                } = &expected
                {
                    return type_pattern_matches(inner, &actual, &mut substitution);
                }
                false
            })
    }

    pub(super) fn select_deferred_method(
        &mut self,
        receiver: &HirExpr,
        method_name: &str,
        args: &[HirExpr],
        _expected_ty: Option<&Type>,
        errors: &mut Vec<ResolveError>,
    ) -> Option<SelectedMethod> {
        let receiver_candidates = self.receiver_adjustment_candidates(receiver.clone());
        let unresolved_inputs = contains_recovery_type(&self.resolved_type(&receiver.ty))
            || args
                .iter()
                .any(|arg| contains_recovery_type(&self.resolved_type(&arg.ty)));
        if let Type::TypeVar(id) = self.resolved_type(&receiver.ty) {
            let bounds = self.engine.borrow().get_bounds(id);
            if !bounds.is_empty() {
                if let Ok(selected) = self.service().select_bound_method_matching(
                    &receiver_candidates,
                    &bounds,
                    method_name,
                    Type::TypeVar(id),
                    matches!(receiver.kind, HirExprKind::Call(_, _, _)),
                    |candidate| self.deferred_method_accepts_args(candidate, args),
                ) {
                    return Some(selected);
                }
            }
        }
        if let Type::Generic(param) = self.resolved_type(&receiver.ty) {
            let bounds = self.bounds.get(&param).cloned().unwrap_or_default();
            if !bounds.is_empty() {
                if let Ok(selected) = self.service().select_bound_method_matching(
                    &receiver_candidates,
                    &bounds,
                    method_name,
                    Type::Generic(param),
                    matches!(receiver.kind, HirExprKind::Call(_, _, _)),
                    |candidate| self.deferred_method_accepts_args(candidate, args),
                ) {
                    return Some(selected);
                }
            }
        }
        let mut proven = Vec::new();
        let mut deferred = Vec::new();
        for candidate in &receiver_candidates {
            let mut selected = self.service().select_concrete_method_candidates(
                std::slice::from_ref(candidate),
                method_name,
                |ty| self.resolved_type(ty),
            );
            if selected.len() > 1 {
                selected.retain(|candidate| self.deferred_method_accepts_args(candidate, args));
            }
            let has_proven = selected
                .iter()
                .any(|candidate| candidate.pending_impl_bounds.is_empty());
            if has_proven {
                selected.retain(|candidate| candidate.pending_impl_bounds.is_empty());
            }
            match selected.len() {
                0 => {}
                1 => {
                    if let Some(mut selected) = selected.pop() {
                        if candidate.adjustment != ReceiverAdjustment::None {
                            selected.receiver_adjustment = candidate.adjustment;
                        }
                        let destination = if selected.pending_impl_bounds.is_empty() {
                            &mut proven
                        } else {
                            &mut deferred
                        };
                        if destination.iter().all(|existing: &SelectedMethod| {
                            existing.target.target != selected.target.target
                        }) {
                            destination.push(selected);
                        }
                    }
                }
                _ => {
                    if !self.strict && unresolved_inputs {
                        self.ambiguous = true;
                        continue;
                    }
                    errors.push(ResolveError::with_span_code(
                        format!(
                            "Ambiguous selection for '{}' on type {}",
                            method_name,
                            self.display_type(&candidate.expr.ty)
                        ),
                        candidate.expr.span.clone(),
                        crate::diagnostic::DiagnosticCode::Selection,
                    ));
                    return None;
                }
            }
        }
        if proven.len() == 1 {
            return proven.pop();
        }
        if proven.len() > 1 {
            if !self.strict && unresolved_inputs {
                self.ambiguous = true;
                return None;
            }
            errors.push(ResolveError::with_span_code(
                format!(
                    "Ambiguous selection for '{}' on type {}",
                    method_name,
                    self.display_type(&receiver.ty)
                ),
                receiver.span.clone(),
                crate::diagnostic::DiagnosticCode::Selection,
            ));
            return None;
        }
        if deferred.is_empty()
            && (contains_recovery_type(&self.resolved_type(&receiver.ty))
                || matches!(self.resolved_type(&receiver.ty), Type::Generic(_)))
        {
            let mut inferred =
                self.service()
                    .select_inferred_method_candidates(receiver, method_name, |ty| {
                        self.resolved_type(ty)
                    });
            inferred.retain(|candidate| self.deferred_method_accepts_args(candidate, args));
            if let Some(expected) = _expected_ty.map(|ty| self.resolved_type(ty)) {
                if !contains_recovery_type(&expected) {
                    inferred.retain(|candidate| {
                        let mut substitution = candidate.owner_substitution.clone();
                        type_pattern_matches(&candidate.return_type, &expected, &mut substitution)
                    });
                }
            }
            inferred.sort_by_key(|candidate| {
                (candidate.target.impl_id(), candidate.target.method_id())
            });
            inferred.dedup_by_key(|candidate| {
                (candidate.target.impl_id(), candidate.target.method_id())
            });
            if inferred.len() == 1 {
                let mut selected = inferred.into_iter().next();
                if let Some(selected) = selected.as_mut() {
                    let target_key = (selected.target.impl_id(), selected.target.method_id());
                    for candidate in &receiver_candidates {
                        let concrete = self.service().select_concrete_method_candidates(
                            std::slice::from_ref(candidate),
                            method_name,
                            |ty| self.resolved_type(ty),
                        );
                        if concrete.iter().any(|candidate| {
                            (candidate.target.impl_id(), candidate.target.method_id()) == target_key
                        }) {
                            if candidate.adjustment != ReceiverAdjustment::None {
                                selected.receiver_adjustment = candidate.adjustment;
                            }
                            break;
                        }
                    }
                    selected.receiver.kind = receiver.kind.clone();
                    selected.receiver.span = receiver.span.clone();
                }
                return selected;
            }
            if inferred.len() > 1 {
                if !self.strict && unresolved_inputs {
                    self.ambiguous = true;
                    return None;
                }
                if let Some(selected) =
                    self.select_shared_trait_member(receiver, &receiver_candidates, &inferred, method_name, args)
                {
                    return Some(selected);
                }
                if let Some(expected) = _expected_ty.map(|ty| self.resolved_type(ty)) {
                    if !contains_recovery_type(&expected) {
                        let narrowed = inferred
                            .iter()
                            .filter(|candidate| {
                                let mut substitution = candidate.owner_substitution.clone();
                                type_pattern_matches(
                                    &candidate.return_type,
                                    &expected,
                                    &mut substitution,
                                )
                            })
                            .count();
                        if narrowed == 0 {
                            return None;
                        }
                    }
                }
                errors.push(ResolveError::with_span(
                    format!(
                        "Ambiguous selection for '{}' on type {}",
                        method_name,
                        self.display_type(&receiver.ty)
                    ),
                    receiver.span.clone(),
                ));
                return None;
            }
        }
        if deferred.len() == 1 {
            return deferred.pop();
        }
        if deferred.len() > 1 {
            if !self.strict {
                self.ambiguous = true;
                return None;
            }
            errors.push(ResolveError::with_span(
                format!(
                    "Ambiguous selection for '{}' on type {}",
                    method_name,
                    self.display_type(&receiver.ty)
                ),
                receiver.span.clone(),
            ));
            return None;
        }
        if self
            .service()
            .mut_receiver_method_requires_mutable_receiver(
                &receiver_candidates,
                method_name,
                |ty| self.resolved_type(ty),
            )
        {
            errors.push(ResolveError::with_span(
                format!(
                    "Cannot call mutable receiver method '{}' without a mutable receiver",
                    method_name
                ),
                receiver.span.clone(),
            ));
        }
        None
    }
}

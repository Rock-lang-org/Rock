use super::*;

impl<'a> SelectionService<'a> {
    pub fn select_concrete_method<F>(
        &self,
        receiver_candidates: &[ReceiverCandidate],
        method_name: &str,
        mut normalize: F,
    ) -> Result<SelectedMethod, SelectionDiagnostic>
    where
        F: FnMut(&Type) -> Type,
    {
        for receiver_candidate in receiver_candidates {
            let mut candidates = self.select_concrete_method_candidates(
                std::slice::from_ref(receiver_candidate),
                method_name,
                &mut normalize,
            );
            Self::prefer_exact_receiver(&mut candidates, receiver_candidate.adjustment);
            match candidates.len() {
                0 => continue,
                1 => return Ok(candidates.pop().expect("one candidate")),
                _ => {
                    return Err(SelectionDiagnostic::AmbiguousCandidates {
                        operation: method_name.to_string(),
                        receiver: normalize(&receiver_candidate.expr.ty),
                        candidates: candidates
                            .iter()
                            .filter_map(|candidate| {
                                candidate.target.impl_id().or(candidate.target.method_id())
                            })
                            .collect(),
                    })
                }
            }
        }
        Err(SelectionDiagnostic::NoImplementation {
            operation: method_name.to_string(),
            receiver: receiver_candidates
                .first()
                .map(|candidate| normalize(&candidate.expr.ty))
                .unwrap_or(Type::Error),
        })
    }

    pub fn select_concrete_method_matching<N, M>(
        &self,
        receiver_candidates: &[ReceiverCandidate],
        method_name: &str,
        mut normalize: N,
        mut matches: M,
    ) -> Result<SelectedMethod, SelectionDiagnostic>
    where
        N: FnMut(&Type) -> Type,
        M: FnMut(&SelectedMethod) -> bool,
    {
        for receiver_candidate in receiver_candidates {
            let mut candidates = self.select_concrete_method_candidates(
                std::slice::from_ref(receiver_candidate),
                method_name,
                &mut normalize,
            );
            candidates.retain(|candidate| matches(candidate));
            Self::prefer_exact_receiver(&mut candidates, receiver_candidate.adjustment);
            let mut seen_targets = Vec::new();
            candidates.retain(|candidate| {
                let target = (candidate.target.impl_id(), candidate.target.method_id());
                if seen_targets.contains(&target) {
                    false
                } else {
                    seen_targets.push(target);
                    true
                }
            });
            match candidates.len() {
                0 => continue,
                1 => return Ok(candidates.pop().expect("one candidate")),
                _ => {
                    let mut candidate_ids = candidates
                        .iter()
                        .filter_map(|candidate| {
                            candidate.target.impl_id().or(candidate.target.method_id())
                        })
                        .collect::<Vec<_>>();
                    candidate_ids.sort();
                    candidate_ids.dedup();
                    return Err(SelectionDiagnostic::AmbiguousCandidates {
                        operation: method_name.to_string(),
                        receiver: normalize(&receiver_candidate.expr.ty),
                        candidates: candidate_ids,
                    });
                }
            }
        }

        Err(SelectionDiagnostic::NoImplementation {
            operation: method_name.to_string(),
            receiver: receiver_candidates
                .first()
                .map(|candidate| normalize(&candidate.expr.ty))
                .unwrap_or(Type::Error),
        })
    }

    pub(crate) fn prefer_exact_receiver(
        candidates: &mut Vec<SelectedMethod>,
        adjustment: ReceiverAdjustment,
    ) {
        if candidates
            .iter()
            .any(|candidate| candidate.receiver_adjustment == adjustment)
        {
            candidates.retain(|candidate| candidate.receiver_adjustment == adjustment);
        }
    }

    pub fn select_concrete_method_candidates<F>(
        &self,
        receiver_candidates: &[ReceiverCandidate],
        method_name: &str,
        mut normalize: F,
    ) -> Vec<SelectedMethod>
    where
        F: FnMut(&Type) -> Type,
    {
        receiver_candidates
            .iter()
            .fold(Vec::new(), |mut found, candidate| {
                let candidate_ty = normalize(&candidate.expr.ty);
                let mut candidates = self
                    .impl_ids_in_order()
                    .into_iter()
                    .filter_map(|id| self.impls.get(&id))
                    .filter_map(|imp| {
                        self.select_impl_method_candidate(
                            imp,
                            candidate,
                            &candidate_ty,
                            method_name,
                        )
                        .map(|selected| (imp.id, selected))
                    })
                    .collect::<Vec<_>>();
                candidates.sort_by_key(|candidate| candidate.0);
                candidates.dedup_by_key(|candidate| candidate.0);
                found.extend(candidates.into_iter().map(|candidate| candidate.1));

                found
            })
    }

    fn select_impl_method_candidate(
        &self,
        imp: &HirImpl,
        candidate: &ReceiverCandidate,
        candidate_ty: &Type,
        method_name: &str,
    ) -> Option<SelectedMethod> {
        if !function_has_receiver(&self.impl_method(imp, method_name)?) {
            return None;
        }
        if !self.impl_matches_receiver_type(imp, candidate_ty) {
            return None;
        }
        let subst = self.impl_receiver_substitution(imp, method_name, candidate_ty)?;
        self.impl_bound_obligations(imp, &subst)?;
        let receiver_adjustment = self.impl_method_self_type_adjustment(
            imp,
            method_name,
            candidate_ty,
            candidate.can_autoref_mut,
        )?;
        let receiver_adjustment = if receiver_adjustment == ReceiverAdjustment::None {
            candidate.adjustment
        } else {
            receiver_adjustment
        };
        self.instantiate_impl_method(
            imp,
            &candidate.expr,
            candidate_ty,
            method_name,
            receiver_adjustment,
        )
    }

    /// Select a method for a receiver whose nominal head is still unknown.
    ///
    /// This is deliberately limited to rigid, fully concrete implementation
    /// patterns.  It materializes a unique method obligation without guessing
    /// an HKT constructor or an abstract receiver head.
    pub fn select_inferred_method_candidates<F>(
        &self,
        receiver: &HirExpr,
        method_name: &str,
        mut normalize: F,
    ) -> Vec<SelectedMethod>
    where
        F: FnMut(&Type) -> Type,
    {
        self.impl_ids_in_order()
            .filter_map(|id| self.impls.get(&id))
            .filter_map(|imp| {
                if !function_has_receiver(&self.impl_method(imp, method_name)?) {
                    return None;
                }
                let receiver_ty = inferred_rigid_receiver_type(&imp.receiver_pattern)?;
                let synthetic_receiver = HirExpr {
                    ty: receiver_ty,
                    kind: receiver.kind.clone(),
                    span: receiver.span.clone(),
                };
                let candidate = ReceiverCandidate {
                    expr: synthetic_receiver,
                    adjustment: ReceiverAdjustment::None,
                    can_autoref_mut: false,
                };
                let candidate_ty = normalize(&candidate.expr.ty);
                let selected =
                    self.select_impl_method_candidate(imp, &candidate, &candidate_ty, method_name)?;
                selected.pending_impl_bounds.is_empty().then_some(selected)
            })
            .collect()
    }

    pub fn mut_receiver_method_requires_mutable_receiver<F>(
        &self,
        receiver_candidates: &[ReceiverCandidate],
        method_name: &str,
        mut normalize: F,
    ) -> bool
    where
        F: FnMut(&Type) -> Type,
    {
        receiver_candidates.iter().any(|candidate| {
            let candidate_ty = normalize(&candidate.expr.ty);
            self.impl_ids_in_order()
                .into_iter()
                .filter_map(|id| self.impls.get(&id))
                .any(|imp| {
                    if !self.impl_matches_receiver_type(imp, &candidate_ty) {
                        return false;
                    }
                    let Some(subst) =
                        self.impl_receiver_substitution(imp, method_name, &candidate_ty)
                    else {
                        return false;
                    };
                    if !self.impl_bounds_satisfied(imp, &subst) {
                        return false;
                    }
                    let Some(method) = self.impl_method(imp, method_name) else {
                        return false;
                    };
                    if method.self_receiver != Some(crate::types::ReceiverMode::Mut) {
                        return false;
                    }
                    self.impl_method_self_type_adjustment(imp, method_name, &candidate_ty, true)
                        .is_some()
                        && !candidate.can_autoref_mut
                })
        })
    }
}

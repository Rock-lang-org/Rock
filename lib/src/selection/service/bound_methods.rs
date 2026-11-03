use super::*;

impl<'a> SelectionService<'a> {
    pub fn select_bound_method(
        &self,
        receiver_candidates: &[ReceiverCandidate],
        bounds: &[crate::types::TraitBound],
        method_name: &str,
        self_param_ty: Type,
    ) -> Result<SelectedMethod, SelectionDiagnostic> {
        Self::prefer_non_ref_receiver_candidate(
            self.select_bound_method_candidates(
                receiver_candidates,
                bounds,
                method_name,
                self_param_ty.clone(),
            ),
            false,
            method_name,
            &self_param_ty,
        )
    }

    pub fn select_bound_method_preferring_non_ref_receiver(
        &self,
        receiver_candidates: &[ReceiverCandidate],
        bounds: &[crate::types::TraitBound],
        method_name: &str,
        self_param_ty: Type,
        receiver_is_call: bool,
    ) -> Result<SelectedMethod, SelectionDiagnostic> {
        Self::prefer_non_ref_receiver_candidate(
            self.select_bound_method_candidates(
                receiver_candidates,
                bounds,
                method_name,
                self_param_ty.clone(),
            ),
            receiver_is_call,
            method_name,
            &self_param_ty,
        )
    }

    pub fn select_bound_method_matching<M>(
        &self,
        receiver_candidates: &[ReceiverCandidate],
        bounds: &[crate::types::TraitBound],
        method_name: &str,
        self_param_ty: Type,
        receiver_is_call: bool,
        mut matches: M,
    ) -> Result<SelectedMethod, SelectionDiagnostic>
    where
        M: FnMut(&SelectedMethod) -> bool,
    {
        let candidates = self.select_bound_method_candidates(
            receiver_candidates,
            bounds,
            method_name,
            self_param_ty.clone(),
        );
        let matching = candidates
            .iter()
            .filter(|candidate| matches(candidate))
            .cloned()
            .collect::<Vec<_>>();
        // Keep a unique declared candidate on mismatch so argument checking can
        // report its expected type rather than hiding an available trait bound.
        Self::prefer_non_ref_receiver_candidate(
            if matching.is_empty() {
                candidates
            } else {
                matching
            },
            receiver_is_call,
            method_name,
            &self_param_ty,
        )
    }

    pub fn prefer_non_ref_receiver_candidate(
        mut candidates: Vec<SelectedMethod>,
        receiver_is_call: bool,
        method_name: &str,
        receiver_ty: &Type,
    ) -> Result<SelectedMethod, SelectionDiagnostic> {
        if receiver_is_call {
            let preferred = candidates
                .iter()
                .filter(|candidate| {
                    candidate
                        .function
                        .as_ref()
                        .and_then(|function| function.params.first())
                        .map_or(true, |param| !matches!(param.ty, Type::Reference { .. }))
                })
                .cloned()
                .collect::<Vec<_>>();
            if !preferred.is_empty() {
                candidates = preferred;
            }
        }

        let target_ids = candidates
            .iter()
            .filter_map(|candidate| {
                Some((
                    candidate.target.trait_id()?,
                    candidate.target.method_id()?,
                    candidate.target.trait_args().to_vec(),
                ))
            })
            .collect::<std::collections::HashSet<_>>();
        if target_ids.len() != 1 {
            return Err(if target_ids.is_empty() {
                SelectionDiagnostic::NoImplementation {
                    operation: method_name.to_string(),
                    receiver: receiver_ty.clone(),
                }
            } else {
                let mut candidate_ids = target_ids
                    .into_iter()
                    .map(|(_, method_id, _)| method_id)
                    .collect::<Vec<_>>();
                candidate_ids.sort();
                candidate_ids.dedup();
                SelectionDiagnostic::AmbiguousCandidates {
                    operation: method_name.to_string(),
                    receiver: receiver_ty.clone(),
                    candidates: candidate_ids,
                }
            });
        }
        candidates
            .into_iter()
            .next()
            .ok_or_else(|| SelectionDiagnostic::NoImplementation {
                operation: method_name.to_string(),
                receiver: receiver_ty.clone(),
            })
    }

    fn select_bound_method_candidates(
        &self,
        receiver_candidates: &[ReceiverCandidate],
        bounds: &[crate::types::TraitBound],
        method_name: &str,
        self_param_ty: Type,
    ) -> Vec<SelectedMethod> {
        let mut candidates = Vec::new();

        for bound in self
            .trait_bounds_with_supertraits(&self_param_ty, bounds)
            .iter()
        {
            let Some(trait_def) = self
                .trait_def_for_member(bound.trait_id, method_name)
                .cloned()
            else {
                continue;
            };
            let trait_subst = crate::selection::generic_substitution_for_owner(
                &trait_def.generic_params,
                &bound.type_args,
            );

            if let Some(method_template) = trait_def.methods.get(method_name) {
                for receiver_candidate in receiver_candidates {
                    let mut method_func = method_template.clone();
                    let method_id = method_func.id;
                    for param in &mut method_func.params {
                        param.ty = param.ty.substitute_generics(&trait_subst);
                    }
                    let mut receiver_subst = HashMap::new();
                    let mut receiver_adjustment = receiver_candidate.adjustment;
                    if method_func.is_method {
                        let Some(self_param) = method_func.params.first() else {
                            continue;
                        };
                        let Some(adjustment) = Self::bound_receiver_adjustment(
                            &self_param.ty,
                            method_func.self_receiver,
                            &self_param_ty,
                            receiver_candidate,
                            &mut receiver_subst,
                        ) else {
                            continue;
                        };
                        receiver_adjustment = adjustment;
                        for param in &mut method_func.params {
                            param.ty = param.ty.substitute_generics(&receiver_subst);
                        }
                    }
                    method_func.ret_type = method_func
                        .ret_type
                        .substitute_generics(&trait_subst)
                        .substitute_generics(&receiver_subst);
                    let target = HirMethodCallTarget::trait_method(
                        trait_def.id,
                        method_id,
                        bound.type_args.clone(),
                        HirTraitDispatchKind::TraitBound,
                    );
                    candidates.push(SelectedMethod {
                        receiver: receiver_candidate.expr.clone(),
                        substituted_params: method_func.params
                            [usize::from(method_func.is_method)..]
                            .to_vec(),
                        return_type: method_func.ret_type.clone(),
                        function: Some(method_func),
                        impl_def: None,
                        target,
                        origin: SelectedOrigin::TraitBound {
                            trait_id: trait_def.id,
                        },
                        receiver_adjustment,
                        pending_impl_bounds: Vec::new(),
                        associated_types: Vec::new(),
                        owner_substitution: HashMap::new(),
                        owner_generic_params: Vec::new(),
                    });
                }
            }

            if let Some(sig) = trait_def.signatures.get(method_name) {
                let sig_params_template: Vec<_> = sig
                    .params
                    .iter()
                    .map(|ty| ty.substitute_generics(&trait_subst))
                    .collect();
                for receiver_candidate in receiver_candidates {
                    let mut receiver_subst = HashMap::new();
                    let mut receiver_adjustment = receiver_candidate.adjustment;
                    if sig.self_receiver.is_some() {
                        let Some(self_param) = sig_params_template.first() else {
                            continue;
                        };
                        let Some(adjustment) = Self::bound_receiver_adjustment(
                            self_param,
                            sig.self_receiver,
                            &self_param_ty,
                            receiver_candidate,
                            &mut receiver_subst,
                        ) else {
                            continue;
                        };
                        receiver_adjustment = adjustment;
                    }
                    let sig_params: Vec<_> = sig_params_template
                        .iter()
                        .map(|ty| ty.substitute_generics(&receiver_subst))
                        .collect();
                    let sig_ret = sig
                        .ret
                        .substitute_generics(&trait_subst)
                        .substitute_generics(&receiver_subst);
                    let sig_bounds: HashMap<_, _> = sig
                        .generic_bounds
                        .iter()
                        .map(|(generic_param, bounds)| {
                            (
                                *generic_param,
                                bounds
                                    .iter()
                                    .map(|bound| crate::types::TraitBound {
                                        trait_id: bound.trait_id,
                                        type_args: bound
                                            .type_args
                                            .iter()
                                            .map(|arg| {
                                                arg.substitute_generics(&trait_subst)
                                                    .substitute_generics(&receiver_subst)
                                            })
                                            .collect(),
                                    })
                                    .collect(),
                            )
                        })
                        .collect();
                    let params = sig_params
                        .iter()
                        .enumerate()
                        .map(|(i, ty)| crate::hir::HirParam {
                            name: if sig.self_receiver.is_some() && i == 0 {
                                "self".to_string()
                            } else {
                                format!("arg{}", i)
                            },
                            local_id: crate::ids::HirLocalId(i as u32),
                            ty: ty.clone(),
                            mutable: sig.self_receiver.is_some_and(|self_receiver| {
                                i == 0 && matches!(self_receiver, crate::types::ReceiverMode::Mut)
                            }),
                            is_ref: false,
                        })
                        .collect();
                    let method_func = crate::hir::HirFunction {
                        id: sig.id,
                        name: method_name.to_string(),
                        generic_params: sig.generic_params.clone(),
                        generic_bounds: sig_bounds.into(),
                        params,
                        ret_type: sig_ret.clone(),
                        body: crate::hir::HirBlock {
                            stmts: Vec::new(),
                            ty: sig_ret.clone(),
                        },
                        is_curried: false,
                        is_method: sig.self_receiver.is_some(),
                        self_receiver: sig.self_receiver,
                        is_unsafe: sig.is_unsafe,
                    };
                    let target = HirMethodCallTarget::trait_method(
                        trait_def.id,
                        sig.id,
                        bound.type_args.clone(),
                        HirTraitDispatchKind::TraitBound,
                    );
                    candidates.push(SelectedMethod {
                        receiver: receiver_candidate.expr.clone(),
                        substituted_params: method_func.params
                            [usize::from(method_func.is_method)..]
                            .to_vec(),
                        return_type: sig_ret,
                        function: Some(method_func),
                        impl_def: None,
                        target,
                        origin: SelectedOrigin::TraitBound {
                            trait_id: trait_def.id,
                        },
                        receiver_adjustment,
                        pending_impl_bounds: Vec::new(),
                        associated_types: Vec::new(),
                        owner_substitution: HashMap::new(),
                        owner_generic_params: Vec::new(),
                    });
                }
            }
        }

        candidates
    }

    fn bound_receiver_adjustment(
        self_param: &Type,
        self_receiver: Option<crate::types::ReceiverMode>,
        self_param_ty: &Type,
        candidate: &ReceiverCandidate,
        receiver_subst: &mut HashMap<GenericParamId, Type>,
    ) -> Option<ReceiverAdjustment> {
        let self_value_ty = Self::receiver_mode_self_value_type(self_param.clone(), self_receiver);
        if !Self::type_pattern_matches_committing(&self_value_ty, self_param_ty, receiver_subst) {
            return None;
        }

        let expected_self = Self::receiver_mode_expected_self_type(
            self_param.substitute_generics(receiver_subst),
            self_receiver,
        );
        if Self::bound_candidate_would_double_borrow(&self_value_ty, self_param_ty, candidate) {
            return None;
        }

        let adjustment = Self::self_type_adjustment(
            &expected_self,
            &candidate.expr.ty,
            candidate.can_autoref_mut,
            true,
            true,
            receiver_subst,
        )?;

        Some(if adjustment == ReceiverAdjustment::None {
            candidate.adjustment
        } else {
            adjustment
        })
    }

    fn receiver_mode_self_value_type(
        ty: Type,
        self_receiver: Option<crate::types::ReceiverMode>,
    ) -> Type {
        match self_receiver {
            Some(crate::types::ReceiverMode::Shared) | Some(crate::types::ReceiverMode::Mut) => {
                match ty {
                    Type::Reference { inner, .. } => *inner,
                    ty => ty,
                }
            }
            Some(crate::types::ReceiverMode::Move) | None => ty,
        }
    }

    fn bound_candidate_would_double_borrow(
        self_value_ty: &Type,
        self_param_ty: &Type,
        candidate: &ReceiverCandidate,
    ) -> bool {
        if candidate.expr.ty == *self_param_ty {
            return false;
        }
        if candidate.adjustment != ReceiverAdjustment::None {
            return false;
        }

        let Type::Reference { inner, .. } = &candidate.expr.ty else {
            return false;
        };

        let mut subst = HashMap::new();
        Self::type_pattern_matches_committing(self_value_ty, self_param_ty, &mut subst)
            && Self::type_pattern_matches_committing(self_value_ty, inner, &mut subst)
    }
}

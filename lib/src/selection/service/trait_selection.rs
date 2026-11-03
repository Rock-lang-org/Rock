use super::*;

impl<'a> SelectionService<'a> {
    pub fn select_required_trait_method(
        &self,
        receiver: &HirExpr,
        receiver_ty: &Type,
        trait_id: DefId,
        method_name: &str,
    ) -> Result<SelectedMethod, SelectionDiagnostic> {
        let member_id = self.trait_member_id(trait_id, method_name).ok_or_else(|| {
            SelectionDiagnostic::TraitMemberMissing {
                trait_id,
                method_name: method_name.to_string(),
            }
        })?;
        self.select_trait_impl_matching(receiver_ty, trait_id, None, |imp| {
            self.impl_method_self_type_adjustment(imp, method_name, receiver_ty, false)
                .is_some()
        })
        .and_then(|imp| {
            let receiver_adjustment = self
                .impl_method_self_type_adjustment(imp, method_name, receiver_ty, false)
                .ok_or_else(|| SelectionDiagnostic::ReceiverMismatch {
                    operation: method_name.to_string(),
                    receiver: receiver_ty.clone(),
                })?;
            self.instantiate_impl_method(
                imp,
                receiver,
                receiver_ty,
                method_name,
                receiver_adjustment,
            )
            .ok_or_else(|| SelectionDiagnostic::SelectedTargetMissing {
                method_name: method_name.to_string(),
                target: HirMethodCallTarget::trait_method(
                    trait_id,
                    member_id,
                    Vec::new(),
                    HirTraitDispatchKind::TraitBound,
                ),
            })
        })
    }

    pub fn select_required_trait_member(
        &self,
        receiver: &HirExpr,
        receiver_ty: &Type,
        trait_id: DefId,
        member_id: DefId,
    ) -> Result<SelectedMethod, SelectionDiagnostic> {
        let method_name = self.trait_member_name_by_id(trait_id, member_id).ok_or(
            SelectionDiagnostic::TraitMemberIdMissing {
                trait_id,
                member_id,
            },
        )?;
        self.select_trait_impl_matching(receiver_ty, trait_id, None, |imp| {
            let Some(method) = self.impl_method_for_trait_member(imp, member_id) else {
                return false;
            };
            self.method_self_type_adjustment(imp, &method.function, receiver_ty, false)
                .is_some()
        })
        .and_then(|imp| {
            let method = self
                .impl_method_for_trait_member(imp, member_id)
                .ok_or_else(|| SelectionDiagnostic::SelectedTargetMissing {
                    method_name: method_name.to_string(),
                    target: HirMethodCallTarget::trait_method(
                        trait_id,
                        member_id,
                        Vec::new(),
                        HirTraitDispatchKind::TraitBound,
                    ),
                })?;
            let receiver_adjustment = self
                .method_self_type_adjustment(imp, &method.function, receiver_ty, false)
                .ok_or_else(|| SelectionDiagnostic::ReceiverMismatch {
                    operation: method_name.to_string(),
                    receiver: receiver_ty.clone(),
                })?;
            self.instantiate_method_function_with_subst(
                imp,
                receiver,
                receiver_ty,
                method.function,
                Some(member_id),
                method.inherited_default,
                HashMap::new(),
                receiver_adjustment,
            )
            .ok_or_else(|| SelectionDiagnostic::SelectedTargetMissing {
                method_name: method_name.to_string(),
                target: HirMethodCallTarget::trait_method(
                    trait_id,
                    member_id,
                    Vec::new(),
                    HirTraitDispatchKind::TraitBound,
                ),
            })
        })
    }

    pub fn select_trait_impl(
        &self,
        receiver_ty: &Type,
        trait_id: DefId,
        trait_args: &[Type],
    ) -> Result<&'a HirImpl, SelectionDiagnostic> {
        self.select_trait_impl_matching(receiver_ty, trait_id, Some(trait_args), |_| true)
    }

    /// Infer a bound's arguments from a unique, fully applicable receiver impl.
    /// This supplies callback input types before lowering an inline lambda body.
    pub fn infer_trait_arguments(&self, receiver_ty: &Type, trait_id: DefId) -> Option<Vec<Type>> {
        let mut candidates = self.impl_ids_in_order().filter_map(|id| {
            let imp = self.impls.get(&id)?;
            if imp.trait_id != Some(trait_id) {
                return None;
            }
            let subst = impl_receiver_pattern_substitution(imp, receiver_ty, self.impls.values())?;
            if !self.impl_bound_obligations(imp, &subst)?.is_empty() {
                return None;
            }
            let args = imp
                .trait_arg_types
                .iter()
                .map(|ty| ty.substitute_generics(&subst))
                .collect::<Vec<_>>();
            (!args.iter().any(Self::type_contains_type_var)).then_some(args)
        });
        let args = candidates.next()?;
        candidates.next().is_none().then_some(args)
    }

    pub fn select_trait_impl_strict(
        &self,
        receiver_ty: &Type,
        trait_id: DefId,
        trait_args: &[Type],
    ) -> Result<&'a HirImpl, SelectionDiagnostic> {
        self.unique_matching_impl(
            self.impl_ids_in_order()
                .into_iter()
                .filter_map(|id| self.impls.get(&id))
                .filter(|imp| {
                    self.impl_matches_trait_ref_strict(imp, receiver_ty, trait_id, Some(trait_args))
                })
                .collect(),
            receiver_ty,
            self.traits
                .get(&trait_id)
                .map(|trait_def| format!("trait '{}'", trait_def.name))
                .unwrap_or_else(|| "the requested trait".to_string()),
        )
    }

    pub fn select_static_trait_method(
        &self,
        owner: &HirExpr,
        owner_ty: &Type,
        trait_id: DefId,
        trait_args: &[Type],
        method_name: &str,
    ) -> Result<SelectedMethod, SelectionDiagnostic> {
        let mut candidates = self
            .impl_ids_in_order()
            .into_iter()
            .filter_map(|id| self.impls.get(&id))
            .filter_map(|imp| {
                let subst =
                    self.impl_trait_ref_substitution(imp, owner_ty, trait_id, Some(trait_args))?;
                if imp
                    .methods
                    .get(method_name)
                    .is_none_or(function_has_receiver)
                {
                    return None;
                }
                self.instantiate_impl_method_with_subst(
                    imp,
                    owner,
                    owner_ty,
                    method_name,
                    subst,
                    ReceiverAdjustment::None,
                )
                .map(|selected| (imp.id, selected))
            })
            .collect::<Vec<_>>();
        candidates.sort_by_key(|candidate| candidate.0);
        candidates.dedup_by_key(|candidate| candidate.0);
        match candidates.as_slice() {
            [(_, selected)] => Ok(selected.clone()),
            [] => Err(SelectionDiagnostic::NoImplementation {
                operation: method_name.to_string(),
                receiver: owner_ty.clone(),
            }),
            _ => Err(SelectionDiagnostic::AmbiguousCandidates {
                operation: method_name.to_string(),
                receiver: owner_ty.clone(),
                candidates: candidates.iter().map(|(impl_id, _)| *impl_id).collect(),
            }),
        }
    }

    pub fn select_static_trait_member(
        &self,
        owner: &HirExpr,
        owner_ty: &Type,
        trait_id: DefId,
        trait_args: &[Type],
        member_id: DefId,
    ) -> Result<SelectedMethod, SelectionDiagnostic> {
        let method_name = self.trait_member_name_by_id(trait_id, member_id).ok_or(
            SelectionDiagnostic::TraitMemberIdMissing {
                trait_id,
                member_id,
            },
        )?;
        let mut candidates = self
            .impl_ids_in_order()
            .filter_map(|id| self.impls.get(&id))
            .filter_map(|imp| {
                let subst =
                    self.impl_trait_ref_substitution(imp, owner_ty, trait_id, Some(trait_args))?;
                let method = self.impl_method_for_trait_member(imp, member_id)?;
                if function_has_receiver(&method.function) {
                    return None;
                }
                self.instantiate_method_function_with_subst(
                    imp,
                    owner,
                    owner_ty,
                    method.function,
                    Some(member_id),
                    method.inherited_default,
                    subst,
                    ReceiverAdjustment::None,
                )
                .map(|selected| (imp.id, selected))
            })
            .collect::<Vec<_>>();
        candidates.sort_by_key(|candidate| candidate.0);
        candidates.dedup_by_key(|candidate| candidate.0);
        match candidates.as_slice() {
            [(_, selected)] => Ok(selected.clone()),
            [] => Err(SelectionDiagnostic::NoImplementation {
                operation: method_name.to_string(),
                receiver: owner_ty.clone(),
            }),
            _ => Err(SelectionDiagnostic::AmbiguousCandidates {
                operation: method_name.to_string(),
                receiver: owner_ty.clone(),
                candidates: candidates.iter().map(|(impl_id, _)| *impl_id).collect(),
            }),
        }
    }

    pub fn select_constructor_trait_member(
        &self,
        target: &Type,
        trait_id: DefId,
        trait_args: &[Type],
        member_id: DefId,
    ) -> Result<SelectedConstructorMember, SelectionDiagnostic> {
        let method_name = self.trait_member_name_by_id(trait_id, member_id).ok_or(
            SelectionDiagnostic::TraitMemberIdMissing {
                trait_id,
                member_id,
            },
        )?;
        let mut candidates = self
            .impl_ids_in_order()
            .filter_map(|id| self.impls.get(&id))
            .filter_map(|imp| {
                let mut subst = constructor_target_substitution(&imp.receiver_pattern, target)?;
                if let Some(target_param) = self
                    .traits
                    .get(&trait_id)
                    .and_then(|trait_def| trait_def.target.as_ref())
                {
                    subst.insert(target_param.id, target.clone());
                }
                if imp.trait_id != Some(trait_id)
                    || imp.trait_arg_types.len() != trait_args.len()
                    || !imp
                        .trait_arg_types
                        .iter()
                        .zip(trait_args)
                        .all(|(expected, actual)| {
                            type_pattern_matches(expected, actual, &mut subst)
                        })
                {
                    return None;
                }
                let method = self.impl_method_for_trait_member(imp, member_id)?;
                if function_has_receiver(&method.function) {
                    return None;
                }
                self.instantiate_constructor_member_with_subst(
                    imp,
                    method.function,
                    member_id,
                    subst,
                )
                .map(|selected| (imp.id, selected))
            })
            .collect::<Vec<_>>();
        candidates.sort_by_key(|candidate| candidate.0);
        candidates.dedup_by_key(|candidate| candidate.0);
        match candidates.as_slice() {
            [(_, selected)] => Ok(selected.clone()),
            [] => Err(SelectionDiagnostic::NoImplementation {
                operation: method_name.to_string(),
                receiver: target.clone(),
            }),
            _ => Err(SelectionDiagnostic::AmbiguousCandidates {
                operation: format!(
                    "{method_name} (upstream coherence invariant violation: overlapping constructor impls reached selection)"
                ),
                receiver: target.clone(),
                candidates: candidates.iter().map(|(impl_id, _)| *impl_id).collect(),
            }),
        }
    }

    pub fn infer_constructor_target_for_applied_type(
        &self,
        actual: &Type,
        trait_id: DefId,
        trait_args: &[Type],
    ) -> Result<Option<Type>, SelectionDiagnostic> {
        let mut candidates = self
            .impl_ids_in_order()
            .filter_map(|id| self.impls.get(&id))
            .filter_map(|imp| {
                if imp.trait_id != Some(trait_id) || imp.trait_arg_types.len() != trait_args.len() {
                    return None;
                }
                let (target, mut subst) =
                    constructor_target_from_applied_type(&imp.receiver_pattern, actual)?;
                imp.trait_arg_types
                    .iter()
                    .zip(trait_args)
                    .all(|(expected, actual)| type_pattern_matches(expected, actual, &mut subst))
                    .then_some((imp.id, target))
            })
            .collect::<Vec<_>>();
        candidates.sort_by_key(|candidate| candidate.0);
        candidates.dedup_by(|left, right| left.1 == right.1);
        match candidates.as_slice() {
            [(_, target)] => Ok(Some(target.clone())),
            [] => Ok(None),
            _ => Err(SelectionDiagnostic::AmbiguousCandidates {
                operation: "type-constructor inference".to_string(),
                receiver: actual.clone(),
                candidates: candidates.iter().map(|(impl_id, _)| *impl_id).collect(),
            }),
        }
    }

    fn select_trait_impl_matching<F>(
        &self,
        receiver_ty: &Type,
        trait_id: DefId,
        trait_args: Option<&[Type]>,
        mut predicate: F,
    ) -> Result<&'a HirImpl, SelectionDiagnostic>
    where
        F: FnMut(&HirImpl) -> bool,
    {
        self.unique_matching_impl(
            self.impl_ids_in_order()
                .into_iter()
                .filter_map(|id| self.impls.get(&id))
                .filter(|imp| {
                    self.impl_matches_trait_ref(imp, receiver_ty, trait_id, trait_args)
                        && predicate(imp)
                })
                .collect(),
            receiver_ty,
            self.traits
                .get(&trait_id)
                .map(|trait_def| format!("trait '{}'", trait_def.name))
                .unwrap_or_else(|| "the requested trait".to_string()),
        )
    }

    fn unique_matching_impl(
        &self,
        mut candidates: Vec<&'a HirImpl>,
        receiver_ty: &Type,
        operation: String,
    ) -> Result<&'a HirImpl, SelectionDiagnostic> {
        candidates.sort_by_key(|imp| imp.id);
        candidates.dedup_by_key(|imp| imp.id);
        match candidates.as_slice() {
            [imp] => Ok(*imp),
            [] => Err(SelectionDiagnostic::NoImplementation {
                operation,
                receiver: receiver_ty.clone(),
            }),
            _ => Err(SelectionDiagnostic::AmbiguousCandidates {
                operation: format!(
                    "{operation} (upstream coherence invariant violation: overlapping impls reached selection)"
                ),
                receiver: receiver_ty.clone(),
                candidates: candidates.iter().map(|imp| imp.id).collect(),
            }),
        }
    }
}

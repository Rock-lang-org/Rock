use super::*;

impl<'a> SelectionService<'a> {
    pub fn select_unary_operator_method(
        &self,
        receiver: &HirExpr,
        receiver_ty: &Type,
        trait_id: DefId,
        method_name: &str,
    ) -> Result<SelectedMethod, SelectionDiagnostic> {
        self.select_required_trait_method(receiver, receiver_ty, trait_id, method_name)
    }

    pub fn select_current_trait_method(
        &self,
        receiver: HirExpr,
        method_name: &str,
        receiver_is_current_trait_self: bool,
    ) -> Result<SelectedMethod, SelectionDiagnostic> {
        if !receiver_is_current_trait_self {
            return Err(SelectionDiagnostic::ReceiverMismatch {
                operation: method_name.to_string(),
                receiver: receiver.ty.clone(),
            });
        }

        let trait_id = self
            .current_trait
            .ok_or_else(|| SelectionDiagnostic::NoImplementation {
                operation: method_name.to_string(),
                receiver: receiver.ty.clone(),
            })?;
        let current_trait = self
            .traits
            .values()
            .find(|trait_def| trait_def.id == trait_id)
            .ok_or_else(|| SelectionDiagnostic::NoImplementation {
                operation: method_name.to_string(),
                receiver: receiver.ty.clone(),
            })?;
        let trait_def = self
            .trait_def_for_member(current_trait.id, method_name)
            .unwrap_or(current_trait);
        let trait_args = trait_def
            .generic_params
            .iter()
            .map(|decl| Type::Generic(decl.id))
            .collect::<Vec<_>>();

        if let Some(method_func) = trait_def.methods.get(method_name).cloned() {
            return Ok(Self::selected_current_trait_method(
                receiver,
                trait_def.id,
                trait_args,
                method_func,
            ));
        }

        let sig = trait_def.signatures.get(method_name).ok_or_else(|| {
            SelectionDiagnostic::NoImplementation {
                operation: method_name.to_string(),
                receiver: receiver.ty.clone(),
            }
        })?;
        if sig.self_receiver.is_some() && sig.params.is_empty() {
            return Err(SelectionDiagnostic::SelectedTargetMissing {
                method_name: method_name.to_string(),
                target: HirMethodCallTarget::trait_method(
                    trait_def.id,
                    sig.id,
                    trait_args,
                    HirTraitDispatchKind::CurrentTrait,
                ),
            });
        }
        let params = sig
            .params
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
            .collect::<Vec<_>>();
        let method_func = crate::hir::HirFunction {
            id: sig.id,
            name: method_name.to_string(),
            generic_params: sig.generic_params.clone(),
            generic_bounds: crate::hir::HirGenericBounds::new(),
            params,
            ret_type: sig.ret.clone(),
            body: crate::hir::HirBlock {
                stmts: Vec::new(),
                ty: sig.ret.clone(),
            },
            is_curried: false,
            is_method: sig.self_receiver.is_some(),
            self_receiver: sig.self_receiver,
            is_unsafe: sig.is_unsafe,
        };

        Ok(Self::selected_current_trait_method(
            receiver,
            trait_def.id,
            trait_args,
            method_func,
        ))
    }

    pub fn select_unresolved_generic_method(
        &self,
        receiver: HirExpr,
        trait_id: DefId,
        method_name: &str,
        trait_args: Vec<Type>,
        return_type: Type,
    ) -> Result<SelectedMethod, SelectionDiagnostic> {
        let trait_def = self
            .trait_def_for_member(trait_id, method_name)
            .ok_or_else(|| SelectionDiagnostic::NoImplementation {
                operation: method_name.to_string(),
                receiver: receiver.ty.clone(),
            })?;
        let method_id = self.trait_member_id(trait_id, method_name).ok_or_else(|| {
            SelectionDiagnostic::SelectedTargetMissing {
                method_name: method_name.to_string(),
                target: HirMethodCallTarget::trait_method(
                    trait_id,
                    trait_id,
                    trait_args.clone(),
                    HirTraitDispatchKind::UnresolvedGeneric,
                ),
            }
        })?;
        let mut method_func = if let Some(method) = trait_def.methods.get(method_name).cloned() {
            method
        } else {
            let sig = trait_def.signatures.get(method_name).ok_or_else(|| {
                SelectionDiagnostic::SelectedTargetMissing {
                    method_name: method_name.to_string(),
                    target: HirMethodCallTarget::trait_method(
                        trait_id,
                        method_id,
                        trait_args.clone(),
                        HirTraitDispatchKind::UnresolvedGeneric,
                    ),
                }
            })?;
            let params = sig
                .params
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
            crate::hir::HirFunction {
                id: method_id,
                name: method_name.to_string(),
                generic_params: sig.generic_params.clone(),
                generic_bounds: crate::hir::HirGenericBounds::new(),
                params,
                ret_type: return_type.clone(),
                body: crate::hir::HirBlock {
                    stmts: Vec::new(),
                    ty: return_type.clone(),
                },
                is_curried: false,
                is_method: sig.self_receiver.is_some(),
                self_receiver: sig.self_receiver,
                is_unsafe: sig.is_unsafe,
            }
        };
        method_func.id = method_id;
        method_func.ret_type = return_type.clone();
        let target = HirMethodCallTarget::trait_method(
            trait_id,
            method_id,
            trait_args,
            HirTraitDispatchKind::UnresolvedGeneric,
        );
        let param_start = if method_func.is_method { 1 } else { 0 };
        let substituted_params = method_func.params[param_start..].to_vec();

        Ok(SelectedMethod {
            receiver,
            function: Some(method_func),
            impl_def: None,
            target,
            origin: SelectedOrigin::UnresolvedGeneric { trait_id },
            receiver_adjustment: ReceiverAdjustment::None,
            substituted_params,
            return_type,
            pending_impl_bounds: Vec::new(),
            associated_types: Vec::new(),
            owner_substitution: HashMap::new(),
            owner_generic_params: Vec::new(),
        })
    }

    pub fn select_unresolved_generic_required_index_method(
        &self,
        receiver: HirExpr,
        trait_id: DefId,
        member_id: DefId,
        _output_id: AssocTypeId,
        index_ty: Type,
        return_type: Type,
        operation: &'static str,
    ) -> Result<SelectedMethod, SelectionDiagnostic> {
        let _ = operation;
        let trait_def = self.trait_by_id(trait_id).ok_or_else(|| {
            SelectionDiagnostic::TraitMemberIdMissing {
                trait_id,
                member_id,
            }
        })?;
        let method_func = if let Some(method) = trait_def
            .methods
            .values()
            .find(|method| method.id == member_id)
            .cloned()
        {
            method
        } else {
            let signature = trait_def
                .signatures
                .values()
                .find(|signature| signature.id == member_id)
                .ok_or(SelectionDiagnostic::TraitMemberIdMissing {
                    trait_id,
                    member_id,
                })?;
            let params = signature
                .params
                .iter()
                .enumerate()
                .map(|(index, ty)| crate::hir::HirParam {
                    name: if signature.self_receiver.is_some() && index == 0 {
                        "self".to_string()
                    } else {
                        format!("arg{index}")
                    },
                    local_id: crate::ids::HirLocalId(index as u32),
                    ty: ty.clone(),
                    mutable: signature.self_receiver.is_some_and(|self_receiver| {
                        index == 0 && matches!(self_receiver, crate::types::ReceiverMode::Mut)
                    }),
                    is_ref: false,
                })
                .collect();
            crate::hir::HirFunction {
                id: member_id,
                name: signature.name.clone(),
                generic_params: signature.generic_params.clone(),
                generic_bounds: crate::hir::HirGenericBounds::new(),
                params,
                ret_type: return_type.clone(),
                body: crate::hir::HirBlock {
                    stmts: Vec::new(),
                    ty: return_type.clone(),
                },
                is_curried: false,
                is_method: signature.self_receiver.is_some(),
                self_receiver: signature.self_receiver,
                is_unsafe: signature.is_unsafe,
            }
        };
        let param_start = if method_func.is_method { 1 } else { 0 };
        let substituted_params = method_func.params[param_start..].to_vec();

        Ok(SelectedMethod {
            receiver,
            function: Some(method_func),
            impl_def: None,
            target: HirMethodCallTarget::trait_method(
                trait_id,
                member_id,
                vec![index_ty],
                HirTraitDispatchKind::UnresolvedGeneric,
            ),
            origin: SelectedOrigin::UnresolvedGeneric { trait_id },
            receiver_adjustment: ReceiverAdjustment::None,
            substituted_params,
            return_type,
            pending_impl_bounds: Vec::new(),
            associated_types: Vec::new(),
            owner_substitution: HashMap::new(),
            owner_generic_params: Vec::new(),
        })
    }

    pub fn select_required_index_method<F>(
        &self,
        receiver_candidates: &[ReceiverCandidate],
        index_trait_id: DefId,
        index_member_id: DefId,
        _index_output_id: AssocTypeId,
        index_ty: &Type,
        operation: &'static str,
        mut normalize: F,
    ) -> Result<SelectedMethod, SelectionDiagnostic>
    where
        F: FnMut(&Type) -> Type,
    {
        for candidate in receiver_candidates {
            let candidate_ty = normalize(&candidate.expr.ty);

            let mut candidates = self
                .impls
                .values()
                .filter_map(|imp| {
                    if imp.trait_id != Some(index_trait_id)
                        || !self.impl_matches_receiver_type(imp, &candidate_ty)
                    {
                        return None;
                    }
                    let method = self.impl_method_for_trait_member(imp, index_member_id)?;
                    let receiver_adjustment = self.method_self_type_adjustment(
                        imp,
                        &method.function,
                        &candidate_ty,
                        candidate.can_autoref_mut,
                    )?;
                    let receiver_adjustment = if receiver_adjustment == ReceiverAdjustment::None {
                        candidate.adjustment
                    } else {
                        receiver_adjustment
                    };
                    self.impl_index_args_substitution(
                        imp,
                        index_member_id,
                        &candidate_ty,
                        std::slice::from_ref(index_ty),
                    )
                    .filter(|subst| self.impl_bound_obligations(imp, subst).is_some())
                    .map(|subst| (imp, method, subst, receiver_adjustment))
                })
                .collect::<Vec<_>>();
            candidates.sort_by_key(|entry| entry.0.id);
            candidates.dedup_by_key(|entry| entry.0.id);
            if candidates.is_empty() {
                continue;
            }
            if candidates.len() > 1 {
                return Err(SelectionDiagnostic::AmbiguousCandidates {
                    operation: operation.to_string(),
                    receiver: candidate_ty,
                    candidates: candidates.iter().map(|(imp, _, _, _)| imp.id).collect(),
                });
            }
            let (imp, method, subst, receiver_adjustment) = candidates.remove(0);
            let mut selected = self
                .instantiate_method_function_with_subst(
                    imp,
                    &candidate.expr,
                    &candidate_ty,
                    method.function,
                    Some(index_member_id),
                    method.inherited_default,
                    subst,
                    receiver_adjustment,
                )
                .ok_or_else(|| SelectionDiagnostic::SelectedTargetMissing {
                    method_name: operation.to_string(),
                    target: HirMethodCallTarget::trait_method(
                        index_trait_id,
                        index_member_id,
                        vec![index_ty.clone()],
                        HirTraitDispatchKind::TraitBound,
                    ),
                })?;
            if let Some(trait_args) = selected.target.trait_args_mut() {
                *trait_args = vec![index_ty.clone()];
            }
            return Ok(selected);
        }

        Err(SelectionDiagnostic::NoImplementation {
            operation: operation.to_string(),
            receiver: receiver_candidates
                .first()
                .map(|candidate| normalize(&candidate.expr.ty))
                .unwrap_or(Type::Error),
        })
    }
}

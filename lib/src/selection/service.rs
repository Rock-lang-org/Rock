use std::collections::HashMap;
use std::sync::OnceLock;

use crate::hir::{
    function_has_receiver, HirAssociatedTypeDef, HirExpr, HirFunction, HirGenericBounds, HirImpl,
    HirImplReceiverPattern, HirMethodCallTarget, HirSelectedTraitMember, HirTrait,
    HirTraitDispatchKind, HirTypeBinding,
};
use crate::ids::{AssocTypeId, DefId};
use crate::selection::matching::{
    constructor_target_from_applied_type, constructor_target_substitution,
    impl_receiver_pattern_substitution, type_pattern_matches,
};
use crate::selection::types::{
    ReceiverAdjustment, ReceiverCandidate, SelectedConstructorMember, SelectedMethod,
    SelectedOrigin, SelectionDiagnostic,
};
use crate::types::{GenericParamId, TraitBound, Type};

pub struct SelectionService<'a> {
    traits: &'a HashMap<DefId, HirTrait>,
    impls: &'a HashMap<DefId, HirImpl>,
    impl_ids: Vec<DefId>,
    sized_trait_id: Option<DefId>,
    #[allow(dead_code)]
    current_trait: Option<DefId>,
    #[allow(dead_code)]
    current_impl_bounds: &'a HirGenericBounds,
    effective_trait_methods: &'a HashMap<(DefId, DefId), DefId>,
}

struct EffectiveTraitMethod {
    function: HirFunction,
    inherited_default: bool,
}

fn empty_effective_trait_methods() -> &'static HashMap<(DefId, DefId), DefId> {
    static EMPTY: OnceLock<HashMap<(DefId, DefId), DefId>> = OnceLock::new();
    EMPTY.get_or_init(HashMap::new)
}

impl<'a> SelectionService<'a> {
    pub fn new(
        traits: &'a HashMap<DefId, HirTrait>,
        impls: &'a HashMap<DefId, HirImpl>,
        sized_trait_id: Option<DefId>,
        current_trait: Option<DefId>,
        current_impl_bounds: &'a HirGenericBounds,
    ) -> Self {
        let mut impl_ids: Vec<_> = impls.keys().copied().collect();
        impl_ids.sort();
        Self {
            traits,
            impls,
            impl_ids,
            sized_trait_id,
            current_trait,
            current_impl_bounds,
            effective_trait_methods: empty_effective_trait_methods(),
        }
    }

    pub fn with_effective_trait_methods(
        mut self,
        effective_trait_methods: &'a HashMap<(DefId, DefId), DefId>,
    ) -> Self {
        self.effective_trait_methods = effective_trait_methods;
        self
    }

    fn impl_ids_in_order(&self) -> impl Iterator<Item = DefId> + '_ {
        self.impl_ids.iter().copied()
    }

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
                        if !self.impl_matches_receiver_type(imp, &candidate_ty) {
                            return None;
                        }
                        let subst =
                            self.impl_receiver_substitution(imp, method_name, &candidate_ty)?;
                        if self.impl_bound_obligations(imp, &subst).is_none() {
                            return None;
                        }
                        let receiver_adjustment = self.impl_method_self_type_adjustment(
                            imp,
                            method_name,
                            &candidate_ty,
                            candidate.can_autoref_mut,
                        )?;
                        let receiver_adjustment = if receiver_adjustment == ReceiverAdjustment::None
                        {
                            candidate.adjustment
                        } else {
                            receiver_adjustment
                        };
                        self.instantiate_impl_method(
                            imp,
                            &candidate.expr,
                            &candidate_ty,
                            method_name,
                            receiver_adjustment,
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
                self.select_concrete_method_candidates(
                    std::slice::from_ref(&candidate),
                    method_name,
                    |ty| normalize(ty),
                )
                .into_iter()
                .filter(|selected| selected.target.impl_id() == Some(imp.id))
                .into_iter()
                .filter(|selected| selected.pending_impl_bounds.is_empty())
                .map(|mut selected| {
                    selected.receiver.kind = receiver.kind.clone();
                    selected.receiver.span = receiver.span.clone();
                    selected
                })
                .next()
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

    fn impl_method(&self, imp: &HirImpl, method_name: &str) -> Option<HirFunction> {
        if imp.trait_id.is_none() {
            return imp.methods.get(method_name).cloned();
        }
        let trait_id = imp.trait_id?;
        let member_id = self.trait_member_id(trait_id, method_name)?;
        let method_id = *self.effective_trait_methods.get(&(imp.id, member_id))?;
        imp.methods
            .values()
            .find(|method| method.id == method_id)
            .cloned()
    }

    fn impl_method_for_trait_member(
        &self,
        imp: &HirImpl,
        member_id: DefId,
    ) -> Option<EffectiveTraitMethod> {
        let method_id = *self.effective_trait_methods.get(&(imp.id, member_id))?;
        if let Some(function) = imp
            .methods
            .values()
            .find(|method| method.id == method_id)
            .cloned()
        {
            return Some(EffectiveTraitMethod {
                function,
                inherited_default: false,
            });
        }

        let trait_id = imp.trait_id?;
        let function = self
            .traits
            .get(&trait_id)?
            .methods
            .values()
            .find(|method| method.id == member_id && method.id == method_id)
            .cloned()?;
        Some(EffectiveTraitMethod {
            function,
            inherited_default: true,
        })
    }

    fn impl_method_self_type_adjustment(
        &self,
        imp: &HirImpl,
        method_name: &str,
        receiver_ty: &Type,
        can_autoref_mut: bool,
    ) -> Option<ReceiverAdjustment> {
        let method = self.impl_method(imp, method_name)?;
        self.method_self_type_adjustment(imp, &method, receiver_ty, can_autoref_mut)
    }

    fn method_self_type_adjustment(
        &self,
        imp: &HirImpl,
        method: &HirFunction,
        receiver_ty: &Type,
        can_autoref_mut: bool,
    ) -> Option<ReceiverAdjustment> {
        if function_has_receiver(method) {
            if let Some(self_param) = method.params.first() {
                let mut subst =
                    impl_receiver_pattern_substitution(imp, receiver_ty, self.impls.values())?;
                let expected_self = Self::receiver_mode_expected_self_type(
                    self_param.ty.substitute_generics(&subst),
                    method.self_receiver,
                );
                return Self::self_type_adjustment(
                    &expected_self,
                    receiver_ty,
                    can_autoref_mut,
                    self.impl_owner_is_reference_like(imp),
                    imp.trait_id.is_some(),
                    &mut subst,
                );
            }
        }
        Some(ReceiverAdjustment::None)
    }

    fn receiver_mode_expected_self_type(
        ty: Type,
        self_receiver: Option<crate::types::ReceiverMode>,
    ) -> Type {
        if matches!(ty, Type::Reference { .. }) {
            return ty;
        }

        match self_receiver {
            Some(crate::types::ReceiverMode::Shared) => Type::Reference {
                mutable: false,
                inner: Box::new(ty),
            },
            Some(crate::types::ReceiverMode::Mut) => Type::Reference {
                mutable: true,
                inner: Box::new(ty),
            },
            Some(crate::types::ReceiverMode::Move) | None => ty,
        }
    }

    fn self_type_adjustment(
        expected_self: &Type,
        receiver_ty: &Type,
        can_autoref_mut: bool,
        allow_reference_receiver: bool,
        allow_unresolved_self_referent: bool,
        subst: &mut HashMap<GenericParamId, Type>,
    ) -> Option<ReceiverAdjustment> {
        if let Type::Reference { mutable, inner } = expected_self {
            if *mutable
                && can_autoref_mut
                && matches!(inner.as_ref(), Type::Reference { .. })
                && Self::type_pattern_matches_committing(inner, receiver_ty, subst)
            {
                return Some(ReceiverAdjustment::AutorefMut);
            }
            if let Type::Reference {
                mutable: receiver_mutable,
                inner: receiver_inner,
            } = receiver_ty
            {
                if !allow_reference_receiver {
                    return None;
                }

                if *mutable {
                    if *receiver_mutable
                        && Self::type_pattern_matches_committing(inner, receiver_inner, subst)
                    {
                        return Some(ReceiverAdjustment::None);
                    }
                    return None;
                }

                if allow_reference_receiver {
                    if Self::type_pattern_matches_committing(inner, receiver_ty, subst) {
                        return Some(ReceiverAdjustment::None);
                    }

                    if let Type::Reference {
                        mutable: false,
                        inner: expected_referent,
                    } = inner.as_ref()
                    {
                        if Self::type_pattern_matches_committing(
                            expected_referent,
                            receiver_inner,
                            subst,
                        ) {
                            return Some(if *receiver_mutable {
                                ReceiverAdjustment::MutToSharedRef
                            } else {
                                ReceiverAdjustment::None
                            });
                        }
                    }
                }

                if Self::type_pattern_matches_committing(inner, receiver_inner, subst) {
                    return Some(if *receiver_mutable {
                        ReceiverAdjustment::MutToSharedRef
                    } else {
                        ReceiverAdjustment::None
                    });
                }

                return None;
            }

            if Self::array_receiver_matches_slice_self(inner, receiver_ty)
                || Self::self_referent_matches_receiver(
                    inner,
                    receiver_ty,
                    allow_unresolved_self_referent,
                    subst,
                )
            {
                if *mutable {
                    return can_autoref_mut.then_some(ReceiverAdjustment::AutorefMut);
                }
                return Some(ReceiverAdjustment::AutorefShared);
            }

            if allow_reference_receiver && !*mutable {
                if let Type::Reference {
                    mutable: false,
                    inner: expected_referent,
                } = inner.as_ref()
                {
                    if Self::self_referent_matches_receiver(
                        expected_referent,
                        receiver_ty,
                        allow_unresolved_self_referent,
                        subst,
                    ) {
                        return Some(ReceiverAdjustment::AutorefShared);
                    }
                }
            }

            return None;
        }

        if matches!(expected_self, Type::Generic(_) | Type::TypeVar(_))
            || Self::array_receiver_matches_slice_self(expected_self, receiver_ty)
            || Self::type_pattern_matches_committing(expected_self, receiver_ty, subst)
        {
            return Some(ReceiverAdjustment::None);
        }

        None
    }

    fn self_referent_matches_receiver(
        expected_referent: &Type,
        receiver_ty: &Type,
        allow_unresolved: bool,
        subst: &mut HashMap<GenericParamId, Type>,
    ) -> bool {
        allow_unresolved && matches!(expected_referent, Type::TypeVar(_))
            || Self::type_pattern_matches_committing(expected_referent, receiver_ty, subst)
    }

    fn impl_owner_is_reference_like(&self, imp: &HirImpl) -> bool {
        matches!(
            &imp.receiver_pattern,
            HirImplReceiverPattern::Exact(Type::Reference { .. })
        )
    }

    fn type_pattern_matches_committing(
        pattern: &Type,
        actual: &Type,
        subst: &mut HashMap<GenericParamId, Type>,
    ) -> bool {
        let mut trial = subst.clone();
        if type_pattern_matches(pattern, actual, &mut trial) {
            *subst = trial;
            true
        } else {
            false
        }
    }

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
                Some((candidate.target.trait_id()?, candidate.target.method_id()?))
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
                    .map(|(_, method_id)| method_id)
                    .collect::<Vec<_>>();
                candidate_ids.sort();
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

    fn impl_receiver_substitution(
        &self,
        imp: &HirImpl,
        method_name: &str,
        receiver_ty: &Type,
    ) -> Option<HashMap<GenericParamId, Type>> {
        if self.impl_method(imp, method_name).is_none() {
            return None;
        }
        impl_receiver_pattern_substitution(imp, receiver_ty, self.impls.values())
    }

    fn impl_matches_trait_ref(
        &self,
        imp: &HirImpl,
        receiver_ty: &Type,
        trait_id: DefId,
        trait_args: Option<&[Type]>,
    ) -> bool {
        self.impl_matches_trait_ref_with_bound_filter(
            imp,
            receiver_ty,
            trait_id,
            trait_args,
            |_| true,
        )
    }

    fn impl_matches_trait_ref_strict(
        &self,
        imp: &HirImpl,
        receiver_ty: &Type,
        trait_id: DefId,
        trait_args: Option<&[Type]>,
    ) -> bool {
        // Proving a trait obligation is not method lookup: receiver autoref and
        // autoderef must not turn `&T` into an implementation for `&mut T`.
        if let HirImplReceiverPattern::Exact(pattern) = &imp.receiver_pattern {
            if !type_pattern_matches(pattern, receiver_ty, &mut HashMap::new()) {
                return false;
            }
        }
        self.impl_matches_trait_ref_with_bound_filter(
            imp,
            receiver_ty,
            trait_id,
            trait_args,
            |obligations| obligations.is_empty(),
        )
    }

    fn impl_matches_trait_ref_with_bound_filter<F>(
        &self,
        imp: &HirImpl,
        receiver_ty: &Type,
        trait_id: DefId,
        trait_args: Option<&[Type]>,
        mut bound_filter: F,
    ) -> bool
    where
        F: FnMut(&[(Type, TraitBound)]) -> bool,
    {
        if imp.trait_id != Some(trait_id)
            || trait_args.is_some_and(|trait_args| imp.trait_arg_types.len() != trait_args.len())
        {
            return false;
        }

        let Some(mut subst) =
            impl_receiver_pattern_substitution(imp, receiver_ty, self.impls.values())
        else {
            return false;
        };

        if let Some(trait_args) = trait_args {
            for (expected, actual) in imp.trait_arg_types.iter().zip(trait_args.iter()) {
                if !type_pattern_matches(expected, actual, &mut subst) {
                    return false;
                }
            }
        }

        let Some(obligations) = self.impl_bound_obligations(imp, &subst) else {
            return false;
        };
        bound_filter(&obligations)
    }

    fn impl_trait_ref_substitution(
        &self,
        imp: &HirImpl,
        receiver_ty: &Type,
        trait_id: DefId,
        trait_args: Option<&[Type]>,
    ) -> Option<HashMap<GenericParamId, Type>> {
        if imp.trait_id != Some(trait_id)
            || trait_args.is_some_and(|trait_args| imp.trait_arg_types.len() != trait_args.len())
        {
            return None;
        }

        let mut subst = impl_receiver_pattern_substitution(imp, receiver_ty, self.impls.values())?;

        if let Some(trait_args) = trait_args {
            for (expected, actual) in imp.trait_arg_types.iter().zip(trait_args.iter()) {
                if !type_pattern_matches(expected, actual, &mut subst) {
                    return None;
                }
            }
        }

        self.impl_bound_obligations(imp, &subst)?;
        Some(subst)
    }

    fn impl_bounds_satisfied(&self, imp: &HirImpl, subst: &HashMap<GenericParamId, Type>) -> bool {
        self.impl_bound_obligations(imp, subst).is_some()
    }

    fn impl_bound_obligations(
        &self,
        imp: &HirImpl,
        subst: &HashMap<GenericParamId, Type>,
    ) -> Option<Vec<(Type, TraitBound)>> {
        if imp.bounds.is_empty() && imp.bounds.predicates.is_empty() {
            return Some(Vec::new());
        }

        let mut obligations = Vec::new();
        if !imp.bounds.predicates.is_empty() {
            for predicate in &imp.bounds.predicates {
                let crate::types::Predicate::Trait {
                    subject,
                    trait_id,
                    args,
                } = predicate.substitute_generics(subst);
                let bound = TraitBound {
                    trait_id,
                    type_args: args,
                };
                if self.assumed_trait_bound_satisfied(&subject, &bound) {
                    continue;
                }
                if Self::type_contains_type_var(&subject)
                    || bound.type_args.iter().any(Self::type_contains_type_var)
                {
                    obligations.push((subject, bound));
                    continue;
                }
                if !self.trait_bound_satisfied(&subject, &bound, imp.id) {
                    return None;
                }
            }
            return Some(obligations);
        }

        for (param, bounds) in &imp.bounds {
            let bounded_ty = Type::Generic(*param).substitute_generics(subst);
            for bound in bounds {
                let bound = TraitBound {
                    trait_id: bound.trait_id,
                    type_args: bound
                        .type_args
                        .iter()
                        .map(|arg| arg.substitute_generics(subst))
                        .collect(),
                };
                if self.assumed_trait_bound_satisfied(&bounded_ty, &bound) {
                    continue;
                }
                if Self::type_contains_type_var(&bounded_ty)
                    || bound.type_args.iter().any(Self::type_contains_type_var)
                {
                    obligations.push((bounded_ty.clone(), bound));
                    continue;
                }
                if !self.trait_bound_satisfied(&bounded_ty, &bound, imp.id) {
                    return None;
                }
            }
        }

        Some(obligations)
    }

    fn type_contains_type_var(ty: &Type) -> bool {
        crate::type_services::visit::type_any(ty, |nested| {
            matches!(nested, Type::TypeVar(_) | Type::Generic(_))
        })
    }

    pub(crate) fn trait_bound_satisfied(
        &self,
        ty: &Type,
        bound: &TraitBound,
        excluded_impl: DefId,
    ) -> bool {
        if self.assumed_trait_bound_satisfied(ty, bound) {
            return true;
        }

        if self.is_builtin_sized_trait(bound.trait_id) {
            return self.type_is_sized(ty);
        }

        self.impl_ids_in_order()
            .into_iter()
            .filter_map(|id| self.impls.get(&id))
            .any(|imp| {
                imp.id != excluded_impl
                    && self.impl_matches_trait_ref_strict(
                        imp,
                        ty,
                        bound.trait_id,
                        Some(&bound.type_args),
                    )
            })
    }

    pub(crate) fn trait_bounds_with_supertraits(
        &self,
        subject: &Type,
        bounds: &[TraitBound],
    ) -> Vec<TraitBound> {
        fn expand(
            service: &SelectionService<'_>,
            subject: &Type,
            bound: TraitBound,
            visiting: &mut std::collections::HashSet<crate::types::Predicate>,
            output: &mut Vec<TraitBound>,
        ) {
            let key = crate::types::Predicate::Trait {
                subject: subject.clone(),
                trait_id: bound.trait_id,
                args: bound.type_args.clone(),
            };
            if !visiting.insert(key.clone()) {
                return;
            }
            if !output.contains(&bound) {
                output.push(bound.clone());
            }

            if let Some(trait_def) = service.traits.get(&bound.trait_id) {
                let mut subst = crate::selection::generic_substitution_for_owner(
                    &trait_def.generic_params,
                    &bound.type_args,
                );
                let target_id =
                    trait_def
                        .target
                        .as_ref()
                        .map(|target| target.id)
                        .unwrap_or(GenericParamId {
                            owner: trait_def.id,
                            index: trait_def.generic_params.len() as u32,
                        });
                subst.insert(target_id, subject.clone());
                for predicate in &trait_def.predicates {
                    let crate::types::Predicate::Trait {
                        subject: implied_subject,
                        trait_id,
                        args,
                    } = predicate.substitute_generics(&subst);
                    if implied_subject == *subject {
                        expand(
                            service,
                            subject,
                            TraitBound {
                                trait_id,
                                type_args: args,
                            },
                            visiting,
                            output,
                        );
                    }
                }
            }
            visiting.remove(&key);
        }

        let mut output = Vec::new();
        let mut visiting = std::collections::HashSet::new();
        for bound in bounds {
            expand(self, subject, bound.clone(), &mut visiting, &mut output);
        }
        output.sort_by_key(|bound| (bound.trait_id, format!("{:?}", bound.type_args)));
        output
    }

    fn assumed_trait_bound_satisfied(&self, ty: &Type, bound: &TraitBound) -> bool {
        if let Type::Generic(param) = ty {
            return self.current_impl_bounds.get(param).is_some_and(|bounds| {
                self.trait_bounds_with_supertraits(ty, bounds)
                    .contains(bound)
            });
        }
        false
    }

    fn is_builtin_sized_trait(&self, trait_id: DefId) -> bool {
        self.sized_trait_id == Some(trait_id)
    }

    fn type_is_sized(&self, ty: &Type) -> bool {
        match ty {
            Type::Slice(_) | Type::Str => false,
            Type::Array(inner, _) => self.type_is_sized(inner),
            Type::Tuple(elems) => elems.iter().all(|elem| self.type_is_sized(elem)),
            Type::I8
            | Type::I16
            | Type::I32
            | Type::I64
            | Type::U8
            | Type::U16
            | Type::U32
            | Type::U64
            | Type::F32
            | Type::F64
            | Type::Bool
            | Type::Char
            | Type::Unit
            | Type::Never
            | Type::Struct { .. }
            | Type::Enum { .. }
            | Type::Reference { .. }
            | Type::Pointer(_)
            | Type::Function { .. } => true,
            Type::Generic(param) => self.current_impl_bounds.get(param).is_some_and(|bounds| {
                bounds
                    .iter()
                    .any(|bound| self.is_builtin_sized_trait(bound.trait_id))
            }),
            Type::Projection { .. }
            | Type::TypeVar(_)
            | Type::Constructor { .. }
            | Type::Apply { .. }
            | Type::Lambda { .. }
            | Type::BoundVar { .. }
            | Type::Error => false,
        }
    }

    fn impl_matches_receiver_type(&self, imp: &HirImpl, receiver_ty: &Type) -> bool {
        impl_receiver_pattern_substitution(imp, receiver_ty, self.impls.values()).is_some()
    }

    fn impl_index_args_substitution(
        &self,
        imp: &HirImpl,
        member_id: DefId,
        receiver_ty: &Type,
        trait_args: &[Type],
    ) -> Option<HashMap<GenericParamId, Type>> {
        let mut subst = impl_receiver_pattern_substitution(imp, receiver_ty, self.impls.values())?;
        if self.impl_method_for_trait_member(imp, member_id).is_none()
            || imp.trait_arg_types.len() != trait_args.len()
        {
            return None;
        }

        for (expected, actual) in imp.trait_arg_types.iter().zip(trait_args.iter()) {
            if !type_pattern_matches(expected, actual, &mut subst) {
                return None;
            }
        }

        Some(subst)
    }

    fn instantiate_impl_method(
        &self,
        imp: &HirImpl,
        receiver: &HirExpr,
        receiver_ty: &Type,
        method_name: &str,
        receiver_adjustment: ReceiverAdjustment,
    ) -> Option<SelectedMethod> {
        self.instantiate_impl_method_with_subst(
            imp,
            receiver,
            receiver_ty,
            method_name,
            HashMap::new(),
            receiver_adjustment,
        )
    }

    fn instantiate_impl_method_with_subst(
        &self,
        imp: &HirImpl,
        receiver: &HirExpr,
        receiver_ty: &Type,
        method_name: &str,
        subst: HashMap<GenericParamId, Type>,
        receiver_adjustment: ReceiverAdjustment,
    ) -> Option<SelectedMethod> {
        let method_func = self.impl_method(imp, method_name)?;
        let trait_member_id = match imp.trait_id {
            Some(trait_id) => Some(self.trait_member_id(trait_id, method_name)?),
            None => None,
        };
        let inherited_trait_default =
            !imp.methods.contains_key(method_name) && imp.trait_id.is_some() && {
                let member_id = trait_member_id.expect("trait impl selected above");
                !self
                    .effective_trait_methods
                    .contains_key(&(imp.id, member_id))
            };
        self.instantiate_method_function_with_subst(
            imp,
            receiver,
            receiver_ty,
            method_func,
            trait_member_id,
            inherited_trait_default,
            subst,
            receiver_adjustment,
        )
    }

    fn instantiate_method_function_with_subst(
        &self,
        imp: &HirImpl,
        receiver: &HirExpr,
        receiver_ty: &Type,
        mut method_func: HirFunction,
        trait_member_id: Option<DefId>,
        inherited_trait_default: bool,
        mut subst: HashMap<GenericParamId, Type>,
        receiver_adjustment: ReceiverAdjustment,
    ) -> Option<SelectedMethod> {
        let receiver_subst =
            impl_receiver_pattern_substitution(imp, receiver_ty, self.impls.values())?;
        for (param, ty) in receiver_subst {
            if subst.get(&param).is_some_and(|existing| existing != &ty) {
                return None;
            }
            subst.insert(param, ty);
        }
        if method_func.is_method {
            if let Some(self_param) = method_func.params.first() {
                let expected_self = Self::receiver_mode_expected_self_type(
                    self_param.ty.substitute_generics(&subst),
                    method_func.self_receiver,
                );
                let can_autoref_mut = matches!(receiver_adjustment, ReceiverAdjustment::AutorefMut);
                if Self::self_type_adjustment(
                    &expected_self,
                    receiver_ty,
                    can_autoref_mut,
                    self.impl_owner_is_reference_like(imp),
                    imp.trait_id.is_some(),
                    &mut subst,
                )
                .is_none()
                {
                    return None;
                }
            }
        }

        for param in &mut method_func.params {
            param.ty = param.ty.substitute_generics(&subst);
        }
        if method_func.is_method {
            if let Some(self_param) = method_func.params.first_mut() {
                self_param.ty = receiver_ty.clone();
            }
        }
        let pending_impl_bounds = self.impl_bound_obligations(imp, &subst)?;
        let owner_generic_params: Vec<GenericParamId> =
            imp.type_generics.iter().map(|decl| decl.id).collect();
        let associated_types = imp
            .associated_types
            .iter()
            .map(|assoc| HirAssociatedTypeDef {
                id: assoc.id,
                name: assoc.name.clone(),
                kind: assoc.kind.clone(),
                ty: assoc.ty.substitute_generics(&subst),
            })
            .collect();
        method_func.ret_type = method_func.ret_type.substitute_generics(&subst);
        Self::substitute_function_generic_bounds(&mut method_func, &subst);
        let selected_trait = match imp.trait_id {
            Some(trait_id) => {
                let member_id = trait_member_id?;
                Some(HirSelectedTraitMember {
                    trait_id,
                    member_id,
                    trait_args: imp
                        .trait_arg_types
                        .iter()
                        .map(|arg| arg.substitute_generics(&subst))
                        .collect(),
                })
            }
            None => None,
        };
        let mut target = if inherited_trait_default {
            let selected_trait = selected_trait.clone()?;
            HirMethodCallTarget::trait_method(
                selected_trait.trait_id,
                selected_trait.member_id,
                selected_trait.trait_args,
                HirTraitDispatchKind::TraitBound,
            )
        } else {
            HirMethodCallTarget::impl_method(imp.id, method_func.id, selected_trait)
        };
        let mut owner_substitution = subst
            .iter()
            .filter(|(param, _)| owner_generic_params.contains(param))
            .map(|(&param, ty)| HirTypeBinding {
                param,
                ty: ty.clone(),
            })
            .collect::<Vec<_>>();
        owner_substitution.sort_by_key(|binding| {
            (
                binding.param.owner.crate_id.0,
                binding.param.owner.local.0,
                binding.param.index,
            )
        });
        target.owner_substitution = owner_substitution;
        let origin = match imp.trait_id {
            Some(trait_id) => SelectedOrigin::TraitImpl {
                impl_id: imp.id,
                trait_id,
            },
            None => SelectedOrigin::InherentImpl { impl_id: imp.id },
        };
        let param_start = if method_func.is_method { 1 } else { 0 };

        Some(SelectedMethod {
            receiver: receiver.clone(),
            substituted_params: method_func.params[param_start..].to_vec(),
            return_type: method_func.ret_type.clone(),
            function: Some(method_func),
            impl_def: Some(imp.clone()),
            target,
            origin,
            receiver_adjustment,
            pending_impl_bounds,
            associated_types,
            owner_substitution: subst,
            owner_generic_params,
        })
    }

    fn instantiate_constructor_member_with_subst(
        &self,
        imp: &HirImpl,
        mut method_func: HirFunction,
        trait_member_id: DefId,
        subst: HashMap<GenericParamId, Type>,
    ) -> Option<SelectedConstructorMember> {
        if function_has_receiver(&method_func) {
            return None;
        }
        for param in &mut method_func.params {
            param.ty = param.ty.substitute_generics(&subst);
        }
        method_func.ret_type = method_func.ret_type.substitute_generics(&subst);
        Self::substitute_function_generic_bounds(&mut method_func, &subst);
        let pending_impl_bounds = self.impl_bound_obligations(imp, &subst)?;
        let owner_generic_params = imp
            .type_generics
            .iter()
            .map(|param| param.id)
            .collect::<Vec<_>>();
        let selected_trait = HirSelectedTraitMember {
            trait_id: imp.trait_id?,
            member_id: trait_member_id,
            trait_args: imp
                .trait_arg_types
                .iter()
                .map(|arg| arg.substitute_generics(&subst))
                .collect(),
        };
        let mut target =
            HirMethodCallTarget::impl_method(imp.id, method_func.id, Some(selected_trait));
        let mut owner_substitution = subst
            .iter()
            .filter(|(param, _)| owner_generic_params.contains(param))
            .map(|(&param, ty)| HirTypeBinding {
                param,
                ty: ty.clone(),
            })
            .collect::<Vec<_>>();
        owner_substitution.sort_by_key(|binding| {
            (
                binding.param.owner.crate_id.0,
                binding.param.owner.local.0,
                binding.param.index,
            )
        });
        target.owner_substitution = owner_substitution;

        Some(SelectedConstructorMember {
            substituted_params: method_func.params.clone(),
            return_type: method_func.ret_type.clone(),
            function: method_func,
            impl_def: imp.clone(),
            target,
            pending_impl_bounds,
            owner_substitution: subst,
            owner_generic_params,
        })
    }

    fn substitute_function_generic_bounds(
        function: &mut HirFunction,
        subst: &HashMap<GenericParamId, Type>,
    ) {
        for bounds in function.generic_bounds.values_mut() {
            for bound in bounds {
                bound.type_args = bound
                    .type_args
                    .iter()
                    .map(|arg| arg.substitute_generics(subst))
                    .collect();
            }
        }
        function.generic_bounds.predicates = function
            .generic_bounds
            .predicates
            .iter()
            .map(|predicate| predicate.substitute_generics(subst))
            .collect();
    }

    fn selected_current_trait_method(
        receiver: HirExpr,
        trait_id: DefId,
        trait_args: Vec<Type>,
        method_func: HirFunction,
    ) -> SelectedMethod {
        let method_id = method_func.id;
        let param_start = if method_func.is_method { 1 } else { 0 };
        let target = HirMethodCallTarget::trait_method(
            trait_id,
            method_id,
            trait_args,
            HirTraitDispatchKind::CurrentTrait,
        );

        SelectedMethod {
            receiver,
            substituted_params: method_func.params[param_start..].to_vec(),
            return_type: method_func.ret_type.clone(),
            function: Some(method_func),
            impl_def: None,
            target,
            origin: SelectedOrigin::CurrentTrait { trait_id },
            receiver_adjustment: ReceiverAdjustment::None,
            pending_impl_bounds: Vec::new(),
            associated_types: Vec::new(),
            owner_substitution: HashMap::new(),
            owner_generic_params: Vec::new(),
        }
    }

    fn array_receiver_matches_slice_self(expected_self: &Type, receiver_ty: &Type) -> bool {
        match (expected_self, receiver_ty) {
            (Type::Slice(expected_elem), Type::Array(actual_elem, _)) => {
                type_pattern_matches(expected_elem, actual_elem, &mut HashMap::new())
            }
            _ => false,
        }
    }

    pub fn trait_by_id(&self, id: DefId) -> Option<&HirTrait> {
        self.traits.get(&id)
    }

    pub fn trait_def_for_member(&self, trait_id: DefId, method_name: &str) -> Option<&HirTrait> {
        let trait_def = self.traits.get(&trait_id)?;
        (Self::trait_member_id_score(trait_def, method_name) > 0).then_some(trait_def)
    }

    pub fn trait_member_id(&self, trait_id: DefId, method_name: &str) -> Option<DefId> {
        let trait_def = self.trait_def_for_member(trait_id, method_name)?;
        trait_def
            .methods
            .get(method_name)
            .map(|method| method.id)
            .or_else(|| trait_def.signatures.get(method_name).map(|sig| sig.id))
    }

    pub(super) fn trait_member_name_by_id(
        &self,
        trait_id: DefId,
        member_id: DefId,
    ) -> Option<&str> {
        let trait_def = self.traits.get(&trait_id)?;
        trait_def
            .methods
            .iter()
            .find_map(|(name, method)| (method.id == member_id).then_some(name.as_str()))
            .or_else(|| {
                trait_def.signatures.iter().find_map(|(name, signature)| {
                    (signature.id == member_id).then_some(name.as_str())
                })
            })
    }

    fn trait_member_id_score(trait_def: &HirTrait, method_name: &str) -> usize {
        let method_score = trait_def
            .methods
            .get(method_name)
            .map(|method| Self::def_id_score(method.id))
            .unwrap_or(0);
        let signature_score = trait_def
            .signatures
            .get(method_name)
            .map(|sig| Self::def_id_score(sig.id))
            .unwrap_or(0);

        method_score.max(signature_score)
    }

    fn def_id_score(id: DefId) -> usize {
        if id.crate_id.0 == u32::MAX {
            1
        } else {
            2
        }
    }
}

fn inferred_rigid_receiver_type(pattern: &HirImplReceiverPattern) -> Option<Type> {
    let ty = match pattern {
        HirImplReceiverPattern::Exact(ty) | HirImplReceiverPattern::Constructor(ty) => ty,
        HirImplReceiverPattern::SliceFamily { element } => {
            return Some(Type::Reference {
                mutable: false,
                inner: Box::new(Type::Slice(Box::new(element.clone()))),
            });
        }
    };

    if !matches!(
        ty,
        Type::Struct { .. }
            | Type::Enum { .. }
            | Type::Reference { .. }
            | Type::Slice(_)
            | Type::Array(_, _)
            | Type::Pointer(_)
            | Type::Tuple(_)
    ) {
        return None;
    }

    (!crate::type_services::visit::type_any(ty, |nested| {
        matches!(
            nested,
            Type::TypeVar(_)
                | Type::Generic(_)
                | Type::Projection { .. }
                | Type::Apply { .. }
                | Type::Constructor { .. }
                | Type::Lambda { .. }
                | Type::BoundVar { .. }
                | Type::Error
        )
    }))
    .then(|| ty.clone())
}

#[cfg(test)]
#[allow(unused_variables)]
mod tests {
    use super::*;
    use crate::collect::resolver::ResolverTables;
    use crate::hir::{
        HirBlock, HirExpr, HirExprKind, HirFunction, HirImpl, HirImplOwner, HirParam,
    };
    use crate::ids::{CrateId, DefId, LocalDefId, TypeVarId};
    use crate::lexer::Span;
    use crate::types::ReceiverMode;
    use crate::types::{GenericParamDecl, GenericParamId, Type};

    fn def_id(index: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(index))
    }

    fn receiver_candidate(expr: HirExpr) -> ReceiverCandidate {
        ReceiverCandidate {
            expr,
            adjustment: ReceiverAdjustment::None,
            can_autoref_mut: false,
        }
    }

    fn method(id: DefId, owner: DefId) -> HirFunction {
        HirFunction {
            id,
            name: "value".to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "self".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Struct {
                    id: owner,
                    args: Vec::new(),
                },
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::I64,
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::I64,
            },
            is_curried: false,
            is_method: true,
            self_receiver: Some(ReceiverMode::Shared),
            is_unsafe: false,
        }
    }

    fn operator_method(id: DefId, owner: DefId, ret_type: Type) -> HirFunction {
        HirFunction {
            id,
            name: "+".to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: vec![
                HirParam {
                    name: "self".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::Struct {
                        id: owner,
                        args: Vec::new(),
                    },
                    mutable: false,
                    is_ref: false,
                },
                HirParam {
                    name: "rhs".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::Struct {
                        id: owner,
                        args: Vec::new(),
                    },
                    mutable: false,
                    is_ref: false,
                },
            ],
            ret_type: ret_type.clone(),
            body: HirBlock {
                stmts: Vec::new(),
                ty: ret_type,
            },
            is_curried: false,
            is_method: true,
            self_receiver: Some(ReceiverMode::Shared),
            is_unsafe: false,
        }
    }

    fn register_trait_member(
        traits: &mut HashMap<DefId, crate::hir::HirTrait>,
        imp: &HirImpl,
        method_name: &str,
    ) -> DefId {
        let trait_id = imp.trait_id.expect("test trait impl");
        let trait_name = imp.trait_name.clone().expect("test trait name");
        let method = imp.methods.get(method_name).expect("test impl method");
        let member_id = DefId::new(
            method.id.crate_id,
            LocalDefId(
                method
                    .id
                    .local
                    .0
                    .checked_add(10_000)
                    .expect("test member ID overflow"),
            ),
        );
        let trait_def = traits
            .entry(trait_id)
            .or_insert_with(|| crate::hir::HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: trait_name,
                generic_params: imp.trait_generics.clone(),
                associated_types: imp
                    .associated_types
                    .iter()
                    .map(|assoc| crate::hir::HirAssociatedTypeDecl {
                        id: assoc.id,
                        name: assoc.name.clone(),
                        kind: assoc.kind.clone(),
                    })
                    .collect(),
                methods: HashMap::new(),
                signatures: HashMap::new(),
            });
        assert_eq!(trait_def.id, trait_id);
        trait_def.signatures.insert(
            method_name.to_string(),
            crate::hir::HirFunctionSig {
                id: member_id,
                name: method_name.to_string(),
                generic_params: method.generic_params.clone(),
                params: method.params.iter().map(|param| param.ty.clone()).collect(),
                ret: method.ret_type.clone(),
                generic_bounds: method.generic_bounds.clone(),
                self_receiver: method.self_receiver,
                is_unsafe: method.is_unsafe,
            },
        );
        member_id
    }

    fn renamed_trait_member_fixture(
        trait_id: DefId,
        member_id: DefId,
        source_name: &str,
        impl_id: DefId,
        impl_method_id: DefId,
    ) -> (HashMap<DefId, HirTrait>, HashMap<DefId, HirImpl>, HirExpr) {
        let owner_id = def_id(9);
        let receiver_ty = Type::Struct {
            id: owner_id,
            args: Vec::new(),
        };
        let mut trait_method = method(member_id, owner_id);
        trait_method.name = source_name.to_string();
        let trait_def = HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "RenamedProtocol".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([(source_name.to_string(), trait_method)]),
            signatures: HashMap::new(),
        };

        let mut impl_method = method(impl_method_id, owner_id);
        impl_method.name = source_name.to_string();
        let imp = HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Owner".to_string()),
            type_name: "Owner".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(receiver_ty.clone()),
            trait_name: Some("RenamedProtocol".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([(source_name.to_string(), impl_method)]),
        };
        let receiver = HirExpr {
            kind: HirExprKind::Var("owner".to_string()),
            ty: receiver_ty,
            span: Span::test(),
        };

        (
            HashMap::from([(trait_id, trait_def)]),
            HashMap::from([(impl_id, imp)]),
            receiver,
        )
    }

    fn effective_trait_methods_for(
        traits: &HashMap<DefId, HirTrait>,
        impls: &HashMap<DefId, HirImpl>,
    ) -> HashMap<(DefId, DefId), DefId> {
        impls
            .values()
            .filter_map(|imp| {
                let trait_id = imp.trait_id?;
                let trait_def = traits.get(&trait_id)?;
                Some(
                    imp.methods
                        .iter()
                        .filter_map(|(name, method)| {
                            let member_id = trait_def
                                .methods
                                .get(name)
                                .map(|member| member.id)
                                .or_else(|| {
                                    trait_def.signatures.get(name).map(|signature| signature.id)
                                })?;
                            Some(((imp.id, member_id), method.id))
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .flatten()
            .collect()
    }

    #[test]
    fn selection_service_selects_required_trait_member_by_exact_id() {
        let trait_id = def_id(10);
        let member_id = def_id(11);
        let impl_id = def_id(12);
        let impl_method_id = def_id(13);
        let (traits, impls, receiver) =
            renamed_trait_member_fixture(trait_id, member_id, "split", impl_id, impl_method_id);
        let effective = HashMap::from([((impl_id, member_id), impl_method_id)]);
        let bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &bounds)
            .with_effective_trait_methods(&effective);

        let selected = service
            .select_required_trait_member(&receiver, &receiver.ty, trait_id, member_id)
            .expect("expected exact trait-member selection");

        match &selected.target.target {
            crate::hir::HirSelectedMethodTarget::ImplMethod {
                impl_id: selected_impl_id,
                method_id,
                selected_trait: Some(selected_trait),
            } => {
                assert_eq!(*selected_impl_id, impl_id);
                assert_eq!(*method_id, impl_method_id);
                assert_eq!(selected_trait.trait_id, trait_id);
                assert_eq!(selected_trait.member_id, member_id);
            }
            target => panic!("expected trait impl target, found {target:?}"),
        }
        assert_eq!(
            selected.function.as_ref().map(|function| function.id),
            Some(impl_method_id)
        );
    }

    #[test]
    fn selection_service_rejects_trait_method_without_effective_mapping() {
        let trait_id = def_id(14);
        let member_id = def_id(15);
        let impl_id = def_id(16);
        let impl_method_id = def_id(17);
        let (traits, impls, receiver) =
            renamed_trait_member_fixture(trait_id, member_id, "split", impl_id, impl_method_id);
        let bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &bounds);

        let result =
            service.select_required_trait_method(&receiver, &receiver.ty, trait_id, "split");

        assert!(matches!(
            result,
            Err(SelectionDiagnostic::NoImplementation { .. })
        ));
    }

    #[test]
    fn selection_service_selects_static_trait_member_by_exact_id_and_trait_args() {
        let trait_id = def_id(20);
        let member_id = def_id(21);
        let impl_id = def_id(22);
        let impl_method_id = def_id(23);
        let (mut traits, mut impls, mut owner) =
            renamed_trait_member_fixture(trait_id, member_id, "recover", impl_id, impl_method_id);
        traits
            .get_mut(&trait_id)
            .expect("fixture trait")
            .methods
            .get_mut("recover")
            .expect("fixture trait member")
            .is_method = false;
        traits
            .get_mut(&trait_id)
            .expect("fixture trait")
            .methods
            .get_mut("recover")
            .expect("fixture trait member")
            .self_receiver = None;
        let imp = impls.get_mut(&impl_id).expect("fixture impl");
        imp.trait_arg_types = vec![Type::I64];
        let impl_method = imp.methods.get_mut("recover").expect("fixture impl method");
        impl_method.is_method = false;
        impl_method.self_receiver = None;
        owner.kind = HirExprKind::Unit;
        let effective = HashMap::from([((impl_id, member_id), impl_method_id)]);
        let bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &bounds)
            .with_effective_trait_methods(&effective);

        let selected = service
            .select_static_trait_member(&owner, &owner.ty, trait_id, &[Type::I64], member_id)
            .expect("expected exact static trait-member selection");

        match &selected.target.target {
            crate::hir::HirSelectedMethodTarget::ImplMethod {
                impl_id: selected_impl_id,
                method_id,
                selected_trait: Some(selected_trait),
            } => {
                assert_eq!(*selected_impl_id, impl_id);
                assert_eq!(*method_id, impl_method_id);
                assert_eq!(selected_trait.member_id, member_id);
                assert_eq!(selected_trait.trait_args, vec![Type::I64]);
            }
            target => panic!("expected trait impl target, found {target:?}"),
        }
        assert_eq!(
            selected.function.as_ref().map(|function| function.id),
            Some(impl_method_id)
        );
    }

    #[test]
    fn selection_service_rejects_static_member_mapped_to_implicit_receiver_method() {
        let trait_id = def_id(24);
        let member_id = def_id(25);
        let impl_id = def_id(26);
        let impl_method_id = def_id(27);
        let (mut traits, mut impls, mut owner) =
            renamed_trait_member_fixture(trait_id, member_id, "recover", impl_id, impl_method_id);
        let trait_member = traits
            .get_mut(&trait_id)
            .expect("fixture trait")
            .methods
            .get_mut("recover")
            .expect("fixture trait member");
        trait_member.is_method = false;
        trait_member.self_receiver = None;
        let impl_method = impls
            .get_mut(&impl_id)
            .expect("fixture impl")
            .methods
            .get_mut("recover")
            .expect("fixture impl method");
        impl_method.is_method = true;
        impl_method.self_receiver = None;
        owner.kind = HirExprKind::Unit;
        let effective = HashMap::from([((impl_id, member_id), impl_method_id)]);
        let bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &bounds)
            .with_effective_trait_methods(&effective);

        assert!(matches!(
            service.select_static_trait_member(&owner, &owner.ty, trait_id, &[], member_id),
            Err(SelectionDiagnostic::NoImplementation { operation, .. }) if operation == "recover"
        ));
    }

    #[test]
    fn selection_service_does_not_fallback_to_same_named_impl_method_without_exact_mapping() {
        let trait_id = def_id(30);
        let member_id = def_id(31);
        let impl_id = def_id(32);
        let impl_method_id = def_id(33);
        let (traits, impls, receiver) =
            renamed_trait_member_fixture(trait_id, member_id, "split", impl_id, impl_method_id);
        let effective = HashMap::new();
        let bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &bounds)
            .with_effective_trait_methods(&effective);

        assert!(matches!(
            service.select_required_trait_member(&receiver, &receiver.ty, trait_id, member_id),
            Err(SelectionDiagnostic::NoImplementation { .. })
        ));
    }

    #[test]
    fn selection_service_allows_exact_trait_default_through_effective_mapping() {
        let trait_id = def_id(40);
        let member_id = def_id(41);
        let impl_id = def_id(42);
        let impl_method_id = def_id(43);
        let (traits, mut impls, receiver) =
            renamed_trait_member_fixture(trait_id, member_id, "split", impl_id, impl_method_id);
        impls
            .get_mut(&impl_id)
            .expect("fixture impl")
            .methods
            .clear();
        let effective = HashMap::from([((impl_id, member_id), member_id)]);
        let bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &bounds)
            .with_effective_trait_methods(&effective);

        let selected = service
            .select_required_trait_member(&receiver, &receiver.ty, trait_id, member_id)
            .expect("expected exact default selection");

        assert_eq!(selected.target.method_id(), Some(member_id));
        assert_eq!(
            selected.function.as_ref().map(|function| function.id),
            Some(member_id)
        );
    }

    #[test]
    fn selection_service_rejects_same_spelled_member_from_another_trait() {
        let trait_id = def_id(50);
        let member_id = def_id(51);
        let impl_id = def_id(52);
        let impl_method_id = def_id(53);
        let (mut traits, impls, receiver) =
            renamed_trait_member_fixture(trait_id, member_id, "split", impl_id, impl_method_id);
        let other_trait_id = def_id(54);
        let other_member_id = def_id(55);
        let mut other_member = method(other_member_id, def_id(9));
        other_member.name = "split".to_string();
        traits.insert(
            other_trait_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: other_trait_id,
                name: "OtherProtocol".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::from([("split".to_string(), other_member)]),
                signatures: HashMap::new(),
            },
        );
        let effective = HashMap::from([((impl_id, other_member_id), impl_method_id)]);
        let bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &bounds)
            .with_effective_trait_methods(&effective);

        assert!(matches!(
            service.select_required_trait_member(&receiver, &receiver.ty, trait_id, member_id),
            Err(SelectionDiagnostic::NoImplementation { .. })
        ));
    }

    #[test]
    fn selection_service_reports_invalid_or_foreign_trait_member_ids_without_panicking() {
        let trait_id = def_id(60);
        let member_id = def_id(61);
        let impl_id = def_id(62);
        let impl_method_id = def_id(63);
        let (mut traits, impls, receiver) =
            renamed_trait_member_fixture(trait_id, member_id, "split", impl_id, impl_method_id);
        let foreign_trait_id = def_id(64);
        let foreign_member_id = def_id(65);
        let mut foreign_member = method(foreign_member_id, def_id(9));
        foreign_member.name = "split".to_string();
        traits.insert(
            foreign_trait_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: foreign_trait_id,
                name: "ForeignProtocol".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::from([("split".to_string(), foreign_member)]),
                signatures: HashMap::new(),
            },
        );
        let effective = HashMap::from([((impl_id, member_id), impl_method_id)]);
        let bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &bounds)
            .with_effective_trait_methods(&effective);

        for invalid_member_id in [def_id(66), foreign_member_id] {
            let error = match service.select_required_trait_member(
                &receiver,
                &receiver.ty,
                trait_id,
                invalid_member_id,
            ) {
                Ok(_) => panic!("invalid trait member ID selected a method"),
                Err(error) => error,
            };
            let message = error.message();
            assert_eq!(
                message,
                "Trait '<unknown trait>' does not declare the selected member"
            );
            assert!(!message.contains("DefId"));
        }
    }

    #[test]
    fn trait_lookup_does_not_select_payload_under_a_different_map_key() {
        let trait_id = def_id(1);
        let wrong_key = def_id(2);
        let traits = HashMap::from([(
            wrong_key,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Reader".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        )]);
        let impls: HashMap<DefId, HirImpl> = HashMap::new();
        let resolver = ResolverTables::default();
        let bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &bounds);

        assert!(service.trait_by_id(trait_id).is_none());
    }

    #[test]
    fn builtin_sized_identity_uses_marked_trait_id() {
        let marked_sized_id = def_id(26);
        let unmarked_id = def_id(27);
        let traits = HashMap::new();
        let impls = HashMap::new();
        let bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, Some(marked_sized_id), None, &bounds);

        assert!(service.is_builtin_sized_trait(marked_sized_id));
        assert!(!service.is_builtin_sized_trait(unmarked_id));
    }

    #[test]
    fn impl_method_does_not_select_trait_default_under_a_different_map_key() {
        let owner = def_id(3);
        let trait_id = def_id(4);
        let wrong_key = def_id(5);
        let default_method = method(def_id(6), owner);
        let traits = HashMap::from([(
            wrong_key,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Reader".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::from([("value".to_string(), default_method)]),
                signatures: HashMap::new(),
            },
        )]);
        let imp = HirImpl {
            id: def_id(7),
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: owner,
                args: Vec::new(),
            }),
            trait_name: Some("Reader".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::new(),
        };
        let impls = HashMap::from([(imp.id, imp.clone())]);
        let resolver = ResolverTables::default();
        let bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &bounds);

        assert!(service.impl_method(&imp, "value").is_none());
    }

    #[test]
    fn trait_impl_method_absent_from_trait_is_not_selected() {
        let owner = def_id(5);
        let trait_id = def_id(6);
        let impl_id = def_id(7);
        let method_id = def_id(8);
        let traits = HashMap::from([(
            trait_id,
            crate::hir::HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Marker".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        )]);
        let impls: HashMap<DefId, HirImpl> = vec![HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: owner,
                args: Vec::new(),
            }),
            trait_name: Some("Marker".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), method(method_id, owner))]),
        }]
        .into_iter()
        .map(|imp| (imp.id, imp))
        .collect();
        let mut resolver = ResolverTables::default();
        resolver.item_names_by_id.insert(owner, "Box".to_string());
        let current_impl_bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
        let receiver = receiver_candidate(HirExpr {
            ty: Type::Struct {
                id: owner,
                args: Vec::new(),
            },
            kind: HirExprKind::Var("box".to_string()),
            span: Span::test(),
        });

        assert!(matches!(
            service.select_concrete_method(&[receiver], "value", Type::clone),
            Err(SelectionDiagnostic::NoImplementation { .. })
        ));
        let receiver = HirExpr {
            ty: Type::Struct {
                id: owner,
                args: Vec::new(),
            },
            kind: HirExprKind::Var("box".to_string()),
            span: Span::test(),
        };
        assert!(matches!(
            service.select_required_trait_method(&receiver, &receiver.ty, trait_id, "value"),
            Err(SelectionDiagnostic::TraitMemberMissing {
                trait_id: missing_trait_id,
                method_name,
            }) if missing_trait_id == trait_id && method_name == "value"
        ));
    }

    fn mut_method(id: DefId, owner: DefId) -> HirFunction {
        let mut method = method(id, owner);
        method.name = "inc".to_string();
        method.self_receiver = Some(ReceiverMode::Mut);
        method.params[0].ty = Type::Reference {
            mutable: true,
            inner: Box::new(Type::Struct {
                id: owner,
                args: Vec::new(),
            }),
        };
        method.params[0].mutable = true;
        method.ret_type = Type::Unit;
        method.body.ty = Type::Unit;
        method
    }

    #[test]
    fn selection_rejects_mut_receiver_without_mutable_lvalue() {
        let owner = def_id(10);
        let impl_id = def_id(20);
        let method_id = def_id(21);
        let imp = HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Counter".to_string()),
            type_name: "Counter".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: owner,
                args: Vec::new(),
            }),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("inc".to_string(), mut_method(method_id, owner))]),
        };
        let traits: HashMap<DefId, HirTrait> = HashMap::new();
        let impls = HashMap::from([(imp.id, imp)]);
        let mut resolver = ResolverTables::default();
        resolver
            .item_names_by_id
            .insert(owner, "Counter".to_string());
        let current_impl_bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
        let receiver_ty = Type::Struct {
            id: owner,
            args: Vec::new(),
        };
        let receiver = HirExpr {
            kind: HirExprKind::Var("counter".to_string()),
            ty: receiver_ty.clone(),
            span: Span::test(),
        };
        let shared_receiver = HirExpr {
            kind: HirExprKind::Var("counter_ref".to_string()),
            ty: Type::Reference {
                mutable: false,
                inner: Box::new(receiver_ty),
            },
            span: Span::test(),
        };

        assert!(service
            .select_concrete_method(&[receiver_candidate(receiver.clone())], "inc", |ty| ty
                .clone())
            .is_err());
        assert!(service
            .select_concrete_method(
                &[ReceiverCandidate {
                    expr: shared_receiver,
                    adjustment: ReceiverAdjustment::None,
                    can_autoref_mut: false,
                }],
                "inc",
                |ty| ty.clone(),
            )
            .is_err());
        let selected = service
            .select_concrete_method(
                &[ReceiverCandidate {
                    expr: receiver,
                    adjustment: ReceiverAdjustment::None,
                    can_autoref_mut: true,
                }],
                "inc",
                |ty| ty.clone(),
            )
            .expect("mutable lvalue should match mut receiver");

        assert_eq!(selected.receiver_adjustment, ReceiverAdjustment::AutorefMut);
    }

    #[test]
    fn selection_uses_mut_to_shared_ref_for_reference_owned_impl() {
        let trait_id = def_id(30);
        let impl_id = def_id(40);
        let method_id = def_id(41);
        let method = HirFunction {
            id: method_id,
            name: "show".to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "self".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::Str),
                },
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::Str,
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::Str,
            },
            is_curried: false,
            is_method: true,
            self_receiver: Some(ReceiverMode::Shared),
            is_unsafe: false,
        };
        let imp = HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("&Str".to_string()),
            type_name: "&Str".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Reference {
                mutable: false,
                inner: Box::new(Type::Str),
            }),
            trait_name: Some("Show".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("show".to_string(), method)]),
        };
        let mut traits = HashMap::new();
        register_trait_member(&mut traits, &imp, "show");
        let impls = HashMap::from([(imp.id, imp)]);
        let resolver = ResolverTables::default();
        let current_impl_bounds = HirGenericBounds::new();
        let effective = effective_trait_methods_for(&traits, &impls);
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds)
            .with_effective_trait_methods(&effective);
        let receiver = HirExpr {
            kind: HirExprKind::Var("s".to_string()),
            ty: Type::Reference {
                mutable: true,
                inner: Box::new(Type::Str),
            },
            span: Span::test(),
        };

        let selected = service
            .select_concrete_method(
                &[ReceiverCandidate {
                    expr: receiver,
                    adjustment: ReceiverAdjustment::None,
                    can_autoref_mut: false,
                }],
                "show",
                |ty| ty.clone(),
            )
            .expect("&mut Str should match &Str reference-owned impl");

        assert_eq!(
            selected.receiver_adjustment,
            ReceiverAdjustment::MutToSharedRef
        );
    }

    #[test]
    fn selection_autorefs_value_for_reference_owned_shared_method() {
        let trait_id = def_id(30);
        let impl_id = def_id(40);
        let method_id = def_id(41);
        let method = HirFunction {
            id: method_id,
            name: "show".to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "self".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::Reference {
                        mutable: false,
                        inner: Box::new(Type::Str),
                    }),
                },
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::Str,
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::Str,
            },
            is_curried: false,
            is_method: true,
            self_receiver: Some(ReceiverMode::Shared),
            is_unsafe: false,
        };
        let imp = HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("&Str".to_string()),
            type_name: "&Str".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Reference {
                mutable: false,
                inner: Box::new(Type::Str),
            }),
            trait_name: Some("Show".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("show".to_string(), method)]),
        };
        let mut traits = HashMap::new();
        register_trait_member(&mut traits, &imp, "show");
        let impls = HashMap::from([(imp.id, imp)]);
        let resolver = ResolverTables::default();
        let current_impl_bounds = HirGenericBounds::new();
        let effective = effective_trait_methods_for(&traits, &impls);
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds)
            .with_effective_trait_methods(&effective);
        let receiver = HirExpr {
            kind: HirExprKind::Var("s".to_string()),
            ty: Type::Str,
            span: Span::test(),
        };

        let selected = service
            .select_concrete_method(&[receiver_candidate(receiver)], "show", |ty| ty.clone())
            .expect("Str should autoref to the &Str impl owner before shared self borrow");

        assert_eq!(
            selected.receiver_adjustment,
            ReceiverAdjustment::AutorefShared
        );
    }

    #[test]
    fn exact_index_selection_reports_missing_implementation() {
        let traits: HashMap<DefId, HirTrait> = HashMap::new();
        let impls: HashMap<DefId, HirImpl> = HashMap::new();
        let bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &bounds);
        let receiver = HirExpr {
            kind: HirExprKind::Var("items".to_string()),
            ty: Type::Array(Box::new(Type::I64), 4),
            span: Span::test(),
        };

        assert!(matches!(
            service.select_required_index_method(
                &[receiver_candidate(receiver)],
                def_id(10),
                def_id(11),
                AssocTypeId(0),
                &Type::I64,
                "[]",
                |ty| ty.clone(),
            ),
            Err(SelectionDiagnostic::NoImplementation { .. })
        ));
    }

    #[test]
    fn selection_service_selects_concrete_impl_method_by_identity() {
        let owner = def_id(10);
        let impl_id = def_id(20);
        let method_id = def_id(21);
        let method = method(method_id, owner);
        let imp = HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: owner,
                args: Vec::new(),
            }),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), method)]),
        };
        let traits: HashMap<DefId, HirTrait> = HashMap::new();
        let impls = HashMap::from([(imp.id, imp)]);
        let mut resolver = ResolverTables::default();
        resolver.item_names_by_id.insert(owner, "Box".to_string());
        let current_impl_bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
        let receiver = HirExpr {
            kind: HirExprKind::Var("box".to_string()),
            ty: Type::Struct {
                id: owner,
                args: Vec::new(),
            },
            span: Span::test(),
        };

        let selected = service
            .select_concrete_method(&[receiver_candidate(receiver)], "value", |ty| ty.clone())
            .expect("expected selected method");

        assert_eq!(selected.target.impl_id(), Some(impl_id));
        assert_eq!(selected.target.method_id(), Some(method_id));
        assert_eq!(selected.return_type, Type::I64);
    }

    #[test]
    fn selection_service_selects_current_trait_self_method_by_trait_identity() {
        let trait_id = def_id(10);
        let method_id = def_id(11);
        let self_param = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let mut method = method(method_id, trait_id);
        method.name = "value".to_string();
        method.params[0].ty = Type::Generic(self_param);
        method.ret_type = Type::Bool;

        let traits = HashMap::from([(
            trait_id,
            crate::hir::HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Reader".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::from([("value".to_string(), method.clone())]),
                signatures: HashMap::new(),
            },
        )]);
        let impls: HashMap<DefId, HirImpl> = HashMap::new();
        let resolver = ResolverTables::default();
        let current_impl_bounds = HirGenericBounds::new();
        let service =
            SelectionService::new(&traits, &impls, None, Some(trait_id), &current_impl_bounds);
        let receiver = HirExpr {
            kind: HirExprKind::Var("self".to_string()),
            ty: Type::Generic(self_param),
            span: Span::test(),
        };

        let selected = service
            .select_current_trait_method(receiver, "value", true)
            .expect("expected current-trait selection");

        assert_eq!(selected.origin, SelectedOrigin::CurrentTrait { trait_id });
        assert_eq!(selected.return_type, Type::Bool);
        assert_eq!(selected.target.impl_id(), None);
        assert_eq!(selected.target.trait_id(), Some(trait_id));
        assert_eq!(selected.target.method_id(), Some(method_id));
    }

    #[test]
    fn selection_service_current_trait_signature_uses_canonical_params_without_duplicate_self() {
        let trait_id = def_id(12);
        let method_id = def_id(13);
        let self_param = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let self_ty = Type::Reference {
            mutable: true,
            inner: Box::new(Type::Generic(self_param)),
        };
        let traits = HashMap::from([(
            trait_id,
            crate::hir::HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Reader".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::from([(
                    "set".to_string(),
                    crate::hir::HirFunctionSig {
                        id: method_id,
                        name: "set".to_string(),
                        generic_params: vec![GenericParamDecl::type_param(self_param, "Self")],
                        params: vec![self_ty.clone(), Type::I64],
                        ret: Type::Bool,
                        generic_bounds: HashMap::new().into(),
                        self_receiver: Some(ReceiverMode::Mut),
                        is_unsafe: false,
                    },
                )]),
            },
        )]);
        let impls: HashMap<DefId, HirImpl> = HashMap::new();
        let resolver = ResolverTables::default();
        let current_impl_bounds = HirGenericBounds::new();
        let service =
            SelectionService::new(&traits, &impls, None, Some(trait_id), &current_impl_bounds);
        let receiver = HirExpr {
            kind: HirExprKind::Var("self".to_string()),
            ty: Type::Generic(self_param),
            span: Span::test(),
        };

        let selected = service
            .select_current_trait_method(receiver, "set", true)
            .expect("expected current-trait signature selection");
        let function = selected
            .function
            .expect("selection should fabricate function");

        assert_eq!(function.params.len(), 2);
        assert_eq!(function.params[0].name, "self");
        assert!(function.params[0].mutable);
        assert_eq!(function.params[0].ty, self_ty);
        assert_eq!(function.params[1].name, "arg1");
        assert_eq!(function.params[1].ty, Type::I64);
        assert_eq!(selected.substituted_params.len(), 1);
        assert_eq!(selected.substituted_params[0].ty, Type::I64);
    }

    #[test]
    fn selection_service_bound_signature_uses_canonical_params_without_duplicate_self() {
        let trait_id = def_id(14);
        let method_id = def_id(15);
        let self_param = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let self_ty = Type::Reference {
            mutable: false,
            inner: Box::new(Type::Generic(self_param)),
        };
        let traits = HashMap::from([(
            trait_id,
            crate::hir::HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Show".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::from([(
                    "show".to_string(),
                    crate::hir::HirFunctionSig {
                        id: method_id,
                        name: "show".to_string(),
                        generic_params: vec![GenericParamDecl::type_param(self_param, "Self")],
                        params: vec![self_ty.clone()],
                        ret: Type::I64,
                        generic_bounds: HashMap::new().into(),
                        self_receiver: Some(ReceiverMode::Shared),
                        is_unsafe: false,
                    },
                )]),
            },
        )]);
        let impls: HashMap<DefId, HirImpl> = HashMap::new();
        let resolver = ResolverTables::default();
        let current_impl_bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
        let receiver = HirExpr {
            kind: HirExprKind::Var("value".to_string()),
            ty: Type::Generic(self_param),
            span: Span::test(),
        };

        let selected = service
            .select_bound_method(
                &[receiver_candidate(receiver)],
                &[crate::types::TraitBound {
                    trait_id,
                    type_args: Vec::new(),
                }],
                "show",
                Type::Generic(self_param),
            )
            .expect("expected bound signature selection");
        let function = selected
            .function
            .expect("selection should fabricate function");

        assert_eq!(function.params.len(), 1);
        assert_eq!(function.params[0].name, "self");
        assert_eq!(function.params[0].ty, self_ty);
        assert!(selected.substituted_params.is_empty());
    }

    #[test]
    fn selection_service_bound_mut_receiver_uses_autoref_mut_candidate() {
        let trait_id = def_id(18);
        let method_id = def_id(19);
        let self_param = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let traits = HashMap::from([(
            trait_id,
            crate::hir::HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Touch".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::from([(
                    "touch".to_string(),
                    crate::hir::HirFunctionSig {
                        id: method_id,
                        name: "touch".to_string(),
                        generic_params: vec![GenericParamDecl::type_param(self_param, "Self")],
                        params: vec![Type::Reference {
                            mutable: true,
                            inner: Box::new(Type::Generic(self_param)),
                        }],
                        ret: Type::Unit,
                        generic_bounds: HashMap::new().into(),
                        self_receiver: Some(ReceiverMode::Mut),
                        is_unsafe: false,
                    },
                )]),
            },
        )]);
        let impls: HashMap<DefId, HirImpl> = HashMap::new();
        let resolver = ResolverTables::default();
        let current_impl_bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
        let receiver = HirExpr {
            kind: HirExprKind::Var("value".to_string()),
            ty: Type::Generic(self_param),
            span: Span::test(),
        };

        let selected = service
            .select_bound_method(
                &[ReceiverCandidate {
                    expr: receiver,
                    adjustment: ReceiverAdjustment::None,
                    can_autoref_mut: true,
                }],
                &[crate::types::TraitBound {
                    trait_id,
                    type_args: Vec::new(),
                }],
                "touch",
                Type::Generic(self_param),
            )
            .expect("mutable trait-bound receiver should autoref mut");

        assert_eq!(selected.receiver_adjustment, ReceiverAdjustment::AutorefMut);
    }

    #[test]
    fn selection_service_bound_shared_receiver_on_ref_uses_deref_candidate() {
        let trait_id = def_id(20);
        let method_id = def_id(21);
        let self_param = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let traits = HashMap::from([(
            trait_id,
            crate::hir::HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Readable".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::from([(
                    "read".to_string(),
                    crate::hir::HirFunctionSig {
                        id: method_id,
                        name: "read".to_string(),
                        generic_params: vec![GenericParamDecl::type_param(self_param, "Self")],
                        params: vec![Type::Reference {
                            mutable: false,
                            inner: Box::new(Type::Generic(self_param)),
                        }],
                        ret: Type::I64,
                        generic_bounds: HashMap::new().into(),
                        self_receiver: Some(ReceiverMode::Shared),
                        is_unsafe: false,
                    },
                )]),
            },
        )]);
        let impls: HashMap<DefId, HirImpl> = HashMap::new();
        let resolver = ResolverTables::default();
        let current_impl_bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
        let receiver = HirExpr {
            kind: HirExprKind::Var("value".to_string()),
            ty: Type::Reference {
                mutable: false,
                inner: Box::new(Type::Generic(self_param)),
            },
            span: Span::test(),
        };
        let deref_receiver = HirExpr {
            kind: HirExprKind::Deref(Box::new(receiver.clone())),
            ty: Type::Generic(self_param),
            span: Span::test(),
        };

        let selected = service
            .select_bound_method(
                &[
                    ReceiverCandidate {
                        expr: receiver,
                        adjustment: ReceiverAdjustment::None,
                        can_autoref_mut: false,
                    },
                    ReceiverCandidate {
                        expr: deref_receiver,
                        adjustment: ReceiverAdjustment::BuiltinDeref,
                        can_autoref_mut: false,
                    },
                ],
                &[crate::types::TraitBound {
                    trait_id,
                    type_args: Vec::new(),
                }],
                "read",
                Type::Generic(self_param),
            )
            .expect("shared trait-bound receiver on &T should dereference before borrow");

        assert!(matches!(selected.receiver.kind, HirExprKind::Deref(_)));
        assert_eq!(
            selected.receiver_adjustment,
            ReceiverAdjustment::AutorefShared
        );
    }

    #[test]
    fn selection_service_rejects_bound_signature_receiver_missing_from_params() {
        let trait_id = def_id(16);
        let method_id = def_id(17);
        let traits = HashMap::from([(
            trait_id,
            crate::hir::HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Show".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::from([(
                    "show".to_string(),
                    crate::hir::HirFunctionSig {
                        id: method_id,
                        name: "show".to_string(),
                        generic_params: Vec::new(),
                        params: Vec::new(),
                        ret: Type::I64,
                        generic_bounds: HashMap::new().into(),
                        self_receiver: Some(ReceiverMode::Shared),
                        is_unsafe: false,
                    },
                )]),
            },
        )]);
        let impls: HashMap<DefId, HirImpl> = HashMap::new();
        let resolver = ResolverTables::default();
        let current_impl_bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
        let receiver = HirExpr {
            kind: HirExprKind::Var("value".to_string()),
            ty: Type::I64,
            span: Span::test(),
        };

        assert!(service
            .select_bound_method(
                &[receiver_candidate(receiver)],
                &[crate::types::TraitBound {
                    trait_id,
                    type_args: Vec::new(),
                }],
                "show",
                Type::I64,
            )
            .is_err());
    }

    #[test]
    fn selection_service_selects_unary_operator_method_by_required_trait() {
        let owner = def_id(20);
        let trait_id = def_id(30);
        let impl_id = def_id(40);
        let method_id = def_id(41);
        let mut method = operator_method(method_id, owner, Type::I64);
        method.name = "-".to_string();
        method.params.pop();
        let mut traits = HashMap::from([(
            trait_id,
            crate::hir::HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Neg".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        )]);
        let imp = HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Number".to_string()),
            type_name: "Number".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: owner,
                args: Vec::new(),
            }),
            trait_name: Some("Neg".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("-".to_string(), method)]),
        };
        register_trait_member(&mut traits, &imp, "-");
        let impls = HashMap::from([(imp.id, imp)]);
        let mut resolver = ResolverTables::default();
        resolver
            .item_names_by_id
            .insert(owner, "Number".to_string());
        let current_impl_bounds = HirGenericBounds::new();
        let effective = effective_trait_methods_for(&traits, &impls);
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds)
            .with_effective_trait_methods(&effective);
        let receiver = HirExpr {
            kind: HirExprKind::Var("n".to_string()),
            ty: Type::Struct {
                id: owner,
                args: Vec::new(),
            },
            span: Span::test(),
        };

        let selected = service
            .select_unary_operator_method(&receiver, &receiver.ty, trait_id, "-")
            .expect("expected unary operator selection");

        assert_eq!(
            selected.origin,
            SelectedOrigin::TraitImpl { impl_id, trait_id }
        );
        assert_eq!(selected.target.method_id(), Some(method_id));
        assert_eq!(selected.return_type, Type::I64);
    }

    #[test]
    fn selection_service_selects_trait_impl_with_trait_args() {
        let owner = def_id(60);
        let trait_id = def_id(61);
        let impl_id = def_id(62);
        let imp = HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Bag".to_string()),
            type_name: "Bag".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: owner,
                args: Vec::new(),
            }),
            trait_name: Some("Index".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "Idx",
            )],
            trait_arg_types: vec![Type::I64],
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        };
        let traits: HashMap<DefId, HirTrait> = HashMap::new();
        let impls = HashMap::from([(imp.id, imp)]);
        let mut resolver = ResolverTables::default();
        resolver.item_names_by_id.insert(owner, "Bag".to_string());
        let current_impl_bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
        let receiver_ty = Type::Struct {
            id: owner,
            args: Vec::new(),
        };

        let selected = service
            .select_trait_impl(&receiver_ty, trait_id, &[Type::I64])
            .expect("expected trait impl selection");

        assert_eq!(selected.id, impl_id);
        assert!(service
            .select_trait_impl(&receiver_ty, trait_id, &[Type::Bool])
            .is_err());
    }

    #[test]
    fn selection_service_rejects_equally_specific_trait_impls() {
        let owner = def_id(600);
        let trait_id = def_id(601);
        let impl_template = HirImpl {
            id: def_id(602),
            owner: HirImplOwner::Named("Bag".to_string()),
            type_name: "Bag".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: owner,
                args: Vec::new(),
            }),
            trait_name: Some("Marker".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::new(),
        };
        let mut second = impl_template.clone();
        second.id = def_id(603);
        let impls = HashMap::from([(impl_template.id, impl_template), (second.id, second)]);
        let traits: HashMap<DefId, HirTrait> = HashMap::new();
        let mut resolver = ResolverTables::default();
        resolver.item_names_by_id.insert(owner, "Bag".to_string());
        let current_impl_bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);

        assert!(service
            .select_trait_impl(
                &Type::Struct {
                    id: owner,
                    args: Vec::new(),
                },
                trait_id,
                &[],
            )
            .is_err());
    }

    #[test]
    fn selection_service_rejects_differently_specific_trait_impls() {
        let owner = def_id(610);
        let trait_id = def_id(611);
        let generic_impl_id = def_id(612);
        let generic = GenericParamId {
            owner: generic_impl_id,
            index: 0,
        };
        let generic_impl = HirImpl {
            id: generic_impl_id,
            owner: HirImplOwner::Named("T".to_string()),
            type_name: "T".to_string(),
            type_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: generic_impl_id,
                    index: 0,
                },
                "T",
            )],
            receiver_pattern: vec![Type::Generic(generic)].into(),
            trait_name: Some("Marker".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::new(),
        };
        let concrete_impl = HirImpl {
            id: def_id(613),
            owner: HirImplOwner::Named("Bag".to_string()),
            type_name: "Bag".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: owner,
                args: Vec::new(),
            }),
            trait_name: Some("Marker".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::new(),
        };
        let impls = HashMap::from([
            (generic_impl.id, generic_impl),
            (concrete_impl.id, concrete_impl),
        ]);
        let traits: HashMap<DefId, HirTrait> = HashMap::new();
        let mut resolver = ResolverTables::default();
        resolver.item_names_by_id.insert(owner, "Bag".to_string());
        let current_impl_bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);

        let error = service
            .select_trait_impl(
                &Type::Struct {
                    id: owner,
                    args: Vec::new(),
                },
                trait_id,
                &[],
            )
            .unwrap_err();
        assert_eq!(
            error,
            crate::selection::SelectionDiagnostic::AmbiguousCandidates {
                operation: "the requested trait (upstream coherence invariant violation: overlapping impls reached selection)".to_string(),
                receiver: Type::Struct {
                    id: owner,
                    args: Vec::new(),
                },
                candidates: vec![generic_impl_id, def_id(613)],
            }
        );
    }

    #[test]
    fn selection_service_current_generic_bound_satisfies_impl_bound() {
        let owner = def_id(63);
        let trait_id = def_id(64);
        let bound_trait_id = def_id(65);
        let impl_id = def_id(66);
        let impl_generic = GenericParamId {
            owner: impl_id,
            index: 0,
        };
        let caller_generic = GenericParamId {
            owner: def_id(67),
            index: 0,
        };
        let marker_bound = TraitBound {
            trait_id: bound_trait_id,
            type_args: Vec::new(),
        };
        let imp = HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: impl_id,
                    index: 0,
                },
                "T",
            )],
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: owner,
                args: vec![Type::Generic(impl_generic)],
            }),
            trait_name: Some("Provider".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::from([(
                impl_generic,
                vec![TraitBound {
                    trait_id: bound_trait_id,
                    type_args: Vec::new(),
                }],
            )])
            .into(),
            methods: HashMap::new(),
        };
        let traits: HashMap<DefId, HirTrait> = HashMap::new();
        let impls = HashMap::from([(imp.id, imp)]);
        let mut resolver = ResolverTables::default();
        resolver.item_names_by_id.insert(owner, "Box".to_string());
        let current_impl_bounds = HashMap::from([(caller_generic, vec![marker_bound])]).into();
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
        let receiver_ty = Type::Struct {
            id: owner,
            args: vec![Type::Generic(caller_generic)],
        };

        let selected = service
            .select_trait_impl(&receiver_ty, trait_id, &[])
            .expect("matching current generic bound should satisfy impl bound");

        assert_eq!(selected.id, impl_id);
    }

    #[test]
    fn selection_service_strict_trait_impl_rejects_pending_impl_bound() {
        let owner = def_id(68);
        let trait_id = def_id(69);
        let bound_trait_id = def_id(70);
        let impl_id = def_id(71);
        let impl_generic = GenericParamId {
            owner: impl_id,
            index: 0,
        };
        let imp = HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: impl_id,
                    index: 0,
                },
                "T",
            )],
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: owner,
                args: vec![Type::Generic(impl_generic)],
            }),
            trait_name: Some("Provider".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::from([(
                impl_generic,
                vec![TraitBound {
                    trait_id: bound_trait_id,
                    type_args: Vec::new(),
                }],
            )])
            .into(),
            methods: HashMap::new(),
        };
        let traits: HashMap<DefId, HirTrait> = HashMap::new();
        let impls = HashMap::from([(imp.id, imp)]);
        let mut resolver = ResolverTables::default();
        resolver.item_names_by_id.insert(owner, "Box".to_string());
        let current_impl_bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
        let receiver_ty = Type::Struct {
            id: owner,
            args: vec![Type::TypeVar(TypeVarId(0))],
        };

        assert!(service
            .select_trait_impl(&receiver_ty, trait_id, &[])
            .is_ok());
        assert!(service
            .select_trait_impl_strict(&receiver_ty, trait_id, &[])
            .is_err());
    }

    #[test]
    fn selection_service_records_unresolved_generic_trait_operator_target() {
        let trait_id = def_id(30);
        let method_id = def_id(31);
        let traits = HashMap::from([(
            trait_id,
            crate::hir::HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Num".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::from([(
                    "+".to_string(),
                    crate::hir::HirFunctionSig {
                        id: method_id,
                        name: "+".to_string(),
                        generic_params: Vec::new(),
                        params: vec![Type::I64],
                        ret: Type::I64,
                        generic_bounds: HashMap::new().into(),
                        self_receiver: Some(ReceiverMode::Shared),
                        is_unsafe: false,
                    },
                )]),
            },
        )]);
        let impls: HashMap<DefId, HirImpl> = HashMap::new();
        let resolver = ResolverTables::default();
        let current_impl_bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
        let receiver = HirExpr {
            kind: HirExprKind::Var("value".to_string()),
            ty: Type::TypeVar(TypeVarId(7)),
            span: Span::test(),
        };

        let selected = service
            .select_unresolved_generic_method(receiver, trait_id, "+", Vec::new(), Type::I64)
            .expect("expected unresolved generic operator target");

        assert_eq!(
            selected.origin,
            SelectedOrigin::UnresolvedGeneric { trait_id }
        );
        assert_eq!(selected.target.method_id(), Some(method_id));
        assert_eq!(selected.return_type, Type::I64);
    }

    #[test]
    fn selection_service_records_unresolved_generic_index_target() {
        let trait_id = def_id(30);
        let method_id = def_id(31);
        let traits = HashMap::from([(
            trait_id,
            crate::hir::HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Index".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::from([(
                    "index".to_string(),
                    crate::hir::HirFunctionSig {
                        id: method_id,
                        name: "index".to_string(),
                        generic_params: Vec::new(),
                        params: vec![Type::I64],
                        ret: Type::Reference {
                            mutable: false,
                            inner: Box::new(Type::I64),
                        },
                        generic_bounds: HashMap::new().into(),
                        self_receiver: Some(ReceiverMode::Shared),
                        is_unsafe: false,
                    },
                )]),
            },
        )]);
        let impls: HashMap<DefId, HirImpl> = HashMap::new();
        let resolver = ResolverTables::default();
        let current_impl_bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
        let receiver = HirExpr {
            kind: HirExprKind::Var("items".to_string()),
            ty: Type::TypeVar(TypeVarId(7)),
            span: Span::test(),
        };
        let return_type = Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        };

        let selected = service
            .select_unresolved_generic_required_index_method(
                receiver,
                trait_id,
                method_id,
                AssocTypeId(0),
                Type::I64,
                return_type.clone(),
                "[]",
            )
            .expect("expected unresolved generic index target");

        let target = selected.target;
        assert_eq!(target.impl_id(), None);
        assert_eq!(target.trait_id(), Some(trait_id));
        assert_eq!(target.trait_args(), &[Type::I64]);
        assert_eq!(target.method_id(), Some(method_id));
        assert_eq!(selected.return_type, return_type);
    }

    #[test]
    fn index_selection_reports_missing_marked_trait_without_panicking() {
        let traits = HashMap::new();
        let impls = HashMap::new();
        let bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &bounds);
        let trait_id = def_id(40);
        let member_id = def_id(41);
        let receiver = HirExpr {
            kind: HirExprKind::Var("value".to_string()),
            ty: Type::TypeVar(TypeVarId(0)),
            span: Span::test(),
        };

        assert!(matches!(
            service.select_unresolved_generic_required_index_method(
                receiver,
                trait_id,
                member_id,
                AssocTypeId(0),
                Type::I64,
                Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::I64),
                },
                "[]",
            ),
            Err(SelectionDiagnostic::TraitMemberIdMissing {
                trait_id: error_trait_id,
                member_id: error_member_id,
            }) if error_trait_id == trait_id && error_member_id == member_id
        ));
    }

    #[test]
    fn selection_service_selects_generic_fixed_array_builtin_slice_impl() {
        let impl_id = def_id(20);
        let trait_id = def_id(30);
        let method_id = def_id(21);
        let generic_t = GenericParamId {
            owner: impl_id,
            index: 0,
        };
        let mut method = method(method_id, impl_id);
        method.name = "echo".to_string();
        method.params[0].ty = Type::Array(Box::new(Type::Generic(generic_t)), 3);
        method.params.push(HirParam {
            name: "value".to_string(),
            local_id: crate::ids::HirLocalId(1),
            ty: Type::I64,
            mutable: false,
            is_ref: false,
        });

        let imp = HirImpl {
            id: impl_id,
            owner: HirImplOwner::BuiltinSlice,
            type_name: "[T; 3]".to_string(),
            type_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: impl_id,
                    index: 0,
                },
                "T",
            )],
            receiver_pattern: HirImplReceiverPattern::SliceFamily {
                element: Type::Generic(generic_t),
            },
            trait_name: Some("Echo".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("echo".to_string(), method)]),
        };
        let mut traits = HashMap::new();
        register_trait_member(&mut traits, &imp, "echo");
        let impls = HashMap::from([(imp.id, imp)]);
        let resolver = ResolverTables::default();
        let current_impl_bounds = HirGenericBounds::new();
        let effective = effective_trait_methods_for(&traits, &impls);
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds)
            .with_effective_trait_methods(&effective);
        let receiver = HirExpr {
            kind: HirExprKind::Var("arr".to_string()),
            ty: Type::Array(Box::new(Type::I64), 3),
            span: Span::test(),
        };

        let target = service
            .select_concrete_method(&[receiver_candidate(receiver)], "echo", |ty| ty.clone())
            .expect("expected fixed-array impl selection")
            .target;

        assert_eq!(target.impl_id(), Some(impl_id));
        assert_eq!(target.trait_id(), Some(trait_id));
        assert_eq!(target.method_id(), Some(method_id));
    }

    #[test]
    fn selection_authority_selects_same_named_method_by_receiver_identity() {
        let left_owner = def_id(100);
        let right_owner = def_id(101);
        let left_impl_id = def_id(110);
        let right_impl_id = def_id(111);
        let left_method_id = def_id(120);
        let right_method_id = def_id(121);
        let impls: HashMap<DefId, HirImpl> = vec![
            HirImpl {
                id: left_impl_id,
                owner: HirImplOwner::Named("left::Box".to_string()),
                type_name: "left::Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                    id: left_owner,
                    args: Vec::new(),
                }),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("value".to_string(), method(left_method_id, left_owner))]),
            },
            HirImpl {
                id: right_impl_id,
                owner: HirImplOwner::Named("right::Box".to_string()),
                type_name: "right::Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                    id: right_owner,
                    args: Vec::new(),
                }),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([(
                    "value".to_string(),
                    method(right_method_id, right_owner),
                )]),
            },
        ]
        .into_iter()
        .map(|imp| (imp.id, imp))
        .collect();
        let traits: HashMap<DefId, HirTrait> = HashMap::new();
        let mut resolver = ResolverTables::default();
        resolver
            .item_names_by_id
            .insert(left_owner, "left::Box".to_string());
        resolver
            .item_names_by_id
            .insert(right_owner, "right::Box".to_string());
        let current_impl_bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
        let receiver = HirExpr {
            kind: HirExprKind::Var("box".to_string()),
            ty: Type::Struct {
                id: right_owner,
                args: Vec::new(),
            },
            span: Span::test(),
        };

        let authority = service
            .select_concrete_method(&[receiver_candidate(receiver)], "value", |ty| ty.clone())
            .expect("expected right-side selected method")
            .authority();

        assert_eq!(authority.impl_id(), Some(right_impl_id));
        assert_eq!(authority.method_id(), Some(right_method_id));
    }

    #[test]
    fn selection_service_rejects_shared_receiver_display_name_collision() {
        let left_owner = def_id(130);
        let right_owner = def_id(131);
        let left_impl_id = def_id(132);
        let right_impl_id = def_id(133);
        let left_method_id = def_id(134);
        let right_method_id = def_id(135);
        let impls: HashMap<DefId, HirImpl> = vec![
            HirImpl {
                id: left_impl_id,
                owner: HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                    id: left_owner,
                    args: Vec::new(),
                }),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("value".to_string(), method(left_method_id, left_owner))]),
            },
            HirImpl {
                id: right_impl_id,
                owner: HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                    id: right_owner,
                    args: Vec::new(),
                }),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([(
                    "value".to_string(),
                    method(right_method_id, right_owner),
                )]),
            },
        ]
        .into_iter()
        .map(|imp| (imp.id, imp))
        .collect();
        let traits: HashMap<DefId, HirTrait> = HashMap::new();
        let mut resolver = ResolverTables::default();
        resolver
            .item_names_by_id
            .insert(right_owner, "Box".to_string());
        let current_impl_bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
        let receiver = HirExpr {
            kind: HirExprKind::Var("box".to_string()),
            ty: Type::Struct {
                id: right_owner,
                args: Vec::new(),
            },
            span: Span::test(),
        };

        let authority = service
            .select_concrete_method(&[receiver_candidate(receiver)], "value", |ty| ty.clone())
            .expect("expected selected method by receiver identity")
            .authority();

        assert_eq!(authority.impl_id(), Some(right_impl_id));
        assert_eq!(authority.method_id(), Some(right_method_id));
    }

    #[test]
    fn selection_service_rejects_trait_receiver_display_name_collision() {
        let left_owner = def_id(136);
        let right_owner = def_id(137);
        let trait_id = def_id(138);
        let left_impl_id = def_id(139);
        let right_impl_id = def_id(140);
        let left_method_id = def_id(141);
        let right_method_id = def_id(142);
        let receiver_ty = Type::Struct {
            id: right_owner,
            args: Vec::new(),
        };
        let impls: HashMap<DefId, HirImpl> = vec![
            HirImpl {
                id: left_impl_id,
                owner: HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                    id: left_owner,
                    args: Vec::new(),
                }),
                trait_name: Some("Add".to_string()),
                trait_id: Some(trait_id),
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([(
                    "+".to_string(),
                    operator_method(left_method_id, left_owner, Type::I64),
                )]),
            },
            HirImpl {
                id: right_impl_id,
                owner: HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                    id: right_owner,
                    args: Vec::new(),
                }),
                trait_name: Some("Add".to_string()),
                trait_id: Some(trait_id),
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([(
                    "+".to_string(),
                    operator_method(right_method_id, right_owner, Type::I64),
                )]),
            },
        ]
        .into_iter()
        .map(|imp| (imp.id, imp))
        .collect();
        let mut traits = HashMap::new();
        register_trait_member(&mut traits, impls.get(&right_impl_id).unwrap(), "+");
        let mut resolver = ResolverTables::default();
        resolver
            .item_names_by_id
            .insert(right_owner, "Box".to_string());
        let current_impl_bounds = HirGenericBounds::new();
        let effective = effective_trait_methods_for(&traits, &impls);
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds)
            .with_effective_trait_methods(&effective);
        let receiver = HirExpr {
            kind: HirExprKind::Var("box".to_string()),
            ty: receiver_ty.clone(),
            span: Span::test(),
        };

        let authority = service
            .select_required_trait_method(&receiver, &receiver_ty, trait_id, "+")
            .expect("expected selected trait method by receiver identity")
            .authority();

        assert_eq!(authority.impl_id(), Some(right_impl_id));
        assert_eq!(authority.method_id(), Some(right_method_id));
    }

    #[test]
    fn selection_service_rejects_index_receiver_display_name_collision() {
        let left_owner = def_id(143);
        let right_owner = def_id(144);
        let trait_id = def_id(145);
        let left_impl_id = def_id(146);
        let right_impl_id = def_id(147);
        let left_method_id = def_id(148);
        let right_method_id = def_id(149);
        let mut left_method = method(left_method_id, left_owner);
        left_method.name = "index".to_string();
        left_method.params.push(HirParam {
            name: "index".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::I64,
            mutable: false,
            is_ref: false,
        });
        let mut right_method = method(right_method_id, right_owner);
        right_method.name = "index".to_string();
        right_method.params.push(HirParam {
            name: "index".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::I64,
            mutable: false,
            is_ref: false,
        });
        let impls: HashMap<DefId, HirImpl> = vec![
            HirImpl {
                id: left_impl_id,
                owner: HirImplOwner::Named("Bag".to_string()),
                type_name: "Bag".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                    id: left_owner,
                    args: Vec::new(),
                }),
                trait_name: Some("Index".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: trait_id,
                        index: 0,
                    },
                    "Idx",
                )],
                trait_arg_types: vec![Type::I64],
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("index".to_string(), left_method)]),
            },
            HirImpl {
                id: right_impl_id,
                owner: HirImplOwner::Named("Bag".to_string()),
                type_name: "Bag".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                    id: right_owner,
                    args: Vec::new(),
                }),
                trait_name: Some("Index".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: trait_id,
                        index: 0,
                    },
                    "Idx",
                )],
                trait_arg_types: vec![Type::I64],
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("index".to_string(), right_method)]),
            },
        ]
        .into_iter()
        .map(|imp| (imp.id, imp))
        .collect();
        let mut traits = HashMap::new();
        let index_member_id =
            register_trait_member(&mut traits, impls.get(&right_impl_id).unwrap(), "index");
        let mut resolver = ResolverTables::default();
        resolver
            .item_names_by_id
            .insert(right_owner, "Bag".to_string());
        let current_impl_bounds = HirGenericBounds::new();
        let effective = HashMap::from([((right_impl_id, index_member_id), right_method_id)]);
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds)
            .with_effective_trait_methods(&effective);
        let receiver = HirExpr {
            kind: HirExprKind::Var("bag".to_string()),
            ty: Type::Struct {
                id: right_owner,
                args: Vec::new(),
            },
            span: Span::test(),
        };

        let authority = service
            .select_required_index_method(
                &[receiver_candidate(receiver)],
                trait_id,
                index_member_id,
                AssocTypeId(0),
                &Type::I64,
                "[]",
                |ty| ty.clone(),
            )
            .expect("expected selected index method by receiver identity")
            .authority();

        assert_eq!(authority.impl_id(), Some(right_impl_id));
        assert_eq!(authority.method_id(), Some(right_method_id));
    }

    #[test]
    fn selection_authority_uses_dependency_resolver_for_artifact_backed_receiver() {
        let dependency_owner = DefId::new(CrateId(7), LocalDefId(3));
        let impl_id = DefId::new(CrateId(7), LocalDefId(4));
        let method_id = DefId::new(CrateId(7), LocalDefId(5));
        let imp = HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("dep::Box".to_string()),
            type_name: "dep::Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: dependency_owner,
                args: Vec::new(),
            }),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), method(method_id, dependency_owner))]),
        };
        let traits: HashMap<DefId, HirTrait> = HashMap::new();
        let impls = HashMap::from([(imp.id, imp)]);
        let resolver = ResolverTables::default();
        let mut dependency_resolver = ResolverTables::default();
        dependency_resolver
            .item_names_by_id
            .insert(dependency_owner, "dep::Box".to_string());
        let dependency_resolvers = HashMap::from([("dep".to_string(), dependency_resolver)]);
        let current_impl_bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
        let receiver = HirExpr {
            kind: HirExprKind::Var("box".to_string()),
            ty: Type::Struct {
                id: dependency_owner,
                args: Vec::new(),
            },
            span: Span::test(),
        };

        let authority = service
            .select_concrete_method(&[receiver_candidate(receiver)], "value", |ty| ty.clone())
            .expect("expected dependency-backed selected method")
            .authority();

        assert_eq!(authority.impl_id(), Some(impl_id));
        assert_eq!(authority.method_id(), Some(method_id));
    }

    #[test]
    fn selection_service_rejects_unrelated_empty_receiver_arg_impl_without_identity() {
        let receiver_owner = def_id(10);
        let unrelated_impl_id = def_id(20);
        let unrelated_method_id = def_id(21);
        let unrelated_method = method(unrelated_method_id, receiver_owner);
        let unrelated_impl = HirImpl {
            id: unrelated_impl_id,
            owner: HirImplOwner::Named("Other".to_string()),
            type_name: "Other".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: def_id(22),
                args: Vec::new(),
            }),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("value".to_string(), unrelated_method)]),
        };
        let traits: HashMap<DefId, HirTrait> = HashMap::new();
        let impls = HashMap::from([(unrelated_impl.id, unrelated_impl)]);
        let resolver = ResolverTables::default();
        let current_impl_bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
        let receiver = HirExpr {
            kind: HirExprKind::Var("box".to_string()),
            ty: Type::Struct {
                id: receiver_owner,
                args: Vec::new(),
            },
            span: Span::test(),
        };

        assert!(service
            .select_concrete_method(&[receiver_candidate(receiver)], "value", |ty| ty.clone())
            .is_err());
    }

    #[test]
    fn selection_service_selects_type_var_bound_signature_by_trait_identity() {
        let trait_id = def_id(30);
        let signature_id = def_id(31);
        let trait_def = crate::hir::HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Readable".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "read".to_string(),
                crate::hir::HirFunctionSig {
                    id: signature_id,
                    name: "read".to_string(),
                    generic_params: Vec::new(),
                    params: vec![Type::Reference {
                        mutable: false,
                        inner: Box::new(Type::I64),
                    }],
                    ret: Type::I64,
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(ReceiverMode::Shared),
                    is_unsafe: false,
                },
            )]),
        };
        let traits = HashMap::from([(trait_def.id, trait_def)]);
        let impls: HashMap<DefId, HirImpl> = HashMap::new();
        let resolver = ResolverTables::default();
        let current_impl_bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
        let receiver = HirExpr {
            kind: HirExprKind::Var("receiver".to_string()),
            ty: Type::I64,
            span: Span::test(),
        };
        let bounds = vec![crate::types::TraitBound {
            trait_id,
            type_args: Vec::new(),
        }];

        let selected = service
            .select_bound_method(&[receiver_candidate(receiver)], &bounds, "read", Type::I64)
            .expect("expected trait-bound signature selection");

        assert_eq!(selected.target.trait_id(), Some(trait_id));
        assert_eq!(selected.target.method_id(), Some(signature_id));
        assert_eq!(selected.return_type, Type::I64);

        let authority = selected.authority();
        assert_eq!(authority.impl_id(), None);
        assert_eq!(authority.trait_id(), Some(trait_id));
        assert_eq!(authority.method_id(), Some(signature_id));
        assert!(authority.trait_args().is_empty());
        assert_eq!(
            authority.receiver_adjustment,
            ReceiverAdjustment::AutorefShared
        );
    }

    #[test]
    fn selection_service_selects_required_trait_operator_impl() {
        let owner = def_id(10);
        let inherent_impl_id = def_id(20);
        let inherent_method_id = def_id(21);
        let trait_id = def_id(30);
        let trait_impl_id = def_id(40);
        let trait_method_id = def_id(41);
        let inherent_impl = HirImpl {
            id: inherent_impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: owner,
                args: Vec::new(),
            }),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "+".to_string(),
                operator_method(inherent_method_id, owner, Type::Bool),
            )]),
        };
        let trait_impl = HirImpl {
            id: trait_impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: owner,
                args: Vec::new(),
            }),
            trait_name: Some("Num".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: vec![Type::Struct {
                id: owner,
                args: Vec::new(),
            }],
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "+".to_string(),
                operator_method(
                    trait_method_id,
                    owner,
                    Type::Struct {
                        id: owner,
                        args: Vec::new(),
                    },
                ),
            )]),
        };
        let mut traits = HashMap::new();
        register_trait_member(&mut traits, &trait_impl, "+");
        let impls = HashMap::from([
            (inherent_impl.id, inherent_impl),
            (trait_impl.id, trait_impl),
        ]);
        let mut resolver = ResolverTables::default();
        resolver.item_names_by_id.insert(owner, "Box".to_string());
        let current_impl_bounds = HirGenericBounds::new();
        let effective = effective_trait_methods_for(&traits, &impls);
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds)
            .with_effective_trait_methods(&effective);
        let receiver_ty = Type::Struct {
            id: owner,
            args: Vec::new(),
        };
        let receiver = HirExpr {
            kind: HirExprKind::Var("box".to_string()),
            ty: receiver_ty.clone(),
            span: Span::test(),
        };

        let selected = service
            .select_required_trait_method(&receiver, &receiver_ty, trait_id, "+")
            .expect("expected required trait operator selection");

        let target = &selected.target;
        assert_eq!(target.impl_id(), Some(trait_impl_id));
        assert_eq!(target.trait_id(), Some(trait_id));
        assert_eq!(target.trait_args(), &[receiver_ty.clone()]);
        assert_eq!(target.method_id(), Some(trait_method_id));
        assert_eq!(selected.return_type, receiver_ty);

        let authority = selected.authority();
        assert_eq!(authority.impl_id(), Some(trait_impl_id));
        assert_eq!(authority.trait_id(), Some(trait_id));
        assert_eq!(authority.method_id(), Some(trait_method_id));
        assert_eq!(authority.trait_args(), &[receiver_ty.clone()]);
        assert_eq!(authority.return_type, receiver_ty);
    }

    #[test]
    fn selection_service_selects_borrowed_receiver_trait_method_for_value_receiver() {
        let owner = def_id(60);
        let trait_id = def_id(61);
        let impl_id = def_id(62);
        let method_id = def_id(63);
        let receiver_ty = Type::Struct {
            id: owner,
            args: Vec::new(),
        };
        let mut method = operator_method(method_id, owner, receiver_ty.clone());
        method.params[0].ty = Type::Reference {
            mutable: false,
            inner: Box::new(receiver_ty.clone()),
        };
        let imp = HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: owner,
                args: Vec::new(),
            }),
            trait_name: Some("Num".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: vec![receiver_ty.clone()],
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("+".to_string(), method)]),
        };
        let mut traits = HashMap::new();
        register_trait_member(&mut traits, &imp, "+");
        let impls = HashMap::from([(imp.id, imp)]);
        let mut resolver = ResolverTables::default();
        resolver.item_names_by_id.insert(owner, "Box".to_string());
        let current_impl_bounds = HirGenericBounds::new();
        let effective = effective_trait_methods_for(&traits, &impls);
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds)
            .with_effective_trait_methods(&effective);
        let receiver = HirExpr {
            kind: HirExprKind::Var("box".to_string()),
            ty: receiver_ty.clone(),
            span: Span::test(),
        };

        let selected = service
            .select_required_trait_method(&receiver, &receiver_ty, trait_id, "+")
            .expect("expected borrowed receiver method to match value receiver");

        let target = &selected.target;
        assert_eq!(target.impl_id(), Some(impl_id));
        assert_eq!(target.method_id(), Some(method_id));
        assert_eq!(selected.return_type, receiver_ty);
    }

    #[test]
    fn selection_service_prefers_user_index_impl_over_builtin_output() {
        let owner = def_id(50);
        let trait_id = def_id(51);
        let assoc_id = crate::ids::AssocTypeId(0);
        let impl_id = def_id(52);
        let method_id = def_id(53);
        let mut index_method = method(method_id, owner);
        index_method.name = "index".to_string();
        index_method.ret_type = Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        };
        index_method.params.push(HirParam {
            name: "index".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::I64,
            mutable: false,
            is_ref: false,
        });
        let mut traits = HashMap::from([(
            trait_id,
            crate::hir::HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Index".to_string(),
                generic_params: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: trait_id,
                        index: 0,
                    },
                    "Idx",
                )],
                associated_types: vec![crate::hir::HirAssociatedTypeDecl {
                    id: assoc_id,
                    name: "Output".to_string(),
                    kind: crate::type_services::kind::Kind::Type,
                }],
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        )]);
        let imp = HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Bag".to_string()),
            type_name: "Bag".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: owner,
                args: Vec::new(),
            }),
            trait_name: Some("Index".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "Idx",
            )],
            trait_arg_types: vec![Type::I64],
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("index".to_string(), index_method)]),
        };
        let index_member_id = register_trait_member(&mut traits, &imp, "index");
        let impls = HashMap::from([(imp.id, imp)]);
        let mut resolver = ResolverTables::default();
        resolver.item_names_by_id.insert(owner, "Bag".to_string());
        let current_impl_bounds = HirGenericBounds::new();
        let effective = HashMap::from([((impl_id, index_member_id), method_id)]);
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds)
            .with_effective_trait_methods(&effective);
        let receiver = HirExpr {
            kind: HirExprKind::Var("bag".to_string()),
            ty: Type::Struct {
                id: owner,
                args: Vec::new(),
            },
            span: Span::test(),
        };

        let selected = service
            .select_required_index_method(
                &[receiver_candidate(receiver)],
                trait_id,
                index_member_id,
                assoc_id,
                &Type::I64,
                "[]",
                |ty| ty.clone(),
            )
            .expect("expected user index impl");

        assert_eq!(selected.target.impl_id(), Some(impl_id));

        let authority = selected.authority();
        assert_eq!(authority.target, selected.target.target);
    }

    #[test]
    fn selection_records_unresolved_impl_bound_obligation() {
        let owner = def_id(80);
        let trait_id = def_id(81);
        let bound_trait_id = def_id(82);
        let impl_id = def_id(83);
        let method_id = def_id(84);
        let generic = GenericParamId {
            owner: impl_id,
            index: 0,
        };
        let unresolved = TypeVarId(0);
        let mut index_method = method(method_id, owner);
        index_method.name = "index".to_string();
        index_method.params[0].ty = Type::Struct {
            id: owner,
            args: vec![Type::Generic(generic)],
        };
        index_method.params.push(HirParam {
            name: "index".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::I64,
            mutable: false,
            is_ref: false,
        });
        let imp = HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: vec![GenericParamDecl::type_param(generic, "T")],
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: owner,
                args: vec![Type::Generic(generic)],
            }),
            trait_name: Some("Index".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: vec![Type::I64],
            associated_types: Vec::new(),
            bounds: HashMap::from([(
                generic,
                vec![TraitBound {
                    trait_id: bound_trait_id,
                    type_args: Vec::new(),
                }],
            )])
            .into(),
            methods: HashMap::from([("index".to_string(), index_method)]),
        };
        let mut traits = HashMap::new();
        let index_member_id = register_trait_member(&mut traits, &imp, "index");
        let impls = HashMap::from([(imp.id, imp)]);
        let mut resolver = ResolverTables::default();
        resolver.item_names_by_id.insert(owner, "Box".to_string());
        let current_impl_bounds = HirGenericBounds::new();
        let effective = HashMap::from([((impl_id, index_member_id), method_id)]);
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds)
            .with_effective_trait_methods(&effective);
        let receiver = HirExpr {
            kind: HirExprKind::Var("box".to_string()),
            ty: Type::Struct {
                id: owner,
                args: vec![Type::TypeVar(unresolved)],
            },
            span: Span::test(),
        };

        let selected = service
            .select_required_index_method(
                &[receiver_candidate(receiver)],
                trait_id,
                index_member_id,
                AssocTypeId(0),
                &Type::I64,
                "[]",
                |ty| ty.clone(),
            )
            .expect("unresolved bounded impl should remain selectable");

        assert_eq!(
            selected.pending_impl_bounds,
            vec![(
                Type::TypeVar(unresolved),
                TraitBound {
                    trait_id: bound_trait_id,
                    type_args: Vec::new(),
                },
            )]
        );
    }

    #[test]
    fn selection_authority_records_associated_output_return_type() {
        let owner = def_id(70);
        let trait_id = def_id(71);
        let impl_id = def_id(72);
        let method_id = def_id(73);
        let mut method = method(method_id, owner);
        method.name = "next".to_string();
        method.ret_type = Type::Projection {
            ty: Box::new(Type::Struct {
                id: owner,
                args: Vec::new(),
            }),
            trait_id,
            assoc_type: crate::types::AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id: crate::ids::AssocTypeId(0),
            },
            trait_args: Vec::new(),
        };
        let imp = HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("IteratorBox".to_string()),
            type_name: "IteratorBox".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: owner,
                args: Vec::new(),
            }),
            trait_name: Some("Iterator".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: vec![crate::hir::HirAssociatedTypeDef {
                id: crate::ids::AssocTypeId(0),
                name: "Item".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::I64,
            }],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("next".to_string(), method.clone())]),
        };
        let mut traits = HashMap::new();
        register_trait_member(&mut traits, &imp, "next");
        let impls = HashMap::from([(imp.id, imp)]);
        let mut resolver = ResolverTables::default();
        resolver
            .item_names_by_id
            .insert(owner, "IteratorBox".to_string());
        let current_impl_bounds = HirGenericBounds::new();
        let effective = effective_trait_methods_for(&traits, &impls);
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds)
            .with_effective_trait_methods(&effective);
        let receiver = HirExpr {
            kind: HirExprKind::Var("iter".to_string()),
            ty: Type::Struct {
                id: owner,
                args: Vec::new(),
            },
            span: Span::test(),
        };

        let authority = service
            .select_required_trait_method(&receiver, &receiver.ty, trait_id, "next")
            .expect("expected iterator selection")
            .authority();

        assert_eq!(authority.impl_id(), Some(impl_id));
        assert_eq!(authority.trait_id(), Some(trait_id));
        assert_eq!(authority.method_id(), Some(method_id));
        assert_eq!(authority.return_type, method.ret_type);
    }

    #[test]
    fn selection_service_rejects_index_impl_with_conflicting_generic_bindings() {
        let owner = def_id(60);
        let trait_id = def_id(61);
        let impl_id = def_id(62);
        let method_id = def_id(63);
        let generic = GenericParamId {
            owner: impl_id,
            index: 0,
        };
        let mut index_method = method(method_id, owner);
        index_method.name = "index".to_string();
        index_method.params[0].ty = Type::TypeVar(TypeVarId(0));
        index_method.params.push(HirParam {
            name: "index".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Generic(generic),
            mutable: false,
            is_ref: false,
        });
        let traits = HashMap::from([(
            trait_id,
            crate::hir::HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Index".to_string(),
                generic_params: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: trait_id,
                        index: 0,
                    },
                    "Idx",
                )],
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        )]);
        let imp = HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Bag".to_string()),
            type_name: "Bag".to_string(),
            type_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: impl_id,
                    index: 0,
                },
                "T",
            )],
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: owner,
                args: vec![Type::Generic(generic)],
            }),
            trait_name: Some("Index".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "T",
            )],
            trait_arg_types: vec![Type::Generic(generic)],
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("index".to_string(), index_method)]),
        };
        let impls = HashMap::from([(imp.id, imp)]);
        let mut resolver = ResolverTables::default();
        resolver.item_names_by_id.insert(owner, "Bag".to_string());
        let current_impl_bounds = HirGenericBounds::new();
        let service = SelectionService::new(&traits, &impls, None, None, &current_impl_bounds);
        let receiver = HirExpr {
            kind: HirExprKind::Var("bag".to_string()),
            ty: Type::Struct {
                id: owner,
                args: vec![Type::I64],
            },
            span: Span::test(),
        };

        assert!(service
            .select_required_index_method(
                &[receiver_candidate(receiver)],
                trait_id,
                method_id,
                AssocTypeId(0),
                &Type::Bool,
                "[]",
                |ty| { ty.clone() }
            )
            .is_err());
    }

    #[test]
    fn supertrait_bound_implies_parent_bound_by_trait_id() {
        let functor_id = def_id(200);
        let applicative_id = def_id(201);
        let caller_id = def_id(202);
        let unary = crate::type_services::kind::Kind::arrow(
            crate::type_services::kind::Kind::Type,
            crate::type_services::kind::Kind::Type,
        );
        let functor_target = GenericParamDecl::new(
            GenericParamId {
                owner: functor_id,
                index: 0,
            },
            "F",
            unary.clone(),
        );
        let applicative_target = GenericParamDecl::new(
            GenericParamId {
                owner: applicative_id,
                index: 0,
            },
            "F",
            unary,
        );
        let traits = HashMap::from([
            (
                functor_id,
                HirTrait {
                    id: functor_id,
                    name: "pkg::Functor".to_string(),
                    generic_params: Vec::new(),
                    target: Some(functor_target),
                    predicates: Vec::new(),
                    associated_types: Vec::new(),
                    methods: HashMap::new(),
                    signatures: HashMap::new(),
                },
            ),
            (
                applicative_id,
                HirTrait {
                    id: applicative_id,
                    name: "pkg::Applicative".to_string(),
                    generic_params: Vec::new(),
                    target: Some(applicative_target.clone()),
                    predicates: vec![crate::types::Predicate::Trait {
                        subject: Type::Generic(applicative_target.id),
                        trait_id: functor_id,
                        args: Vec::new(),
                    }],
                    associated_types: Vec::new(),
                    methods: HashMap::new(),
                    signatures: HashMap::new(),
                },
            ),
        ]);
        let caller_generic = GenericParamId {
            owner: caller_id,
            index: 0,
        };
        let current_bounds = HashMap::from([(
            caller_generic,
            vec![TraitBound {
                trait_id: applicative_id,
                type_args: Vec::new(),
            }],
        )])
        .into();
        let impls = HashMap::new();
        let service = SelectionService::new(&traits, &impls, None, None, &current_bounds);

        assert!(service.trait_bound_satisfied(
            &Type::Generic(caller_generic),
            &TraitBound {
                trait_id: functor_id,
                type_args: Vec::new(),
            },
            def_id(999),
        ));
    }

    #[test]
    fn constructor_trait_selection_preserves_result_section_substitution() {
        let trait_id = def_id(700);
        let member_id = def_id(701);
        let impl_id = def_id(702);
        let method_id = def_id(703);
        let result_id = def_id(704);
        let io_error_id = def_id(705);
        let impl_error = GenericParamDecl::type_param(
            GenericParamId {
                owner: impl_id,
                index: 0,
            },
            "E",
        );
        let unary = crate::type_services::kind::Kind::arrow(
            crate::type_services::kind::Kind::Type,
            crate::type_services::kind::Kind::Type,
        );
        let static_method = |id, name: &str| HirFunction {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: HirGenericBounds::new(),
            params: vec![HirParam {
                name: "value".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::I64,
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::I64,
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::I64,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };
        let trait_def = HirTrait {
            id: trait_id,
            name: "Applicative".to_string(),
            generic_params: Vec::new(),
            target: Some(GenericParamDecl::new(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "F",
                unary.clone(),
            )),
            predicates: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("pure".to_string(), static_method(member_id, "pure"))]),
            signatures: HashMap::new(),
        };
        let bound = || Type::BoundVar {
            depth: 0,
            index: 0,
            kind: crate::type_services::kind::Kind::Type,
        };
        let section = |error| Type::Lambda {
            params: vec![crate::type_services::kind::Kind::Type],
            body: Box::new(Type::Enum {
                id: result_id,
                args: vec![bound(), error],
            }),
        };
        let imp = HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("ResultSection".to_string()),
            type_name: "ResultSection".to_string(),
            type_generics: vec![impl_error.clone()],
            receiver_pattern: HirImplReceiverPattern::Constructor(section(Type::Generic(
                impl_error.id,
            ))),
            trait_name: Some("Applicative".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HirGenericBounds::new(),
            methods: HashMap::from([("pure".to_string(), static_method(method_id, "pure"))]),
        };
        let traits = HashMap::from([(trait_id, trait_def)]);
        let impls = HashMap::from([(impl_id, imp)]);
        let bounds = HirGenericBounds::new();
        let effective = effective_trait_methods_for(&traits, &impls);
        let service = SelectionService::new(&traits, &impls, None, None, &bounds)
            .with_effective_trait_methods(&effective);
        let io_error = Type::Struct {
            id: io_error_id,
            args: Vec::new(),
        };

        let selected = service
            .select_constructor_trait_member(&section(io_error.clone()), trait_id, &[], member_id)
            .expect("Result section should select its Applicative impl");

        assert_eq!(selected.target.impl_id(), Some(impl_id));
        assert_eq!(selected.target.trait_id(), Some(trait_id));
        assert_eq!(
            selected
                .target
                .owner_substitution
                .iter()
                .find(|binding| binding.param == impl_error.id)
                .map(|binding| &binding.ty),
            Some(&io_error),
        );

        let inferred = service
            .infer_constructor_target_for_applied_type(
                &Type::Enum {
                    id: result_id,
                    args: vec![Type::I64, io_error.clone()],
                },
                trait_id,
                &[],
            )
            .expect("constructor inference should not be ambiguous")
            .expect("Result application should infer its constructor section");
        assert_eq!(inferred, section(io_error));
    }

    #[test]
    fn constructor_trait_selection_reports_upstream_ambiguity() {
        let trait_id = def_id(710);
        let member_id = def_id(711);
        let option_id = def_id(712);
        let unary = crate::type_services::kind::Kind::arrow(
            crate::type_services::kind::Kind::Type,
            crate::type_services::kind::Kind::Type,
        );
        let function = |id| HirFunction {
            id,
            name: "pure".to_string(),
            generic_params: Vec::new(),
            generic_bounds: HirGenericBounds::new(),
            params: Vec::new(),
            ret_type: Type::Unit,
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::Unit,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };
        let trait_def = HirTrait {
            id: trait_id,
            name: "Applicative".to_string(),
            generic_params: Vec::new(),
            target: Some(GenericParamDecl::new(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "F",
                unary,
            )),
            predicates: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("pure".to_string(), function(member_id))]),
            signatures: HashMap::new(),
        };
        let target = Type::Constructor {
            id: option_id,
            flavor: crate::types::NominalTypeKind::Enum,
        };
        let impl_for = |impl_id: DefId, method_id: DefId| HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Option".to_string()),
            type_name: "Option".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Constructor(target.clone()),
            trait_name: Some("Applicative".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HirGenericBounds::new(),
            methods: HashMap::from([("pure".to_string(), function(method_id))]),
        };
        let traits = HashMap::from([(trait_id, trait_def)]);
        let impls = HashMap::from([
            (def_id(713), impl_for(def_id(713), def_id(714))),
            (def_id(715), impl_for(def_id(715), def_id(716))),
        ]);
        let bounds = HirGenericBounds::new();
        let effective = effective_trait_methods_for(&traits, &impls);
        let service = SelectionService::new(&traits, &impls, None, None, &bounds)
            .with_effective_trait_methods(&effective);

        let error = service
            .select_constructor_trait_member(&target, trait_id, &[], member_id)
            .expect_err("overlapping constructor impls must remain defensively ambiguous");

        assert!(error
            .message()
            .contains("upstream coherence invariant violation"));
    }
}

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
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
    active_obligations: RefCell<HashSet<(Type, TraitBound)>>,
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
            active_obligations: RefCell::new(HashSet::new()),
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
        if !function_has_receiver(method) {
            return Some(ReceiverAdjustment::None);
        }
        let self_param = method.params.first()?;
        let mut subst = impl_receiver_pattern_substitution(imp, receiver_ty, self.impls.values())?;
        let expected_self = Self::receiver_mode_expected_self_type(
            self_param.ty.substitute_generics(&subst),
            method.self_receiver,
        );
        Self::self_type_adjustment(
            &expected_self,
            receiver_ty,
            can_autoref_mut,
            self.impl_owner_is_reference_like(imp),
            imp.trait_id.is_some(),
            &mut subst,
        )
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
                if !self.trait_bound_satisfied(&subject, &bound) {
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
                if !self.trait_bound_satisfied(&bounded_ty, &bound) {
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

    pub(crate) fn trait_bound_satisfied(&self, ty: &Type, bound: &TraitBound) -> bool {
        if self.assumed_trait_bound_satisfied(ty, bound) {
            return true;
        }

        if self.is_builtin_sized_trait(bound.trait_id) {
            return self.type_is_sized(ty);
        }

        // The same impl may solve a structurally smaller obligation (for
        // example nested carriers); reject cycles in goals, not impl IDs.
        let goal = (ty.clone(), bound.clone());
        {
            let mut active = self.active_obligations.borrow_mut();
            if active.len() >= 128 || !active.insert(goal.clone()) {
                return false;
            }
        }
        let satisfied = self
            .impl_ids_in_order()
            .into_iter()
            .filter_map(|id| self.impls.get(&id))
            .any(|imp| {
                self.impl_matches_trait_ref_strict(imp, ty, bound.trait_id, Some(&bound.type_args))
            });
        self.active_obligations.borrow_mut().remove(&goal);
        satisfied
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
mod tests;

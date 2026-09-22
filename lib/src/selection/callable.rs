use crate::hir::HirLanguageItems;
use crate::selection::{ReceiverCandidate, SelectedMethod, SelectionDiagnostic, SelectionService};
use crate::types::{TraitBound, Type};

impl SelectionService<'_> {
    /// Select only marker-owned members, never a similarly named inherent method.
    pub fn select_callable_method(
        &self,
        candidates: &[ReceiverCandidate],
        items: &HirLanguageItems,
        bounds: impl Fn(&Type) -> Vec<TraitBound>,
        member_name: Option<&str>,
    ) -> Result<Option<SelectedMethod>, SelectionDiagnostic> {
        for (kind, trait_id, member_id) in items.callable_protocols() {
            let name = self.trait_member_name_by_id(trait_id, member_id).ok_or(
                SelectionDiagnostic::TraitMemberIdMissing {
                    trait_id,
                    member_id,
                },
            )?;
            if member_name.is_some_and(|requested| requested != name) {
                continue;
            }
            // Probe capability independently of mutability: an immutable FnMut
            // receiver must not silently fall back to consuming FnOnce.
            for candidate in candidates {
                let ty = &candidate.expr.ty;
                let mut available = self.trait_bounds_with_supertraits(ty, &bounds(ty));
                if let Some((params, ret, callable_kind, safety)) = native_signature(ty) {
                    if *callable_kind <= kind && *safety == crate::types::FunctionSafety::Safe {
                        available.push(TraitBound {
                            trait_id,
                            type_args: vec![callable_args_type(params), ret.clone()],
                        });
                    }
                }
                available.retain(|bound| bound.trait_id == trait_id);
                let mut probe = candidate.clone();
                probe.can_autoref_mut = true;
                let bound_selection = match self.select_bound_method(
                    std::slice::from_ref(&probe),
                    &available,
                    name,
                    ty.clone(),
                ) {
                    Ok(selected) => Some(selected),
                    Err(error @ SelectionDiagnostic::AmbiguousCandidates { .. }) => {
                        return Err(error)
                    }
                    Err(_) => None,
                };
                let selected = if let Some(selected) = bound_selection {
                    selected
                } else {
                    if matches!(
                        ty,
                        Type::TypeVar(_) | Type::Generic(_) | Type::Function { .. }
                    ) {
                        continue;
                    }
                    if let Type::Reference { inner, .. } = ty {
                        if matches!(inner.as_ref(), Type::TypeVar(_) | Type::Generic(_)) {
                            continue;
                        }
                    }
                    let mut selected = self.select_concrete_method_candidates(
                        std::slice::from_ref(&probe),
                        name,
                        Clone::clone,
                    );
                    selected.retain(|selected| {
                        (!matches!(ty, Type::Reference { .. })
                            || selected.pending_impl_bounds.is_empty())
                            && selected.impl_def.as_ref().is_none_or(|imp| {
                                match &imp.receiver_pattern {
                                    crate::hir::HirImplReceiverPattern::Exact(pattern) => {
                                        super::type_pattern_matches(
                                            pattern,
                                            ty,
                                            &mut std::collections::HashMap::new(),
                                        )
                                    }
                                    _ => true,
                                }
                            })
                            && selected.target.trait_id() == Some(trait_id)
                            && match &selected.target.target {
                                crate::hir::HirSelectedMethodTarget::ImplMethod {
                                    selected_trait: Some(member),
                                    ..
                                } => member.member_id == member_id,
                                _ => false,
                            }
                    });
                    if selected.len() > 1 {
                        return Err(SelectionDiagnostic::AmbiguousCandidates {
                            operation: "function call".into(),
                            receiver: ty.clone(),
                            candidates: selected
                                .iter()
                                .filter_map(|s| s.target.impl_id())
                                .collect(),
                        });
                    }
                    let Some(selected) = selected.pop() else {
                        continue;
                    };
                    selected
                };
                if kind == crate::types::CallableKind::FnMut && !candidate.can_autoref_mut {
                    return Err(SelectionDiagnostic::ReceiverMismatch {
                        operation: "mutable function call (requires a mutable receiver)".into(),
                        receiver: ty.clone(),
                    });
                }
                return Ok(Some(selected));
            }
        }
        Ok(None)
    }
}

fn native_signature(
    ty: &Type,
) -> Option<(
    &[Type],
    &Type,
    &crate::types::CallableKind,
    &crate::types::FunctionSafety,
)> {
    match ty {
        Type::Function {
            params,
            ret,
            callable_kind,
            safety,
            ..
        } => Some((params, ret, callable_kind, safety)),
        Type::Reference { mutable, inner } => {
            let signature = native_signature(inner)?;
            let allowed = if *mutable {
                crate::types::CallableKind::FnMut
            } else {
                crate::types::CallableKind::Fn
            };
            (*signature.2 <= allowed).then_some(signature)
        }
        _ => None,
    }
}

pub(crate) fn callable_args_type(params: &[Type]) -> Type {
    match params {
        [] => Type::Unit,
        [param] => param.clone(),
        params => Type::Tuple(params.to_vec()),
    }
}

use std::collections::HashMap;

use crate::hir::{HirEnum, HirImpl, HirImplReceiverPattern, HirStruct};
use crate::ids::DefId;
use crate::infer::constraints::ObligationState;
use crate::infer::InferenceEngine;
use crate::types::{TraitBound, Type};

use super::impl_lookup::{
    callable_impl_substitution, impl_bounds_satisfied, impl_exists_for, impl_receiver_owner_matches,
};
use super::type_predicates::contains_unresolved_type;
use super::BuiltinTraitIds;

pub(super) fn infer_explicit_impl_trait_args(
    engine: &mut InferenceEngine,
    ty: &Type,
    bound: &TraitBound,
    impls: &HashMap<DefId, HirImpl>,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    builtin_traits: BuiltinTraitIds,
) -> ObligationState {
    let mut candidates = Vec::new();
    let mut impl_ids = impls.keys().copied().collect::<Vec<_>>();
    impl_ids.sort();
    for id in impl_ids {
        let Some(imp) = impls.get(&id) else {
            continue;
        };
        if imp.trait_id != Some(bound.trait_id)
            || imp.trait_arg_types.len() != bound.type_args.len()
            || !impl_receiver_owner_matches(ty, imp, structs, enums)
        {
            continue;
        }
        let Some(mut subst) =
            callable_impl_substitution(imp, ty, bound.trait_id, builtin_traits, impls)
        else {
            continue;
        };
        for (expected, actual) in imp.trait_arg_types.iter().zip(&bound.type_args) {
            crate::selection::infer_generic_subst_from_types(
                expected,
                &engine.resolve(actual),
                &mut subst,
            );
        }
        let mut probe = engine.clone_for_probe();
        if imp
            .trait_arg_types
            .iter()
            .zip(&bound.type_args)
            .any(|(expected, actual)| {
                probe
                    .unify(actual, &expected.substitute_generics(&subst))
                    .is_err()
            })
        {
            continue;
        }
        if !impl_bounds_satisfied(imp, &subst, impls, structs, enums, builtin_traits) {
            continue;
        }
        let authority = (
            imp.id,
            bound
                .type_args
                .iter()
                .map(|arg| probe.resolve(arg))
                .collect::<Vec<_>>(),
        );
        if candidates
            .iter()
            .all(|(existing, _)| existing != &authority)
        {
            candidates.push((authority, probe));
        }
    }

    match candidates.as_slice() {
        [] => ObligationState::Pending,
        [(_, selected)] => {
            engine.commit_probe(selected.clone_for_commit());
            ObligationState::Solved
        }
        _ => ObligationState::Ambiguous,
    }
}

pub(super) fn infer_constructor_trait_obligation(
    engine: &mut InferenceEngine,
    ty: &Type,
    bound: &TraitBound,
    impls: &HashMap<DefId, HirImpl>,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    builtin_traits: BuiltinTraitIds,
    operation_span: &crate::lexer::Span,
) -> ObligationState {
    let self_ty @ Type::TypeVar(_) = engine.resolve(ty) else {
        return ObligationState::Solved;
    };
    select_unique_impl_by_unification(
        engine,
        &self_ty,
        bound,
        impls,
        structs,
        enums,
        builtin_traits,
        operation_span,
        UndecidedBounds::Wait,
    )
}

/// Improves a trait obligation whose Self type is a nominal type that still
/// contains inference variables, such as `Vec ?E: From [I64; 3]`.
///
/// Syntactic impl matching cannot see through those variables (`Vec I64`
/// never matches `Vec ?E`), so candidates are tested by unification instead;
/// a unique candidate is committed, which is what lets `x: Vec _ = arr.into!`
/// learn its element type from `impl From [T; N] for Vec T`. Its unification
/// is committed even while its own bounds still wait on inference, because no
/// other impl could discharge the obligation.
pub(super) fn infer_partially_known_self_obligation(
    engine: &mut InferenceEngine,
    ty: &Type,
    bound: &TraitBound,
    impls: &HashMap<DefId, HirImpl>,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    builtin_traits: BuiltinTraitIds,
    operation_span: &crate::lexer::Span,
) -> ObligationState {
    let self_ty = engine.resolve(ty);
    if !has_nominal_head(&self_ty) {
        return ObligationState::Pending;
    }
    select_unique_impl_by_unification(
        engine,
        &self_ty,
        bound,
        impls,
        structs,
        enums,
        builtin_traits,
        operation_span,
        UndecidedBounds::CommitUnique,
    )
}

/// Slice-family impls never target a nominal head, so skipping them below
/// cannot hide a candidate for such a Self type.
fn has_nominal_head(ty: &Type) -> bool {
    match ty {
        Type::Struct { .. } | Type::Enum { .. } | Type::Constructor { .. } => true,
        Type::Apply { constructor, .. } => has_nominal_head(constructor),
        _ => false,
    }
}

#[allow(clippy::too_many_arguments)]
fn select_unique_impl_by_unification(
    engine: &mut InferenceEngine,
    self_ty: &Type,
    bound: &TraitBound,
    impls: &HashMap<DefId, HirImpl>,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    builtin_traits: BuiltinTraitIds,
    operation_span: &crate::lexer::Span,
    undecided_bounds: UndecidedBounds,
) -> ObligationState {
    let mut candidates = Vec::new();
    let mut pending_candidate = false;
    let mut undecided_candidates = Vec::new();
    let mut impl_ids = impls.keys().copied().collect::<Vec<_>>();
    impl_ids.sort();
    for impl_id in impl_ids {
        let Some(imp) = impls.get(&impl_id) else {
            continue;
        };
        if imp.trait_id != Some(bound.trait_id) {
            continue;
        }
        let receiver = match &imp.receiver_pattern {
            HirImplReceiverPattern::Constructor(receiver)
            | HirImplReceiverPattern::Exact(receiver) => receiver,
            HirImplReceiverPattern::SliceFamily { .. } => continue,
        };
        let mut probe = engine.clone_for_probe();
        let subst = imp
            .type_generics
            .iter()
            .map(|param| {
                (
                    param.id,
                    probe.fresh_type_var_at_kind(operation_span.clone(), param.kind.clone()),
                )
            })
            .collect::<HashMap<_, _>>();
        let candidate = receiver.substitute_generics(&subst);
        if probe.unify(self_ty, &candidate).is_err() {
            continue;
        }
        if imp.trait_arg_types.len() != bound.type_args.len()
            || imp
                .trait_arg_types
                .iter()
                .zip(&bound.type_args)
                .any(|(expected, actual)| {
                    probe
                        .unify(actual, &expected.substitute_generics(&subst))
                        .is_err()
                })
        {
            continue;
        }
        let subst = subst
            .into_iter()
            .map(|(param, ty)| (param, probe.resolve(&ty)))
            .collect::<HashMap<_, _>>();
        if !impl_bounds_satisfied(imp, &subst, impls, structs, enums, builtin_traits) {
            // An unresolved impl parameter is not evidence against a candidate.
            // Wait for value/callback constraints instead of choosing a different
            // constructor merely because its bounds are already concrete.
            let undecided = imp.bounds.iter().all(|(param, bounds)| {
                let Some(ty) = subst.get(param) else {
                    return false;
                };
                bounds.iter().all(|bound| {
                    let args = bound
                        .type_args
                        .iter()
                        .map(|arg| arg.substitute_generics(&subst))
                        .collect::<Vec<_>>();
                    contains_unresolved_type(ty)
                        || args.iter().any(contains_unresolved_type)
                        || impl_exists_for(
                            ty,
                            bound.trait_id,
                            &args,
                            impls,
                            structs,
                            enums,
                            builtin_traits,
                        )
                })
            });
            if undecided {
                pending_candidate = true;
                if undecided_bounds == UndecidedBounds::CommitUnique {
                    undecided_candidates.push(probe);
                }
            }
            continue;
        }
        let trait_args = imp
            .trait_arg_types
            .iter()
            .map(|arg| probe.resolve(&arg.substitute_generics(&subst)))
            .collect::<Vec<_>>();
        let authority_key = (imp.id, trait_args);
        if candidates
            .iter()
            .all(|(existing, _)| existing != &authority_key)
        {
            candidates.push((authority_key, probe));
        }
    }
    if pending_candidate {
        // The only impl that can still apply fixes the obligation's shape even
        // though its bounds wait on inference; keep the obligation open so
        // those bounds are proven once they are decidable.
        if let ([], [selected]) = (candidates.as_slice(), undecided_candidates.as_slice()) {
            if refines(engine, selected, self_ty, bound) {
                engine.commit_probe(selected.clone_for_commit());
            }
        }
        return ObligationState::Pending;
    }
    match candidates.as_slice() {
        [] => ObligationState::Pending,
        [(_, selected)] => {
            engine.commit_probe(selected.clone_for_commit());
            ObligationState::Solved
        }
        _ => ObligationState::Ambiguous,
    }
}

/// Whether a candidate probe learned something about the obligation's own
/// types; committing a probe that only bound its fresh impl variables would
/// change the substitution generation without progress.
fn refines(
    engine: &InferenceEngine,
    probe: &InferenceEngine,
    self_ty: &Type,
    bound: &TraitBound,
) -> bool {
    std::iter::once(self_ty)
        .chain(&bound.type_args)
        .any(|ty| engine.resolve(ty) != probe.resolve(ty))
}

/// What a candidate whose impl bounds still wait on inference means.
#[derive(Clone, Copy, PartialEq, Eq)]
enum UndecidedBounds {
    /// Wait without committing anything.
    Wait,
    /// When it is the only impl that can apply, commit its unification.
    CommitUnique,
}

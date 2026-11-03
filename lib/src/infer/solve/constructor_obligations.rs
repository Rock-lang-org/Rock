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
    let Type::TypeVar(var) = engine.resolve(ty) else {
        return ObligationState::Solved;
    };
    let mut candidates = Vec::new();
    let mut pending_candidate = false;
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
        if probe.unify(&Type::TypeVar(var), &candidate).is_err() {
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
            pending_candidate |= imp.bounds.iter().all(|(param, bounds)| {
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

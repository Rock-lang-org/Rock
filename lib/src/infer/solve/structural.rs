use std::collections::{HashMap, HashSet, VecDeque};

use crate::hir::{HirEnum, HirImpl, HirStruct};
use crate::ids::DefId;
use crate::ids::TypeVarId;
use crate::infer::constraints::{Constraint, ConstraintOwner, ConstraintStore, ObligationState};
use crate::infer::{InferenceEngine, UnifyError};
use crate::types::Type;

use super::constructor_obligations::{
    infer_constructor_trait_obligation, infer_explicit_impl_trait_args,
};
use super::impl_lookup::callable_trait_satisfied;
use super::type_predicates::{contains_unresolved_type, try_head_is_pending};
use super::BuiltinTraitIds;

pub(super) fn solve_structural_constraints_to_fixed_point(
    engine: &mut InferenceEngine,
    store: &mut ConstraintStore,
    impls: &HashMap<DefId, HirImpl>,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    builtin_traits: BuiltinTraitIds,
    finalize_pending: bool,
    owners: Option<&HashSet<ConstraintOwner>>,
    changed_type_vars: &mut HashSet<TypeVarId>,
) {
    let mut queue = store.obligation_ids().collect::<VecDeque<_>>();
    let mut queued = queue.iter().copied().collect::<HashSet<_>>();

    while let Some(id) = queue.pop_front() {
        queued.remove(&id);
        if !store.includes_owner(id, owners) {
            continue;
        }
        if store.state(id) == Some(ObligationState::Ambiguous) {
            store.reopen_ambiguous(id);
        }
        if store.state(id) != Some(ObligationState::Pending) {
            continue;
        }
        let generation = engine.substitution_generation();
        if store.last_generation(id) == Some(generation) {
            continue;
        }
        let Some(constraint) = store.constraint(id).cloned() else {
            continue;
        };
        let state = match constraint {
            Constraint::Equality { left, right, .. } => {
                let mut probe = engine.clone_for_probe();
                match probe.unify(&left, &right) {
                    Ok(()) => {
                        *engine = probe;
                        if contains_unresolved_type(&engine.resolve(&left))
                            || contains_unresolved_type(&engine.resolve(&right))
                        {
                            ObligationState::Pending
                        } else {
                            ObligationState::Solved
                        }
                    }
                    Err(_) => {
                        if contains_unresolved_type(&engine.resolve(&left))
                            || contains_unresolved_type(&engine.resolve(&right))
                        {
                            ObligationState::Pending
                        } else {
                            ObligationState::Failed
                        }
                    }
                }
            }
            Constraint::Coercion {
                actual, expected, ..
            } => {
                let resolved_actual = engine.resolve(&actual);
                let resolved_expected = engine.resolve(&expected);
                if matches!(resolved_expected, Type::TypeVar(_)) {
                    ObligationState::Pending
                } else {
                    let mut probe = engine.clone_for_probe();
                    match unify_argument_coercion(&mut probe, &resolved_actual, &resolved_expected)
                    {
                        Ok(()) => {
                            *engine = probe;
                            if contains_unresolved_type(&engine.resolve(&actual))
                                || contains_unresolved_type(&engine.resolve(&expected))
                            {
                                ObligationState::Pending
                            } else {
                                ObligationState::Solved
                            }
                        }
                        Err(_) => {
                            if contains_unresolved_type(&resolved_actual)
                                || contains_unresolved_type(&resolved_expected)
                            {
                                ObligationState::Pending
                            } else {
                                ObligationState::Failed
                            }
                        }
                    }
                }
            }
            Constraint::Trait {
                ty, bound, span, ..
            } if matches!(engine.resolve(&ty), Type::TypeVar(_))
                && engine
                    .kind_of(&ty)
                    .map(|kind| kind != crate::type_services::kind::Kind::Type)
                    .unwrap_or(false) =>
            {
                infer_constructor_trait_obligation(
                    engine,
                    &ty,
                    &bound,
                    impls,
                    structs,
                    enums,
                    builtin_traits,
                    &span,
                )
            }
            Constraint::Trait { ty, bound, .. }
                if bound.type_args.len() == 2
                    && (builtin_traits.fn_once == Some(bound.trait_id)
                        || builtin_traits.fn_mut == Some(bound.trait_id)
                        || builtin_traits.fn_trait == Some(bound.trait_id)) =>
            {
                let resolved = engine.resolve(&ty);
                let (params, ret) = match resolved {
                    Type::Function { params, ret, .. } => (params, ret),
                    concrete => {
                        let state = infer_explicit_impl_trait_args(
                            engine,
                            &concrete,
                            &bound,
                            impls,
                            structs,
                            enums,
                            builtin_traits,
                        );
                        store.mark_attempt(id, engine.substitution_generation());
                        store.set_state(id, state);
                        continue;
                    }
                };
                let args = match params.as_slice() {
                    [] => Type::Unit,
                    [param] => param.clone(),
                    params => Type::Tuple(params.to_vec()),
                };
                let mut probe = engine.clone_for_probe();
                if probe.unify(&bound.type_args[0], &args).is_err()
                    || probe.unify(&bound.type_args[1], &ret).is_err()
                {
                    if contains_unresolved_type(&engine.resolve(&bound.type_args[0]))
                        || contains_unresolved_type(&engine.resolve(&bound.type_args[1]))
                    {
                        ObligationState::Pending
                    } else {
                        ObligationState::Failed
                    }
                } else {
                    *engine = probe;
                    // Once argument/result types unify, structural callability
                    // is known even if those types will later be generalized.
                    // Still check receiver capability and safety here.
                    let args = bound
                        .type_args
                        .iter()
                        .map(|ty| engine.resolve(ty))
                        .collect::<Vec<_>>();
                    match callable_trait_satisfied(
                        &engine.resolve(&ty),
                        bound.trait_id,
                        &args,
                        builtin_traits,
                    ) {
                        Some(true) => ObligationState::Solved,
                        Some(false) => ObligationState::Failed,
                        None => ObligationState::Pending,
                    }
                }
            }
            Constraint::Trait { ty, bound, .. } => {
                if contains_unresolved_type(&engine.resolve(&ty)) {
                    ObligationState::Pending
                } else if bound
                    .type_args
                    .iter()
                    .any(|arg| contains_unresolved_type(&engine.resolve(arg)))
                {
                    infer_explicit_impl_trait_args(
                        engine,
                        &engine.resolve(&ty),
                        &bound,
                        impls,
                        structs,
                        enums,
                        builtin_traits,
                    )
                } else {
                    ObligationState::Solved
                }
            }
            Constraint::IntLiteral { var, .. } | Constraint::FloatLiteral { var, .. } => {
                if matches!(engine.resolve(&Type::TypeVar(var)), Type::TypeVar(_)) {
                    ObligationState::Pending
                } else {
                    ObligationState::Solved
                }
            }
            Constraint::Try {
                carrier,
                residual,
                return_ty,
                ..
            } => {
                if [carrier, residual, return_ty]
                    .iter()
                    .any(|ty| try_head_is_pending(&engine.resolve(ty)))
                {
                    ObligationState::Pending
                } else {
                    ObligationState::Solved
                }
            }
        };
        store.mark_attempt(id, engine.substitution_generation());
        store.set_state(id, state);

        let changed = engine.take_changed_type_vars();
        changed_type_vars.extend(changed.iter().copied());
        for var in changed {
            for dependent in store.dependents_for(var).to_vec() {
                if !store.includes_owner(dependent, owners) {
                    continue;
                }
                if store.state(dependent) == Some(ObligationState::Ambiguous) {
                    store.reopen_ambiguous(dependent);
                }
                if store.state(dependent) == Some(ObligationState::Pending)
                    && queued.insert(dependent)
                {
                    queue.push_back(dependent);
                }
            }
        }
        store.refresh_dependencies(id, engine);
    }
    if finalize_pending {
        for id in store.obligation_ids().collect::<Vec<_>>() {
            if store.includes_owner(id, owners) && store.state(id) == Some(ObligationState::Pending)
            {
                store.set_state(id, ObligationState::Ambiguous);
            }
        }
    }
}

pub(super) fn unify_argument_coercion(
    engine: &mut InferenceEngine,
    actual: &Type,
    expected: &Type,
) -> Result<(), UnifyError> {
    if let (
        Type::Reference {
            mutable: actual_mutable,
            inner: actual_inner,
        },
        Type::Reference {
            mutable: expected_mutable,
            inner: expected_inner,
        },
    ) = (actual, expected)
    {
        if (!expected_mutable || *actual_mutable)
            && matches!(actual_inner.as_ref(), Type::Array(_, _))
            && matches!(expected_inner.as_ref(), Type::Slice(_))
        {
            let Type::Array(actual_element, _) = actual_inner.as_ref() else {
                unreachable!()
            };
            let Type::Slice(expected_element) = expected_inner.as_ref() else {
                unreachable!()
            };
            return engine.unify(actual_element, expected_element);
        }
    }

    engine.unify(actual, expected)
}

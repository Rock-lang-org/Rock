use std::collections::{HashMap, HashSet};

use crate::hir::{HirEnum, HirImpl, HirStruct};
use crate::ids::DefId;
use crate::ids::{Idx, TypeVarId};
use crate::infer::constraints::{Constraint, ConstraintOwner, ConstraintStore, ObligationState};
use crate::infer::InferenceEngine;
use crate::language_items::LanguageItems;
use crate::type_services::facts::TypeFacts;
use crate::type_services::normalize::TypeNormalizer;
use crate::types::{TraitBound, Type};

use super::impl_lookup::impl_exists_for;
use super::structural::{solve_structural_constraints_to_fixed_point, unify_argument_coercion};
use super::trait_bounds::trait_bounds_with_supertraits;
use super::type_predicates::{contains_inference_pending_type, contains_unresolved_type};
use super::{BuiltinTraitIds, SolveError, SolveErrorKind, SolveResult};

/// std implements the callable traits for `&F` when `F: Fn`, `FnOnce` and
/// `FnMut` for `&mut F` when `F: FnMut`, and `Fn` for `&mut F` when `F: Fn`.
/// A callable bound on a borrow of a free variable is therefore a bound on the
/// variable itself: that is the bound callers instantiate for their callback
/// and generalization exports.
fn borrowed_free_callable_subject(
    engine: &InferenceEngine,
    subject: Type,
    bound: TraitBound,
    builtin_traits: &BuiltinTraitIds,
) -> (Type, TraitBound) {
    let callable = [
        builtin_traits.fn_once,
        builtin_traits.fn_mut,
        builtin_traits.fn_trait,
    ]
    .contains(&Some(bound.trait_id));
    let Type::Reference { mutable, inner } = &subject else {
        return (subject, bound);
    };
    let inner = engine.resolve(inner);
    // `&mut F: Fn` needs `F: Fn`; weakening it to `FnMut` would accept a
    // closure that mutates its captures.
    let required = if *mutable && Some(bound.trait_id) != builtin_traits.fn_trait {
        builtin_traits.fn_mut
    } else {
        builtin_traits.fn_trait
    };
    match (callable, &inner, required) {
        (true, Type::TypeVar(_), Some(trait_id)) => (
            inner,
            TraitBound {
                trait_id,
                type_args: bound.type_args,
            },
        ),
        _ => (subject, bound),
    }
}

/// `Result String, ?e: Sync` holds exactly when `?e: Sync` does. For an auto
/// trait on a type that still contains free variables, prove it while assuming
/// the free variables and return those assumptions (with the trait each
/// position requires): they become bounds of the variables, like a direct
/// `?v: Sync`, and where clauses once the owner is generalized.
///
/// When the proof fails even with every free variable assumed, no solution
/// of those variables can make it hold: `Result Raw, ?e: Send` fails because
/// of `Raw`. That is reported as a violation, not as an ambiguity about `?e`.
/// An application with an unsolved head (`?f I64`) has no structure to
/// prove yet, so it stays undecided.
enum AutoTraitResidual {
    NotApplicable,
    Holds(Vec<(TypeVarId, DefId)>),
    Fails,
}

fn auto_trait_residual_bounds(
    subject: &Type,
    bound: &TraitBound,
    impls: &HashMap<DefId, HirImpl>,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    builtin_traits: BuiltinTraitIds,
    assumed: &dyn Fn(&Type, DefId, &[Type]) -> bool,
) -> AutoTraitResidual {
    let auto_trait = [builtin_traits.send, builtin_traits.sync, builtin_traits.owned]
        .contains(&Some(bound.trait_id));
    if !auto_trait
        || !bound.type_args.is_empty()
        || matches!(subject, Type::TypeVar(_))
        || !crate::type_services::visit::type_any(subject, |ty| matches!(ty, Type::TypeVar(_)))
        || crate::type_services::visit::type_any(subject, |ty| {
            matches!(ty, Type::Projection { .. } | Type::Error)
        })
    {
        return AutoTraitResidual::NotApplicable;
    }
    let residual = std::cell::RefCell::new(Vec::new());
    let assume_free = |ty: &Type, trait_id: DefId, args: &[Type]| {
        if assumed(ty, trait_id, args) {
            return true;
        }
        match ty {
            Type::TypeVar(var) if args.is_empty() => {
                if !residual.borrow().contains(&(*var, trait_id)) {
                    residual.borrow_mut().push((*var, trait_id));
                }
                true
            }
            _ => false,
        }
    };
    let satisfied = super::auto_traits::auto_trait_satisfied(
        subject,
        bound.trait_id,
        impls,
        structs,
        enums,
        builtin_traits,
        &mut HashSet::new(),
        &assume_free,
    );
    if satisfied {
        return AutoTraitResidual::Holds(residual.into_inner());
    }
    let unsolved_head = crate::type_services::visit::type_any(subject, |ty| {
        matches!(ty, Type::Apply { constructor, .. } if matches!(constructor.as_ref(), Type::TypeVar(_)))
    });
    if unsolved_head {
        AutoTraitResidual::NotApplicable
    } else {
        AutoTraitResidual::Fails
    }
}

pub(super) fn solve_constraints_in_place_mode(
    engine: &mut InferenceEngine,
    store: &mut ConstraintStore,
    impls: &HashMap<DefId, HirImpl>,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    traits: &HashMap<DefId, crate::hir::HirTrait>,
    language_items: &LanguageItems<DefId>,
    finalize_pending: bool,
    owners: Option<&HashSet<ConstraintOwner>>,
    assumptions: &crate::hir::HirGenericBounds,
    // Free constructor-kinded variables that generalization will abstract
    // (`F` in an inferred `F A -> ...` signature); others stay errors.
    generalizable: &HashSet<TypeVarId>,
) -> SolveResult {
    let builtin_traits = BuiltinTraitIds::from_language_items(language_items);
    // Structural proofs and conditional impl bounds must retain the same
    // declaration evidence as top-level trait selection.
    let assumed = |ty: &Type, trait_id: DefId, args: &[Type]| {
        let declared_predicate = assumptions.predicates.iter().any(|predicate| {
            let crate::types::Predicate::Trait {
                subject,
                trait_id: predicate_trait,
                args: predicate_args,
            } = predicate;
            *predicate_trait == trait_id && subject == ty && predicate_args.as_slice() == args
        });
        if declared_predicate {
            return true;
        }
        let Type::Generic(param) = ty else {
            return false;
        };
        let required = TraitBound {
            trait_id,
            type_args: args.to_vec(),
        };
        assumptions.get(param).is_some_and(|bounds| {
            bounds.iter().any(|bound| {
                trait_bounds_with_supertraits(ty, bound, traits).contains(&required)
            })
        })
    };
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    let mut generic_bounds: HashMap<TypeVarId, Vec<TraitBound>> = HashMap::new();
    let mut changed_type_vars = HashSet::new();
    let mut deferred_predicates = Vec::new();

    solve_structural_constraints_to_fixed_point(
        engine,
        store,
        impls,
        structs,
        enums,
        builtin_traits,
        finalize_pending,
        owners,
        &mut changed_type_vars,
        &assumed,
    );

    // Validate equality failures only after structural propagation reaches
    // quiescence, so temporarily unresolved constructor equations can settle.
    for (id, constraint) in store
        .iter()
        .map(|(id, value)| (id, value.clone()))
        .collect::<Vec<_>>()
    {
        if !store.includes_owner(id, owners) {
            continue;
        }
        let Constraint::Equality {
            left,
            right,
            span,
            context,
        } = constraint
        else {
            continue;
        };
        let resolved_left = engine.resolve(&left);
        let resolved_right = engine.resolve(&right);
        let mut probe = engine.clone_for_probe();
        if let Err(error) = probe.unify(&resolved_left, &resolved_right) {
            if !contains_unresolved_type(&resolved_left)
                && !contains_unresolved_type(&resolved_right)
            {
                errors.push(SolveError {
                    kind: SolveErrorKind::Unify { context, error },
                    span,
                });
                store.set_state(id, ObligationState::Failed);
            }
        } else {
            store.set_state(
                id,
                if contains_unresolved_type(&probe.resolve(&left))
                    || contains_unresolved_type(&probe.resolve(&right))
                {
                    ObligationState::Pending
                } else {
                    ObligationState::Solved
                },
            );
        }
    }

    for (id, constraint) in store
        .iter()
        .map(|(id, value)| (id, value.clone()))
        .collect::<Vec<_>>()
    {
        if !store.includes_owner(id, owners) {
            continue;
        }
        let Constraint::Coercion {
            actual,
            expected,
            span,
            context,
        } = constraint
        else {
            continue;
        };
        let resolved_actual = engine.resolve(&actual);
        let resolved_expected = engine.resolve(&expected);
        if matches!(resolved_expected, Type::TypeVar(_)) {
            continue;
        }
        let mut probe = engine.clone_for_probe();
        if let Err(error) =
            unify_argument_coercion(&mut probe, &resolved_actual, &resolved_expected)
        {
            if !contains_unresolved_type(&resolved_actual)
                && !contains_unresolved_type(&resolved_expected)
            {
                errors.push(SolveError {
                    kind: SolveErrorKind::Unify { context, error },
                    span,
                });
                store.set_state(id, ObligationState::Failed);
            }
        }
    }

    for (id, constraint) in store
        .iter()
        .map(|(id, value)| (id, value.clone()))
        .collect::<Vec<_>>()
    {
        if !store.includes_owner(id, owners) {
            continue;
        }
        match constraint {
            Constraint::Trait {
                ty,
                bound,
                span,
                context,
            } => {
                let normalization_env = engine.normalization_env();
                let resolved_ty = engine.resolve(&ty);
                let resolved = TypeNormalizer::new(&normalization_env)
                    .normalize(&resolved_ty)
                    .unwrap_or(resolved_ty);
                let (resolved, bound) = borrowed_free_callable_subject(
                    engine,
                    resolved,
                    bound,
                    &builtin_traits,
                );
                if finalize_pending {
                    match auto_trait_residual_bounds(
                        &resolved,
                        &bound,
                        impls,
                        structs,
                        enums,
                        builtin_traits,
                        &assumed,
                    ) {
                        AutoTraitResidual::Holds(residual) => {
                            for (var, trait_id) in residual {
                                generic_bounds.entry(var).or_default().push(TraitBound {
                                    trait_id,
                                    type_args: Vec::new(),
                                });
                            }
                            store.set_state(id, ObligationState::Solved);
                            continue;
                        }
                        AutoTraitResidual::Fails => {
                            let trait_name = traits
                                .get(&bound.trait_id)
                                .map(|trait_def| trait_def.name.as_str())
                                .unwrap_or("<unknown trait>");
                            errors.push(SolveError {
                                kind: SolveErrorKind::TraitNotImplemented {
                                    ty: resolved.clone(),
                                    trait_name: trait_name.to_string(),
                                    trait_args: Vec::new(),
                                    context: context.clone(),
                                },
                                span,
                            });
                            store.set_state(id, ObligationState::Failed);
                            continue;
                        }
                        AutoTraitResidual::NotApplicable => {}
                    }
                }
                if !matches!(resolved, Type::TypeVar(_) | Type::Generic(_))
                    && contains_inference_pending_type(&resolved)
                {
                    continue;
                }
                let resolved_bound = TraitBound {
                    trait_id: bound.trait_id,
                    type_args: bound
                        .type_args
                        .iter()
                        .map(|arg| {
                            let resolved = engine.resolve(&arg);
                            TypeNormalizer::new(&normalization_env)
                                .normalize(&resolved)
                                .unwrap_or(resolved)
                        })
                        .collect(),
                };
                match &resolved {
                    Type::TypeVar(type_var) => {
                        // Still a free variable — will become a Generic param.
                        generic_bounds.entry(*type_var).or_default().extend(
                            trait_bounds_with_supertraits(&resolved, &resolved_bound, traits),
                        );
                        if finalize_pending
                            && (engine.kind_of_type_var(*type_var)
                                == crate::type_services::kind::Kind::Type
                                || generalizable.contains(type_var))
                        {
                            store.set_state(id, ObligationState::Solved);
                        }
                    }
                    Type::Generic(_) => {
                        // Already a generic parameter — bound satisfied at mono time.
                        if finalize_pending {
                            store.set_state(id, ObligationState::Solved);
                        }
                    }
                    concrete_ty => {
                        if context == "try operator"
                            && language_items
                                .try_protocol
                                .as_ref()
                                .is_some_and(|protocol| bound.trait_id == protocol.try_trait_id)
                            && contains_inference_pending_type(concrete_ty)
                        {
                            continue;
                        }
                        let generic_impl =
                            crate::type_services::visit::type_any(concrete_ty, |ty| {
                                matches!(ty, Type::Generic(_))
                            }) && crate::selection::SelectionService::new(
                                traits,
                                impls,
                                builtin_traits.sized,
                                None,
                                assumptions,
                            )
                            .select_trait_impl_strict(
                                concrete_ty,
                                resolved_bound.trait_id,
                                &resolved_bound.type_args,
                            )
                            .is_ok();
                        let impl_found = generic_impl
                            || impl_exists_for(
                                concrete_ty,
                                resolved_bound.trait_id,
                                &resolved_bound.type_args,
                                impls,
                                structs,
                                enums,
                                builtin_traits,
                                &assumed,
                            );
                        // `String: From ?x` with `?x` still free is not a violation yet:
                        // it may become a where clause of the owning function when the
                        // function is generalized.
                        if !impl_found
                            && resolved_bound.type_args.iter().any(contains_unresolved_type)
                        {
                            if finalize_pending {
                                deferred_predicates.push(id);
                            }
                            continue;
                        }
                        if !impl_found {
                            if context == "try operator"
                                && language_items
                                    .try_protocol
                                    .as_ref()
                                    .is_some_and(|protocol| bound.trait_id == protocol.try_trait_id)
                            {
                                errors.push(SolveError {
                                    kind: SolveErrorKind::TryNonCarrier {
                                        ty: concrete_ty.clone(),
                                    },
                                    span,
                                });
                                continue;
                            }
                            let trait_name = traits
                                .get(&resolved_bound.trait_id)
                                .map(|trait_def| trait_def.name.as_str())
                                .unwrap_or("<unknown trait>");
                            errors.push(SolveError {
                                kind: SolveErrorKind::TraitNotImplemented {
                                    ty: concrete_ty.clone(),
                                    trait_name: trait_name.to_string(),
                                    trait_args: resolved_bound.type_args.clone(),
                                    context: context.clone(),
                                },
                                span,
                            });
                            store.set_state(id, ObligationState::Failed);
                        } else if finalize_pending {
                            store.set_state(id, ObligationState::Solved);
                        }
                    }
                }
            }

            // Integer literal constraints: unresolved literals default later, but a
            // resolved literal must still be an integer type.
            Constraint::IntLiteral { var, span } => {
                let resolved = engine.resolve(&Type::TypeVar(var));
                match &resolved {
                    Type::TypeVar(_) => {}
                    Type::Generic(_) | Type::Projection { .. } => {}
                    ty if TypeFacts::is_integer(ty) => {}
                    other => {
                        errors.push(SolveError {
                            kind: SolveErrorKind::IntegerLiteral {
                                found: other.clone(),
                            },
                            span,
                        });
                        store.set_state(id, ObligationState::Failed);
                    }
                }
            }

            Constraint::FloatLiteral { var, span: _ } => {
                let resolved = engine.resolve(&Type::TypeVar(var));
                match &resolved {
                    Type::TypeVar(_) => {
                        warnings.push(format!(
                            "warning: type of float literal is ambiguous (TypeVar {}); \
                             add a type annotation such as `: F64`",
                            var.raw()
                        ));
                    }
                    ty if TypeFacts::is_float(ty) => {}
                    other => {
                        warnings.push(format!(
                            "warning: float literal resolved to non-float type `{}`",
                            engine.display_type(other)
                        ));
                    }
                }
            }
            Constraint::Equality { .. } | Constraint::Coercion { .. } | Constraint::Try { .. } => {}
        }
    }

    if finalize_pending {
        for id in store.obligation_ids().collect::<Vec<_>>() {
            if !store.includes_owner(id, owners)
                || store.state(id) != Some(ObligationState::Ambiguous)
                || deferred_predicates.contains(&id)
            {
                continue;
            }
            let Some(constraint) = store.constraint(id) else {
                continue;
            };
            let context = match constraint {
                Constraint::Equality { context, .. } | Constraint::Trait { context, .. } => {
                    context.clone()
                }
                Constraint::Coercion {
                    actual,
                    expected,
                    context,
                    ..
                } => format!(
                    "{context}: {} -> {}",
                    engine.resolve(actual),
                    engine.resolve(expected)
                ),
                Constraint::Try { .. } => "try operator".to_string(),
                Constraint::IntLiteral { .. } | Constraint::FloatLiteral { .. } => continue,
            };
            let span = match constraint {
                Constraint::Trait { span, .. }
                | Constraint::IntLiteral { span, .. }
                | Constraint::FloatLiteral { span, .. }
                | Constraint::Equality { span, .. }
                | Constraint::Coercion { span, .. }
                | Constraint::Try { span, .. } => span.clone(),
            };
            errors.push(SolveError {
                kind: SolveErrorKind::Ambiguous { context },
                span,
            });
        }
    }

    for (representative, bounds) in &generic_bounds {
        for bound in bounds {
            if !engine.get_bounds(*representative).contains(bound) {
                engine.add_bound(*representative, bound.clone());
            }
        }
    }

    let mut changed_type_vars = changed_type_vars.into_iter().collect::<Vec<_>>();
    changed_type_vars.sort_by_key(|id| id.raw());
    SolveResult {
        warnings,
        errors,
        generic_bounds,
        changed_type_vars,
        deferred_predicates,
    }
}

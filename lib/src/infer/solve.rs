//! Phase 4 constraint solver: discharge accumulated constraints after lowering.
//!
//! This runs between generalization and finalization. It:
//!   - Verifies trait bounds: if a type variable resolved to a concrete type, checks
//!     that the type has an impl for the required trait.
//!   - Checks integer/float literal constraints: if a TypeVar from a literal still
//!     has no concrete type, emits an ambiguity warning.
//!
//! Trait bound violations on concrete types are hard errors.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::hir::{HirEnum, HirImpl, HirImplReceiverPattern, HirStruct, HirVariantFields};
use crate::ids::DefId;
use crate::ids::{Idx, TypeVarId};
use crate::infer::constraints::{Constraint, ConstraintOwner, ConstraintStore, ObligationState};
use crate::infer::{InferenceEngine, UnifyError};
use crate::language_items::LanguageItems;
use crate::type_services::facts::TypeFacts;
use crate::type_services::normalize::TypeNormalizer;
use crate::types::{CallableKind, CaptureKind, GenericParamId, TraitBound, Type};

#[derive(Debug, Clone)]
pub enum SolveErrorKind {
    Unify {
        context: String,
        error: UnifyError,
    },
    TraitNotImplemented {
        ty: Type,
        trait_name: String,
        context: String,
    },
    TryNonCarrier {
        ty: Type,
    },
    IntegerLiteral {
        found: Type,
    },
    Message(String),
}

#[derive(Debug, Clone)]
pub struct SolveError {
    pub kind: SolveErrorKind,
    pub span: crate::lexer::Span,
}

impl SolveError {
    pub fn contains(&self, pattern: &str) -> bool {
        self.message().contains(pattern)
    }

    pub fn message(&self) -> String {
        self.render_with(|ty| ty.to_string())
    }

    pub fn render(&self, engine: &InferenceEngine) -> String {
        self.render_with(|ty| engine.display_type(ty))
    }

    fn render_with(&self, display: impl Fn(&Type) -> String) -> String {
        match &self.kind {
            SolveErrorKind::Unify { context, error } => {
                format!("{context}: {}", error.render_with(&display))
            }
            SolveErrorKind::TraitNotImplemented {
                ty,
                trait_name,
                context,
            } => format!(
                "type `{}` does not implement trait `{}` (required by {})",
                display(ty),
                trait_name,
                context
            ),
            SolveErrorKind::TryNonCarrier { ty } => {
                format!("Cannot use '?' on non-carrier type {}", display(ty))
            }
            SolveErrorKind::IntegerLiteral { found } => format!(
                "integer literal resolved to non-integer type `{}`",
                display(found)
            ),
            SolveErrorKind::Message(message) => message.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct BuiltinTraitIds {
    sized: Option<DefId>,
    fn_once: Option<DefId>,
    fn_mut: Option<DefId>,
    fn_trait: Option<DefId>,
    send: Option<DefId>,
    sync: Option<DefId>,
}

impl BuiltinTraitIds {
    fn from_language_items(items: &LanguageItems<DefId>) -> Self {
        Self {
            sized: items.sized.as_ref().map(|items| items.trait_id),
            fn_once: items.fn_once.as_ref().map(|items| items.trait_id),
            fn_mut: items.fn_mut.as_ref().map(|items| items.trait_id),
            fn_trait: items.fn_trait.as_ref().map(|items| items.trait_id),
            send: items.send.as_ref().map(|items| items.trait_id),
            sync: items.sync.as_ref().map(|items| items.trait_id),
        }
    }
}

/// Result of constraint solving.
pub struct SolveResult {
    /// Warnings (non-fatal: ambiguous literals that were defaulted).
    pub warnings: Vec<String>,
    /// Hard errors: concrete type violates a required trait bound.
    pub errors: Vec<SolveError>,
    /// TypeVar ids that remain free but have at least one trait bound attached.
    pub generic_bounds: HashMap<TypeVarId, Vec<TraitBound>>,
    /// Representatives changed while this worklist invocation was running.
    pub changed_type_vars: Vec<TypeVarId>,
}

/// Solve all constraints in `store`.
///
/// `impls` is the list of all known trait implementations.
/// Trait violations on concrete types are always hard errors.
pub fn solve_constraints(
    engine: &mut InferenceEngine,
    store: &ConstraintStore,
    impls: &HashMap<DefId, HirImpl>,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    traits: &HashMap<DefId, crate::hir::HirTrait>,
    language_items: &LanguageItems<DefId>,
) -> SolveResult {
    let mut store = store.clone();
    solve_constraints_in_place(
        engine,
        &mut store,
        impls,
        structs,
        enums,
        traits,
        language_items,
    )
}

pub fn solve_constraints_in_place(
    engine: &mut InferenceEngine,
    store: &mut ConstraintStore,
    impls: &HashMap<DefId, HirImpl>,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    traits: &HashMap<DefId, crate::hir::HirTrait>,
    language_items: &LanguageItems<DefId>,
) -> SolveResult {
    solve_constraints_in_place_mode(
        engine,
        store,
        impls,
        structs,
        enums,
        traits,
        language_items,
        true,
        None,
        &crate::hir::HirGenericBounds::new(),
    )
}

pub(crate) fn solve_constraints_in_place_for_owners(
    engine: &mut InferenceEngine,
    store: &mut ConstraintStore,
    impls: &HashMap<DefId, HirImpl>,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    traits: &HashMap<DefId, crate::hir::HirTrait>,
    language_items: &LanguageItems<DefId>,
    owners: &HashSet<ConstraintOwner>,
    finalize_pending: bool,
    assumptions: &crate::hir::HirGenericBounds,
) -> SolveResult {
    solve_constraints_in_place_mode(
        engine,
        store,
        impls,
        structs,
        enums,
        traits,
        language_items,
        finalize_pending,
        Some(owners),
        assumptions,
    )
}

fn solve_constraints_in_place_mode(
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
) -> SolveResult {
    let builtin_traits = BuiltinTraitIds::from_language_items(language_items);
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    let mut generic_bounds: HashMap<TypeVarId, Vec<TraitBound>> = HashMap::new();
    let mut changed_type_vars = HashSet::new();

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
                            && engine.kind_of_type_var(*type_var)
                                == crate::type_services::kind::Kind::Type
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
                        if !generic_impl
                            && !impl_exists_for(
                                concrete_ty,
                                resolved_bound.trait_id,
                                &resolved_bound.type_args,
                                impls,
                                structs,
                                enums,
                                builtin_traits,
                            )
                        {
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
                kind: SolveErrorKind::Message(format!(
                    "ambiguous constraint at quiescence: {context}; add type information to select one canonical solution"
                )),
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
    }
}

fn solve_structural_constraints_to_fixed_point(
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
                    if contains_unresolved_type(&engine.resolve(&bound.type_args[0]))
                        || contains_unresolved_type(&engine.resolve(&bound.type_args[1]))
                    {
                        ObligationState::Pending
                    } else {
                        ObligationState::Solved
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

fn unify_argument_coercion(
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

fn try_head_is_pending(ty: &Type) -> bool {
    match ty {
        Type::TypeVar(_) | Type::Projection { .. } | Type::Error => true,
        Type::Apply { constructor, .. } => try_head_is_pending(constructor),
        _ => false,
    }
}

fn infer_explicit_impl_trait_args(
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

fn infer_constructor_trait_obligation(
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

fn contains_unresolved_type(ty: &Type) -> bool {
    crate::type_services::visit::type_any(ty, |nested| {
        matches!(
            nested,
            Type::TypeVar(_)
                | Type::Generic(_)
                | Type::Projection { .. }
                | Type::BoundVar { .. }
                | Type::Error
        )
    })
}

fn contains_inference_pending_type(ty: &Type) -> bool {
    crate::type_services::visit::type_any(ty, |nested| match nested {
        Type::TypeVar(_) | Type::Projection { .. } | Type::Error => true,
        Type::Apply { constructor, args } => {
            contains_inference_pending_type(constructor)
                || args.iter().any(contains_inference_pending_type)
        }
        _ => false,
    })
}

fn trait_bounds_with_supertraits(
    subject: &Type,
    initial: &TraitBound,
    traits: &HashMap<DefId, crate::hir::HirTrait>,
) -> Vec<TraitBound> {
    fn expand(
        subject: &Type,
        bound: TraitBound,
        traits: &HashMap<DefId, crate::hir::HirTrait>,
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
        if let Some(trait_def) = traits.get(&bound.trait_id) {
            let mut subst = crate::selection::generic_substitution_for_owner(
                &trait_def.generic_params,
                &bound.type_args,
            );
            let target_id = trait_def.target.as_ref().map(|target| target.id).unwrap_or(
                crate::types::GenericParamId {
                    owner: trait_def.id,
                    index: trait_def.generic_params.len() as u32,
                },
            );
            subst.insert(target_id, subject.clone());
            for predicate in &trait_def.predicates {
                let crate::types::Predicate::Trait {
                    subject: implied_subject,
                    trait_id,
                    args,
                } = predicate.substitute_generics(&subst);
                if implied_subject == *subject {
                    expand(
                        subject,
                        TraitBound {
                            trait_id,
                            type_args: args,
                        },
                        traits,
                        visiting,
                        output,
                    );
                }
            }
        }
        visiting.remove(&key);
    }

    let mut output = Vec::new();
    expand(
        subject,
        initial.clone(),
        traits,
        &mut std::collections::HashSet::new(),
        &mut output,
    );
    output
}

/// Check whether `ty` has an impl of the canonical trait ID in the given impl list.
///
/// Matches on the `type_name` field of `HirImpl`. Generic impls (with
/// `type_generics` non-empty, e.g. `impl<T> Show for Vec<T>`) are treated as
/// matching any type of the right shape.
fn impl_exists_for(
    ty: &Type,
    trait_id: DefId,
    trait_args: &[Type],
    impls: &HashMap<DefId, HirImpl>,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    builtin_traits: BuiltinTraitIds,
) -> bool {
    if builtin_traits.sized == Some(trait_id) {
        return type_is_sized(ty);
    }

    if let Some(satisfied) = callable_trait_satisfied(ty, trait_id, trait_args, builtin_traits) {
        return satisfied;
    }

    if explicit_impl_exists_for(
        ty,
        trait_id,
        trait_args,
        impls,
        structs,
        enums,
        builtin_traits,
    ) {
        return true;
    }

    auto_trait_satisfied(
        ty,
        trait_id,
        impls,
        structs,
        enums,
        builtin_traits,
        &mut std::collections::HashSet::new(),
    )
}

fn explicit_impl_exists_for(
    ty: &Type,
    trait_id: DefId,
    trait_args: &[Type],
    impls: &HashMap<DefId, HirImpl>,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    builtin_traits: BuiltinTraitIds,
) -> bool {
    let marker_provider_crate =
        if builtin_traits.send == Some(trait_id) || builtin_traits.sync == Some(trait_id) {
            Some(trait_id.crate_id)
        } else {
            None
        };
    let mut impl_ids = impls.keys().copied().collect::<Vec<_>>();
    impl_ids.sort();
    impl_ids.into_iter().any(|id| {
        if marker_provider_crate.is_some_and(|provider| id.crate_id != provider) {
            return false;
        }
        let Some(imp) = impls.get(&id) else {
            return false;
        };
        if imp.trait_id != Some(trait_id) || !impl_receiver_owner_matches(ty, imp, structs, enums) {
            return false;
        }

        let Some(mut subst) = callable_impl_substitution(imp, ty, trait_id, builtin_traits, impls)
        else {
            return false;
        };

        impl_trait_args_match(&imp.trait_arg_types, trait_args, &mut subst)
            && impl_bounds_satisfied(imp, &subst, impls, structs, enums, builtin_traits)
    })
}

fn impl_bounds_satisfied(
    imp: &HirImpl,
    subst: &HashMap<crate::types::GenericParamId, Type>,
    impls: &HashMap<DefId, HirImpl>,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    builtin_traits: BuiltinTraitIds,
) -> bool {
    imp.bounds.iter().all(|(generic_param, bounds)| {
        let Some(ty) = subst.get(generic_param) else {
            return false;
        };

        bounds.iter().all(|bound| {
            let type_args: Vec<Type> = bound
                .type_args
                .iter()
                .map(|arg| arg.substitute_generics(subst))
                .collect();
            impl_exists_for(
                ty,
                bound.trait_id,
                &type_args,
                impls,
                structs,
                enums,
                builtin_traits,
            )
        })
    })
}

fn callable_trait_satisfied(
    ty: &Type,
    trait_id: DefId,
    trait_args: &[Type],
    builtin_traits: BuiltinTraitIds,
) -> Option<bool> {
    let required_kind = if builtin_traits.fn_trait == Some(trait_id) {
        CallableKind::Fn
    } else if builtin_traits.fn_mut == Some(trait_id) {
        CallableKind::FnMut
    } else if builtin_traits.fn_once == Some(trait_id) {
        CallableKind::FnOnce
    } else {
        return None;
    };
    let Type::Function {
        params,
        ret,
        callable_kind,
        safety,
        ..
    } = ty
    else {
        return None;
    };
    if trait_args.len() != 2
        || *callable_kind > required_kind
        || *safety == crate::types::FunctionSafety::Unsafe
    {
        return Some(false);
    }
    let args = match params.as_slice() {
        [] => Type::Unit,
        [param] => param.clone(),
        params => Type::Tuple(params.to_vec()),
    };
    let args_match = trait_args[0] == args
        || matches!((&trait_args[0], &args), (Type::Tuple(elements), Type::Unit) if elements.is_empty());
    Some(args_match && trait_args[1] == **ret)
}

fn callable_impl_substitution(
    imp: &HirImpl,
    ty: &Type,
    trait_id: DefId,
    builtin: BuiltinTraitIds,
    impls: &HashMap<DefId, HirImpl>,
) -> Option<HashMap<crate::types::GenericParamId, Type>> {
    if [builtin.fn_trait, builtin.fn_mut, builtin.fn_once].contains(&Some(trait_id)) {
        if let crate::hir::HirImplReceiverPattern::Exact(pattern) = &imp.receiver_pattern {
            let mut subst = HashMap::new();
            return crate::selection::type_pattern_matches(pattern, ty, &mut subst)
                .then_some(subst);
        }
    }
    crate::selection::impl_receiver_pattern_substitution(imp, ty, impls.values())
}

fn auto_trait_satisfied(
    ty: &Type,
    trait_id: DefId,
    impls: &HashMap<DefId, HirImpl>,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    builtin_traits: BuiltinTraitIds,
    visiting: &mut std::collections::HashSet<Type>,
) -> bool {
    let is_send = builtin_traits.send == Some(trait_id);
    let is_sync = builtin_traits.sync == Some(trait_id);
    if !is_send && !is_sync {
        return false;
    }
    if !visiting.insert(ty.clone()) {
        return true;
    }

    let nested = |nested: &Type, visiting: &mut std::collections::HashSet<Type>| {
        explicit_impl_exists_for(nested, trait_id, &[], impls, structs, enums, builtin_traits)
            || auto_trait_satisfied(
                nested,
                trait_id,
                impls,
                structs,
                enums,
                builtin_traits,
                visiting,
            )
    };

    let satisfied = match ty {
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
        | Type::Str => true,
        Type::Array(inner, _) | Type::Slice(inner) => nested(inner, visiting),
        Type::Tuple(elements) => elements.iter().all(|element| nested(element, visiting)),
        Type::Reference { .. } if is_send => false,
        Type::Reference { inner, .. } => {
            let required = builtin_traits.sync;
            required.is_some_and(|required| {
                explicit_impl_exists_for(
                    inner,
                    required,
                    &[],
                    impls,
                    structs,
                    enums,
                    builtin_traits,
                ) || auto_trait_satisfied(
                    inner,
                    required,
                    impls,
                    structs,
                    enums,
                    builtin_traits,
                    visiting,
                )
            })
        }
        Type::Pointer(_) => false,
        Type::Function { captures, .. } if is_send => captures
            .iter()
            .all(|capture| capture.kind == CaptureKind::Move && nested(&capture.ty, visiting)),
        Type::Function { captures, .. } => captures.iter().all(|capture| {
            let required = match capture.kind {
                CaptureKind::SharedBorrow => builtin_traits.sync,
                CaptureKind::MutableBorrow | CaptureKind::Move => builtin_traits.sync,
            };
            required.is_some_and(|required| {
                explicit_impl_exists_for(
                    &capture.ty,
                    required,
                    &[],
                    impls,
                    structs,
                    enums,
                    builtin_traits,
                ) || auto_trait_satisfied(
                    &capture.ty,
                    required,
                    impls,
                    structs,
                    enums,
                    builtin_traits,
                    visiting,
                )
            })
        }),
        Type::Struct { id, args } => structs.get(id).is_some_and(|structure| {
            let subst = args
                .iter()
                .cloned()
                .enumerate()
                .map(|(index, ty)| {
                    (
                        GenericParamId {
                            owner: *id,
                            index: index as u32,
                        },
                        ty,
                    )
                })
                .collect();
            structure
                .fields
                .iter()
                .all(|field| nested(&field.ty.substitute_generics(&subst), visiting))
        }),
        Type::Enum { id, args } => enums.get(id).is_some_and(|enumeration| {
            let subst = args
                .iter()
                .cloned()
                .enumerate()
                .map(|(index, ty)| {
                    (
                        GenericParamId {
                            owner: *id,
                            index: index as u32,
                        },
                        ty,
                    )
                })
                .collect();
            enumeration
                .variants
                .iter()
                .all(|variant| match &variant.fields {
                    HirVariantFields::Unit => true,
                    HirVariantFields::Positional(fields) => fields
                        .iter()
                        .all(|field| nested(&field.substitute_generics(&subst), visiting)),
                    HirVariantFields::Named(fields) => fields
                        .iter()
                        .all(|field| nested(&field.ty.substitute_generics(&subst), visiting)),
                })
        }),
        Type::Generic(_)
        | Type::Projection { .. }
        | Type::TypeVar(_)
        | Type::Constructor { .. }
        | Type::Apply { .. }
        | Type::Lambda { .. }
        | Type::BoundVar { .. }
        | Type::Error => false,
    };
    visiting.remove(ty);
    satisfied
}

fn impl_trait_args_match(
    expected: &[Type],
    actual: &[Type],
    subst: &mut HashMap<crate::types::GenericParamId, Type>,
) -> bool {
    if expected.len() != actual.len() {
        return false;
    }

    expected
        .iter()
        .zip(actual.iter())
        .all(|(expected, actual)| {
            matches!((expected, actual), (Type::Tuple(items), Type::Unit) if items.is_empty())
                || matches!((expected, actual), (Type::Unit, Type::Tuple(items)) if items.is_empty())
                || crate::selection::type_pattern_matches(expected, actual, subst)
        })
}

fn type_is_sized(ty: &Type) -> bool {
    match ty {
        Type::Slice(_) | Type::Str => false,
        Type::Array(inner, _) => type_is_sized(inner),
        Type::Tuple(elems) => elems.iter().all(type_is_sized),
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
        Type::Generic(_)
        | Type::Projection { .. }
        | Type::TypeVar(_)
        | Type::Constructor { .. }
        | Type::Apply { .. }
        | Type::Lambda { .. }
        | Type::BoundVar { .. }
        | Type::Error => false,
    }
}

fn impl_receiver_owner_matches(
    ty: &Type,
    imp: &HirImpl,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
) -> bool {
    if let (
        Type::Reference { .. },
        HirImplReceiverPattern::Exact(pattern @ Type::Reference { .. }),
    ) = (ty, &imp.receiver_pattern)
    {
        // Generic borrowed receivers have different display spellings once
        // instantiated; compare their structure and nominal IDs instead.
        return crate::selection::type_pattern_matches(pattern, ty, &mut HashMap::new());
    }
    if matches!(
        &imp.receiver_pattern,
        HirImplReceiverPattern::Exact(Type::Generic(_))
    ) || matches!(
        &imp.receiver_pattern,
        HirImplReceiverPattern::Exact(Type::Apply { constructor, .. })
            if matches!(constructor.as_ref(), Type::Generic(_))
    ) {
        return true;
    }
    if let Some(id) = constructor_owner_id(ty) {
        return (structs.contains_key(&id) || enums.contains_key(&id))
            && receiver_owner_id(imp) == Some(id);
    }
    match ty {
        Type::Struct { id, .. } => structs.contains_key(id) && receiver_owner_id(imp) == Some(*id),
        Type::Enum { id, .. } => enums.contains_key(id) && receiver_owner_id(imp) == Some(*id),
        _ => {
            let ty_name = type_base_name(ty);
            imp.type_name == ty_name
                || (!imp.type_generics.is_empty() && type_shape_matches(&imp.type_name, &ty_name))
        }
    }
}

fn constructor_owner_id(ty: &Type) -> Option<DefId> {
    match ty {
        Type::Constructor { id, .. } | Type::Struct { id, .. } | Type::Enum { id, .. } => Some(*id),
        Type::Apply { constructor, .. } => constructor_owner_id(constructor),
        Type::Lambda { body, .. } => constructor_owner_id(body),
        _ => None,
    }
}

fn receiver_owner_id(imp: &HirImpl) -> Option<DefId> {
    match &imp.receiver_pattern {
        crate::hir::HirImplReceiverPattern::Exact(Type::Struct { id, .. })
        | crate::hir::HirImplReceiverPattern::Exact(Type::Enum { id, .. }) => Some(*id),
        crate::hir::HirImplReceiverPattern::Constructor(ty) => constructor_owner_id(ty),
        crate::hir::HirImplReceiverPattern::Exact(_)
        | crate::hir::HirImplReceiverPattern::SliceFamily { .. } => None,
    }
}

fn fixed_array_len_from_type_name(type_name: &str) -> Option<usize> {
    let inner = type_name.strip_prefix('[')?.strip_suffix(']')?;
    let (_, len) = inner.split_once(';')?;
    len.trim().parse().ok()
}

fn type_shape_matches(candidate: &str, lookup: &str) -> bool {
    if candidate == lookup {
        return true;
    }

    match (
        fixed_array_len_from_type_name(candidate),
        fixed_array_len_from_type_name(lookup),
    ) {
        (Some(candidate_len), Some(lookup_len)) => candidate_len == lookup_len,
        _ => candidate == "Array" && lookup == "Array",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::hir::HirImplOwner;
    use crate::ids::{AssocTypeId, CrateId, LocalDefId, TypeVarId};
    use crate::language_items::FnOnceLanguageItems;
    use crate::lexer::Span;
    use crate::types::TraitBound;

    #[test]
    fn solve_errors_keep_unification_payload_until_rendering() {
        let error = SolveError {
            kind: SolveErrorKind::Unify {
                context: "assignment".to_string(),
                error: UnifyError::Mismatch {
                    expected: Type::I64,
                    found: Type::Bool,
                },
            },
            span: Span::test(),
        };
        let engine = InferenceEngine::new();
        assert!(matches!(error.kind, SolveErrorKind::Unify { .. }));
        assert!(error.render(&engine).contains("assignment"));
        assert!(error.render(&engine).contains("I64"));
        assert!(error.render(&engine).contains("Bool"));
        assert!(error.contains("Type mismatch"));
    }

    fn def_id(index: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(index))
    }

    fn test_impl(type_name: &str, owner_id: DefId, trait_id: DefId) -> HirImpl {
        HirImpl {
            id: def_id(100),
            owner: HirImplOwner::Named(type_name.to_string()),
            type_name: type_name.to_string(),
            type_generics: Vec::new(),
            receiver_pattern: crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                id: owner_id,
                args: Vec::new(),
            }),
            trait_name: Some("Show".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        }
    }

    #[test]
    fn solve_constraints_returns_generic_bounds_by_typed_type_var_id() {
        let mut engine = InferenceEngine::new();
        let mut store = ConstraintStore::new();
        store.add_trait(
            Type::TypeVar(TypeVarId(0)),
            TraitBound {
                trait_id: crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(1)),
                type_args: Vec::new(),
            },
            Span::test(),
            "test",
        );

        let result = solve_constraints_in_place(
            &mut engine,
            &mut store,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &LanguageItems::default(),
        );

        assert_eq!(
            result.generic_bounds.get(&TypeVarId(0)),
            Some(&vec![TraitBound {
                trait_id: crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(1)),
                type_args: Vec::new(),
            }])
        );
    }

    #[test]
    fn concrete_trait_bound_matches_struct_impl_by_def_id() {
        let trait_id = def_id(1);
        let struct_id = def_id(2);
        let mut structs = HashMap::new();
        structs.insert(
            struct_id,
            HirStruct {
                id: struct_id,
                name: "Widget".to_string(),
                generic_params: Vec::new(),
                fields: Vec::new(),
            },
        );

        assert!(impl_exists_for(
            &Type::Struct {
                id: struct_id,
                args: Vec::new(),
            },
            trait_id,
            &[],
            &HashMap::from([(def_id(3), test_impl("Widget", struct_id, trait_id))]),
            &structs,
            &HashMap::new(),
            BuiltinTraitIds::default(),
        ));
    }

    #[test]
    fn constructor_trait_selection_solver_matches_constructor_impl_by_def_id() {
        let trait_id = def_id(20);
        let enum_id = def_id(21);
        let constructor = Type::Constructor {
            id: enum_id,
            flavor: crate::types::NominalTypeKind::Enum,
        };
        let mut imp = test_impl("Maybe", enum_id, trait_id);
        imp.receiver_pattern = crate::hir::HirImplReceiverPattern::Constructor(constructor.clone());
        let impls = HashMap::from([(imp.id, imp)]);
        let enums = HashMap::from([(
            enum_id,
            HirEnum {
                id: enum_id,
                name: "Maybe".to_string(),
                generic_params: vec![crate::types::GenericParamDecl::type_param(
                    crate::types::GenericParamId {
                        owner: enum_id,
                        index: 0,
                    },
                    "T",
                )],
                variants: Vec::new(),
            },
        )]);
        let mut store = ConstraintStore::new();
        store.add_trait(
            constructor,
            TraitBound {
                trait_id,
                type_args: Vec::new(),
            },
            Span::test(),
            "constructor bound",
        );

        let result = solve_constraints_in_place(
            &mut InferenceEngine::new(),
            &mut store,
            &impls,
            &HashMap::new(),
            &enums,
            &HashMap::new(),
            &LanguageItems::default(),
        );

        assert!(result.errors.is_empty(), "{:?}", result.errors);
    }

    #[test]
    fn constructor_candidate_with_unsatisfied_impl_bound_is_not_selected() {
        let constructor_trait = def_id(24);
        let required_trait = def_id(25);
        let enum_id = def_id(26);
        let impl_id = def_id(27);
        let generic = crate::types::GenericParamId {
            owner: impl_id,
            index: 0,
        };
        let mut imp = test_impl("Bounded", enum_id, constructor_trait);
        imp.id = impl_id;
        imp.type_generics = vec![crate::types::GenericParamDecl::type_param(generic, "T")];
        imp.receiver_pattern = HirImplReceiverPattern::Constructor(Type::Constructor {
            id: enum_id,
            flavor: crate::types::NominalTypeKind::Enum,
        });
        imp.bounds = HashMap::from([(
            generic,
            vec![TraitBound {
                trait_id: required_trait,
                type_args: Vec::new(),
            }],
        )])
        .into();
        let impls = HashMap::from([(impl_id, imp)]);
        let mut engine = InferenceEngine::new();
        let constructor = engine.fresh_type_var_of_kind(crate::type_services::kind::Kind::arrow(
            crate::type_services::kind::Kind::Type,
            crate::type_services::kind::Kind::Type,
        ));
        let Type::TypeVar(constructor_id) = constructor else {
            unreachable!();
        };
        let mut store = ConstraintStore::new();
        let origin = Span {
            file_path: "main.rk".into(),
            start: 14,
            end: 21,
        };
        store.add_trait(
            Type::TypeVar(constructor_id),
            TraitBound {
                trait_id: constructor_trait,
                type_args: Vec::new(),
            },
            origin.clone(),
            "bounded constructor candidate",
        );

        let result = solve_constraints_in_place(
            &mut engine,
            &mut store,
            &impls,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &LanguageItems::default(),
        );

        assert!(result.errors[0].contains("bounded constructor candidate"));
        assert_eq!(result.errors[0].span.file_path, origin.file_path);
        assert_eq!(
            (result.errors[0].span.start, result.errors[0].span.end),
            (origin.start, origin.end)
        );
        assert_eq!(
            engine.resolve(&Type::TypeVar(constructor_id)),
            Type::TypeVar(constructor_id)
        );
    }

    #[test]
    fn structural_worklist_retries_hkt_equality_after_callable_progress() {
        let callable_trait = def_id(40);
        let carrier_id = def_id(41);
        let mut engine = InferenceEngine::new();
        let mut normalization_env = crate::type_services::normalize::TypeNormalizationEnv::new();
        normalization_env.register_nominal(carrier_id, crate::types::NominalTypeKind::Enum, 1);
        engine.set_normalization_env(normalization_env);
        let constructor = engine.fresh_type_var_of_kind(crate::type_services::kind::Kind::arrow(
            crate::type_services::kind::Kind::Type,
            crate::type_services::kind::Kind::Type,
        ));
        let payload = engine.fresh_type_var();
        let callable_result = engine.fresh_type_var();
        let callable = Type::function(
            vec![Type::I64],
            Type::Enum {
                id: carrier_id,
                args: vec![Type::Bool],
            },
        );
        let application = Type::Apply {
            constructor: Box::new(constructor.clone()),
            args: vec![payload.clone()],
        };
        let mut store = ConstraintStore::new();
        store.add_equality(
            application,
            callable_result.clone(),
            Span::test(),
            "deferred HKT equality",
        );
        store.add_trait(
            callable,
            TraitBound {
                trait_id: callable_trait,
                type_args: vec![Type::I64, callable_result.clone()],
            },
            Span::test(),
            "callable result",
        );
        let language_items = LanguageItems {
            fn_once: Some(FnOnceLanguageItems {
                trait_id: callable_trait,
                output_id: AssocTypeId(0),
                method_id: def_id(42),
            }),
            ..LanguageItems::default()
        };

        let result = solve_constraints_in_place(
            &mut engine,
            &mut store,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &language_items,
        );

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(
            engine.resolve(&callable_result).to_string(),
            Type::Enum {
                id: carrier_id,
                args: vec![Type::Bool],
            }
            .to_string()
        );
        assert_eq!(
            engine.resolve(&constructor),
            Type::Constructor {
                id: carrier_id,
                flavor: crate::types::NominalTypeKind::Enum,
            }
        );
        assert_eq!(engine.resolve(&payload), Type::Bool);
    }

    #[test]
    fn structural_worklist_wakes_try_after_carrier_progress() {
        let mut engine = InferenceEngine::new();
        let carrier = engine.fresh_type_var();
        let mut store = ConstraintStore::new();
        let try_obligation = store.add_try(
            carrier.clone(),
            Type::Bool,
            Type::I64,
            Type::I64,
            Span::test(),
        );
        store.add_equality(
            carrier.clone(),
            Type::I64,
            Span::test(),
            "deferred carrier equality",
        );

        let result = solve_constraints_in_place(
            &mut engine,
            &mut store,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &LanguageItems::default(),
        );

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(engine.resolve(&carrier), Type::I64);
        assert_eq!(
            store.obligation_state(try_obligation),
            Some(ObligationState::Solved)
        );
        assert_eq!(store.obligation_attempts(try_obligation), Some(2));
    }

    #[test]
    fn try_obligation_accepts_generalized_constructor_heads() {
        let engine = &mut InferenceEngine::new();
        let generic = Type::Generic(GenericParamId {
            owner: def_id(49),
            index: 0,
        });
        let carrier = Type::Apply {
            constructor: Box::new(generic.clone()),
            args: vec![Type::I64],
        };
        let mut store = ConstraintStore::new();
        let obligation =
            store.add_try(carrier.clone(), Type::I64, Type::I64, carrier, Span::test());

        let result = solve_constraints_in_place(
            engine,
            &mut store,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &LanguageItems::default(),
        );

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(
            store.obligation_state(obligation),
            Some(ObligationState::Solved)
        );
    }

    #[test]
    fn structural_worklist_wakes_only_dependent_obligations() {
        let callable_trait = def_id(50);
        let carrier_id = def_id(51);
        let mut engine = InferenceEngine::new();
        let mut normalization_env = crate::type_services::normalize::TypeNormalizationEnv::new();
        normalization_env.register_nominal(carrier_id, crate::types::NominalTypeKind::Enum, 1);
        engine.set_normalization_env(normalization_env);
        let constructor = engine.fresh_type_var_of_kind(crate::type_services::kind::Kind::arrow(
            crate::type_services::kind::Kind::Type,
            crate::type_services::kind::Kind::Type,
        ));
        let payload = engine.fresh_type_var();
        let callable_result = engine.fresh_type_var();
        let unrelated = engine.fresh_type_var();
        let application = Type::Apply {
            constructor: Box::new(constructor),
            args: vec![payload],
        };
        let mut store = ConstraintStore::new();
        let dependent = store.add_equality(
            application,
            callable_result.clone(),
            Span::test(),
            "dependent equality",
        );
        let callable = Type::function(
            vec![Type::I64],
            Type::Enum {
                id: carrier_id,
                args: vec![Type::Bool],
            },
        );
        let callable_id = store.add_trait(
            callable,
            TraitBound {
                trait_id: callable_trait,
                type_args: vec![Type::I64, callable_result],
            },
            Span::test(),
            "callable result",
        );
        let unrelated_id = store.add_int_literal(
            match unrelated {
                Type::TypeVar(id) => id,
                _ => unreachable!(),
            },
            Span::test(),
        );
        let language_items = LanguageItems {
            fn_once: Some(FnOnceLanguageItems {
                trait_id: callable_trait,
                output_id: AssocTypeId(0),
                method_id: def_id(52),
            }),
            ..LanguageItems::default()
        };

        let result = solve_constraints_in_place(
            &mut engine,
            &mut store,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &language_items,
        );

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(store.obligation_attempts(dependent), Some(2));
        assert_eq!(store.obligation_attempts(callable_id), Some(1));
        assert_eq!(store.obligation_attempts(unrelated_id), Some(1));
    }

    #[test]
    fn unknown_constructor_heads_remain_ambiguous_at_quiescence() {
        let mut engine = InferenceEngine::new();
        let kind = crate::type_services::kind::Kind::arrow(
            crate::type_services::kind::Kind::Type,
            crate::type_services::kind::Kind::Type,
        );
        let left = engine.fresh_type_var_of_kind(kind.clone());
        let right = engine.fresh_type_var_of_kind(kind);
        let mut store = ConstraintStore::new();
        let id = store.add_equality(
            Type::Apply {
                constructor: Box::new(left),
                args: vec![Type::I64],
            },
            Type::Apply {
                constructor: Box::new(right),
                args: vec![Type::Bool],
            },
            Span::test(),
            "unknown constructor heads",
        );

        let result = solve_constraints_in_place(
            &mut engine,
            &mut store,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &LanguageItems::default(),
        );

        assert!(result.errors[0].contains("unknown constructor heads"));
        assert_eq!(store.obligation_state(id), Some(ObligationState::Ambiguous));
    }

    #[test]
    fn multiple_constructor_candidates_remain_ambiguous() {
        let trait_id = def_id(60);
        let first_id = def_id(61);
        let second_id = def_id(62);
        let make_impl = |id: DefId, owner: DefId| HirImpl {
            id,
            owner: HirImplOwner::Named(format!("Constructor{id:?}")),
            type_name: format!("Constructor{id:?}"),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Constructor(Type::Constructor {
                id: owner,
                flavor: crate::types::NominalTypeKind::Enum,
            }),
            trait_name: None,
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::new(),
        };
        let impls = HashMap::from([
            (first_id, make_impl(first_id, first_id)),
            (second_id, make_impl(second_id, second_id)),
        ]);
        let mut engine = InferenceEngine::new();
        let mut normalization_env = crate::type_services::normalize::TypeNormalizationEnv::new();
        normalization_env.register_nominal(first_id, crate::types::NominalTypeKind::Enum, 0);
        normalization_env.register_nominal(second_id, crate::types::NominalTypeKind::Enum, 0);
        engine.set_normalization_env(normalization_env);
        let constructor = engine.fresh_type_var_of_kind(crate::type_services::kind::Kind::arrow(
            crate::type_services::kind::Kind::Type,
            crate::type_services::kind::Kind::Type,
        ));
        let Type::TypeVar(constructor_id) = constructor else {
            unreachable!();
        };
        let mut store = ConstraintStore::new();
        let obligation = store.add_trait(
            Type::TypeVar(constructor_id),
            TraitBound {
                trait_id,
                type_args: Vec::new(),
            },
            Span::test(),
            "constructor candidates",
        );

        let result = solve_constraints_in_place(
            &mut engine,
            &mut store,
            &impls,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &LanguageItems::default(),
        );

        assert!(result.errors[0].contains("constructor candidates"));
        assert_eq!(
            store.obligation_state(obligation),
            Some(ObligationState::Ambiguous)
        );
        assert_eq!(
            engine.resolve(&Type::TypeVar(constructor_id)),
            Type::TypeVar(constructor_id)
        );
    }

    #[test]
    fn ambiguous_obligation_reopens_after_new_evidence() {
        let trait_id = def_id(70);
        let first_id = def_id(71);
        let second_id = def_id(72);
        let make_impl = |id: DefId| HirImpl {
            id,
            owner: crate::hir::HirImplOwner::Named(format!("Constructor{id:?}")),
            type_name: format!("Constructor{id:?}"),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Constructor(Type::Constructor {
                id,
                flavor: crate::types::NominalTypeKind::Enum,
            }),
            trait_name: None,
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::new(),
        };
        let impls = HashMap::from([
            (first_id, make_impl(first_id)),
            (second_id, make_impl(second_id)),
        ]);
        let mut engine = InferenceEngine::new();
        let variable = engine.fresh_type_var_of_kind(crate::type_services::kind::Kind::arrow(
            crate::type_services::kind::Kind::Type,
            crate::type_services::kind::Kind::Type,
        ));
        let Type::TypeVar(variable_id) = variable else {
            unreachable!();
        };
        let mut store = ConstraintStore::new();
        let obligation = store.add_trait(
            Type::TypeVar(variable_id),
            TraitBound {
                trait_id,
                type_args: Vec::new(),
            },
            Span::test(),
            "ambiguity reopening",
        );
        let result = solve_constraints_in_place(
            &mut engine,
            &mut store,
            &impls,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &LanguageItems::default(),
        );
        assert!(result.errors[0].contains("ambiguity reopening"));
        assert_eq!(
            store.obligation_state(obligation),
            Some(ObligationState::Ambiguous)
        );
        store.reopen_ambiguous(obligation);
        assert_eq!(
            store.obligation_state(obligation),
            Some(ObligationState::Pending)
        );
    }

    #[test]
    fn recursive_supertrait_obligations_terminate_and_are_memoized() {
        let first = def_id(30);
        let second = def_id(31);
        let trait_def = |id: DefId, name: &str, parent: DefId| crate::hir::HirTrait {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            target: None,
            predicates: vec![crate::types::Predicate::Trait {
                subject: Type::Generic(crate::types::GenericParamId {
                    owner: id,
                    index: 0,
                }),
                trait_id: parent,
                args: Vec::new(),
            }],
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        };
        let traits = HashMap::from([
            (first, trait_def(first, "First", second)),
            (second, trait_def(second, "Second", first)),
        ]);
        let mut engine = InferenceEngine::new();
        let variable = engine.fresh_type_var();
        let Type::TypeVar(variable_id) = variable.clone() else {
            unreachable!();
        };
        let mut store = ConstraintStore::new();
        store.add_trait(
            variable,
            TraitBound {
                trait_id: first,
                type_args: Vec::new(),
            },
            Span::test(),
            "recursive test",
        );

        let result = solve_constraints_in_place(
            &mut engine,
            &mut store,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &traits,
            &LanguageItems::default(),
        );

        let bounds = &result.generic_bounds[&variable_id];
        assert_eq!(bounds.len(), 2);
        assert!(bounds.iter().any(|bound| bound.trait_id == first));
        assert!(bounds.iter().any(|bound| bound.trait_id == second));
    }

    #[test]
    fn same_named_traits_do_not_satisfy_each_other_by_name() {
        let required_trait_id = def_id(1);
        let wrong_trait_id = def_id(2);
        let struct_id = def_id(3);
        let mut structs = HashMap::new();
        structs.insert(
            struct_id,
            HirStruct {
                id: struct_id,
                name: "Widget".to_string(),
                generic_params: Vec::new(),
                fields: Vec::new(),
            },
        );

        assert!(!impl_exists_for(
            &Type::Struct {
                id: struct_id,
                args: Vec::new(),
            },
            required_trait_id,
            &[],
            &HashMap::from([(def_id(4), test_impl("Widget", struct_id, wrong_trait_id))]),
            &structs,
            &HashMap::new(),
            BuiltinTraitIds::default(),
        ));
    }
}

/// Return the base name of a type for impl lookup (e.g. "I64", "Vec", "Point").
fn type_base_name(ty: &Type) -> String {
    match ty {
        Type::I8 => "I8".into(),
        Type::I16 => "I16".into(),
        Type::I32 => "I32".into(),
        Type::I64 => "I64".into(),
        Type::U8 => "U8".into(),
        Type::U16 => "U16".into(),
        Type::U32 => "U32".into(),
        Type::U64 => "U64".into(),
        Type::F32 => "F32".into(),
        Type::F64 => "F64".into(),
        Type::Bool => "Bool".into(),
        Type::Str => "Str".into(),
        Type::Char => "Char".into(),
        Type::Slice(_) => "Array".into(),
        Type::Array(_, _) => ty.to_string(),
        Type::Struct { .. } | Type::Enum { .. } => ty.to_string(),
        _ => ty.to_string(),
    }
}

use std::collections::{BTreeMap, HashSet, VecDeque};

use crate::ids::DefId;
use crate::lower::ResolveError;
use crate::types::Type;

use super::{authority, generalize, solve, ConstraintOwner, ObligationState, PartialHir};

pub(super) fn drive_inference_to_quiescence(hir: &mut PartialHir) -> Result<(), Vec<ResolveError>> {
    fn declaration_bounds(hir: &PartialHir) -> crate::hir::HirGenericBounds {
        let mut result = crate::hir::HirGenericBounds::new();
        for bounds in hir
            .functions
            .values()
            .map(|function| &function.generic_bounds)
            .chain(hir.impls.values().map(|imp| &imp.bounds))
            .chain(
                hir.impls
                    .values()
                    .flat_map(|imp| imp.methods.values().map(|method| &method.generic_bounds)),
            )
            .chain(
                hir.traits
                    .values()
                    .flat_map(|tr| tr.methods.values().map(|method| &method.generic_bounds)),
            )
        {
            for (param, bounds) in bounds {
                let existing = result.entry(*param).or_default();
                for bound in bounds {
                    if !existing.contains(bound) {
                        existing.push(bound.clone());
                    }
                }
            }
            // Concrete-subject clauses (`where String: From T`) are assumptions too.
            for predicate in &bounds.predicates {
                if !result.predicates.contains(predicate) {
                    result.predicates.push(predicate.clone());
                }
            }
        }
        // A parameter passed by value already requires a sized runtime layout;
        // retain that fact when proving blanket impls for references to it.
        if let Some(sized) = &hir.language_items.sized {
            for function in hir
                .functions
                .values()
                .chain(hir.impls.values().flat_map(|imp| imp.methods.values()))
                .chain(hir.traits.values().flat_map(|tr| tr.methods.values()))
            {
                for param in &function.params {
                    if let Type::Generic(id) = &param.ty {
                        let bound = crate::types::TraitBound {
                            trait_id: sized.trait_id,
                            type_args: Vec::new(),
                        };
                        let bounds = result.entry(*id).or_default();
                        if !bounds.contains(&bound) {
                            bounds.push(bound);
                        }
                    }
                }
            }
        }
        result
    }
    #[derive(Clone, Copy)]
    enum WorkItem {
        Structural,
        Authority(authority::AuthorityObligationId),
    }

    fn solve_component_worklist(
        hir: &mut PartialHir,
        owners: &HashSet<ConstraintOwner>,
        authority_obligations: &mut [authority::AuthorityObligation],
        strict: bool,
        assumptions: &crate::hir::HirGenericBounds,
    ) -> Result<(), Vec<ResolveError>> {
        if strict {
            for obligation in authority_obligations.iter_mut() {
                obligation.wake();
            }
        }
        let mut queue = VecDeque::from([WorkItem::Structural]);
        queue.extend(
            authority_obligations
                .iter()
                .map(|obligation| WorkItem::Authority(obligation.id)),
        );
        let mut structural_queued = true;
        let mut authority_queued = authority_obligations
            .iter()
            .map(|obligation| obligation.id)
            .collect::<HashSet<_>>();

        while let Some(item) = queue.pop_front() {
            match item {
                WorkItem::Structural => {
                    structural_queued = false;
                    let solved = solve::solve_constraints_in_place_for_owners(
                        &mut hir.engine,
                        &mut hir.constraint_store,
                        &hir.impls,
                        &hir.structs,
                        &hir.enums,
                        &hir.traits,
                        &hir.language_items,
                        owners,
                        false,
                        assumptions,
                        &HashSet::new(),
                    );
                    if !solved.errors.is_empty() {
                        return Err(solved
                            .errors
                            .into_iter()
                            .map(|error| {
                                ResolveError::with_span_code(
                                    error.render(&hir.engine),
                                    error.span,
                                    crate::diagnostic::DiagnosticCode::Type,
                                )
                            })
                            .collect());
                    }
                    let changed = solved.changed_type_vars.into_iter().collect::<HashSet<_>>();
                    for obligation in authority_obligations.iter_mut() {
                        if obligation.depends_on_any(&changed) {
                            obligation.wake();
                            if authority_queued.insert(obligation.id) {
                                queue.push_back(WorkItem::Authority(obligation.id));
                            }
                        }
                    }
                }
                WorkItem::Authority(id) => {
                    authority_queued.remove(&id);
                    let index = id.raw() as usize;
                    let constraint_count = hir.constraint_store.iter().count();
                    let result = authority::run_authority_obligation(
                        hir,
                        &mut authority_obligations[index],
                        strict,
                        strict,
                    )?;
                    let changed = hir
                        .engine
                        .take_changed_type_vars()
                        .into_iter()
                        .collect::<HashSet<_>>();
                    let added_constraints = hir.constraint_store.iter().count() != constraint_count;
                    if (!changed.is_empty() || added_constraints) && !structural_queued {
                        structural_queued = true;
                        queue.push_back(WorkItem::Structural);
                    }
                    let completed_owner = authority_obligations[index].owner;
                    for obligation in authority_obligations.iter_mut() {
                        let propagation_edge = result.progress
                            && obligation.owner == completed_owner
                            && obligation.kind == authority::AuthorityObligationKind::Propagation;
                        // Materializing a nested site can change the receiver
                        // of an outer site without a tracked variable change.
                        let strict_body_progress = strict
                            && result.progress
                            && obligation.owner == completed_owner
                            && obligation.state != ObligationState::Solved;
                        if propagation_edge
                            || strict_body_progress
                            || obligation.depends_on_any(&changed)
                        {
                            obligation.wake();
                            if authority_queued.insert(obligation.id) {
                                queue.push_back(WorkItem::Authority(obligation.id));
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    let root_entrypoint_id = hir.resolver.item_paths.get("main").copied();
    let mut components = hir.inference_scc_order.clone();
    if components.is_empty() {
        components = hir
            .functions
            .keys()
            .filter_map(|id| hir.inference_sccs.get(id).copied().or(Some(*id)))
            .collect();
        components.sort();
        components.dedup();
    }
    let mut members = BTreeMap::<_, Vec<_>>::new();
    for (&id, &component) in &hir.inference_sccs {
        members.entry(component).or_default().push(id);
    }
    let ordered: HashSet<_> = components.iter().copied().collect();
    components.extend(members.keys().filter(|id| !ordered.contains(*id)).copied());
    if components.is_empty() {
        components.push(DefId::new(hir.root_crate_id, crate::ids::LocalDefId(0)));
    }
    let mut first_component = true;
    let mut component_work = Vec::with_capacity(components.len());
    for component in components {
        let mut ids = members.remove(&component).unwrap_or_default();
        if ids.is_empty() && hir.functions.contains_key(&component) {
            ids.push(component);
        }
        ids.sort();
        let mut owners = ids
            .iter()
            .copied()
            .map(ConstraintOwner::Body)
            .collect::<HashSet<_>>();
        if first_component {
            owners.insert(ConstraintOwner::Global);
            first_component = false;
        }
        let authority_obligations = authority::authority_obligations(hir, &owners);
        component_work.push((ids, owners, authority_obligations));
    }

    // Seed inferred headers from caller context before dependency-first body
    // solving can commit an otherwise underconstrained rigid type.
    for (_, owners, _) in component_work.iter().rev() {
        authority::propagate_function_instances_for_owners_with_progress(hir, owners)?;
    }

    // Declaration bounds and declared parameter types do not change until generalization.
    let assumptions = declaration_bounds(hir);
    let mut component_queue = (0..component_work.len()).collect::<VecDeque<_>>();
    while let Some(index) = component_queue.pop_front() {
        let generation = hir.engine.substitution_generation();
        let (_, owners, authority_obligations) = &mut component_work[index];
        solve_component_worklist(hir, owners, authority_obligations, false, &assumptions)?;
        if generation != hir.engine.substitution_generation() {
            component_queue.extend(0..index);
        }
    }

    for (_, owners, _) in &component_work {
        hir.engine
            .apply_numeric_defaults_for_owners(&hir.constraint_store, Some(owners));
    }

    // Numeric defaults can discharge dependencies across SCC boundaries.
    // Revisit deferred authority before the strict diagnostic pass, even if
    // another component has already consumed the engine's changed-var list.
    for (_, _, obligations) in &mut component_work {
        for obligation in obligations {
            obligation.wake();
        }
    }

    let mut component_queue = (0..component_work.len()).collect::<VecDeque<_>>();
    while let Some(index) = component_queue.pop_front() {
        let generation = hir.engine.substitution_generation();
        let (_, owners, authority_obligations) = &mut component_work[index];
        solve_component_worklist(hir, owners, authority_obligations, false, &assumptions)?;
        if generation != hir.engine.substitution_generation() {
            component_queue.extend(0..index);
        }
    }

    // A value determined by a function parameter is inferred generically, not
    // an unconstrained construction site eligible for a declaration fallback.
    let mut parameter_vars = HashSet::new();
    for function in hir
        .functions
        .values()
        .chain(hir.impls.values().flat_map(|item| item.methods.values()))
        .chain(hir.traits.values().flat_map(|item| item.methods.values()))
    {
        for param in &function.params {
            crate::type_services::visit::visit_type(
                &hir.engine.resolve(&param.ty),
                &mut |ty: &Type| {
                    if let Type::TypeVar(id) = ty {
                        parameter_vars.insert(*id);
                    }
                },
            );
        }
    }
    hir.engine
        .apply_generic_defaults(&parameter_vars)
        .map_err(|error| {
            vec![ResolveError::from_optional_span_code(
                error.error.render(&hir.engine),
                error.span,
                crate::diagnostic::DiagnosticCode::Type,
            )]
        })?;
    // An undetermined receiver is usually a consequence: its only caller failed
    // to type-check, so nothing fixed it. Keep checking later components and
    // report their definite errors first.
    let mut undetermined_receivers = Vec::new();
    for (_, owners, authority_obligations) in &mut component_work {
        match solve_component_worklist(hir, owners, authority_obligations, true, &assumptions) {
            Err(errors) if errors.iter().all(ResolveError::is_undetermined_receiver) => {
                undetermined_receivers.extend(errors);
                continue;
            }
            result => result?,
        }
        if authority_obligations.iter().any(|obligation| {
            obligation.kind != authority::AuthorityObligationKind::Propagation
                && obligation.state != ObligationState::Solved
        }) {
            undetermined_receivers.push(authority::pending_authority_error(authority_obligations));
        }
    }

    // Free variables in the signatures of functions generalization will abstract.
    let mut generalizable = HashSet::new();
    for function in hir.functions.values() {
        if root_entrypoint_id == Some(function.id) || !function.generic_params.is_empty() {
            continue;
        }
        for ty in function
            .params
            .iter()
            .map(|param| &param.ty)
            .chain(std::iter::once(&function.ret_type))
        {
            crate::type_services::visit::visit_type(
                &hir.engine.resolve(ty),
                &mut |nested: &Type| {
                    if let Type::TypeVar(id) = nested {
                        generalizable.insert(*id);
                    }
                },
            );
        }
    }

    let mut deferred_predicates = Vec::new();
    let mut held_errors = Vec::new();
    let mut held_deferred = Vec::new();
    for (_, owners, _) in &component_work {
        let solved = solve::solve_constraints_in_place_for_owners(
            &mut hir.engine,
            &mut hir.constraint_store,
            &hir.impls,
            &hir.structs,
            &hir.enums,
            &hir.traits,
            &hir.language_items,
            &owners,
            true,
            &assumptions,
            &generalizable,
        );
        if !solved.errors.is_empty() {
            if undetermined_receivers.is_empty() {
                return Err(final_solve_errors(
                    hir,
                    solved.errors,
                    &solved.deferred_predicates,
                    root_entrypoint_id,
                    &generalizable,
                ));
            }
            held_errors.extend(solved.errors);
            held_deferred.extend(solved.deferred_predicates);
            continue;
        }
        deferred_predicates.extend(solved.deferred_predicates);
    }
    if !undetermined_receivers.is_empty() {
        // Ambiguities left by the unfixed receiver are consequences too; only a
        // definite failure elsewhere outranks the receiver error.
        if held_errors.iter().any(|error| !error.is_ambiguity()) {
            return Err(final_solve_errors(
                hir,
                held_errors,
                &held_deferred,
                root_entrypoint_id,
                &generalizable,
            ));
        }
        return Err(undetermined_receivers);
    }

    let attached = attach_deferred_predicates(hir, &deferred_predicates, root_entrypoint_id)?;
    for (ids, _, _) in component_work {
        generalize::generalize_functions(hir, &ids, root_entrypoint_id);
    }
    // Generalization mints generics (possibly constructor-kinded: `F` in `F A`);
    // later normalization must know their kinds instead of defaulting to Type.
    let mut env = hir.engine.normalization_env();
    for function in hir.functions.values() {
        for param in &function.generic_params {
            env.register_generic_kind(param.id, param.kind.clone());
        }
    }
    hir.engine.set_normalization_env(env);
    reject_ungeneralized_predicates(hir, attached)?;
    let call_errors = authority::check_monomorphic_call_arguments(hir);
    if call_errors.is_empty() {
        Ok(())
    } else {
        Err(call_errors)
    }
}

/// A deferred obligation whose subject is concrete and whose trait arguments are
/// still free after the final solve (`String: From ?x`).
struct AttachedPredicate {
    function_id: DefId,
    error: solve::SolveError,
}

fn deferred_predicate_error(
    hir: &PartialHir,
    ty: &Type,
    bound: &crate::types::TraitBound,
    span: &crate::lexer::Span,
    context: &str,
) -> solve::SolveError {
    solve::SolveError {
        kind: solve::SolveErrorKind::TraitNotImplemented {
            ty: hir.engine.resolve(ty),
            trait_name: hir
                .traits
                .get(&bound.trait_id)
                .map(|trait_def| trait_def.name.clone())
                .unwrap_or_else(|| "<unknown trait>".to_string()),
            trait_args: bound
                .type_args
                .iter()
                .map(|arg| hir.engine.resolve(arg))
                .collect(),
            context: context.to_string(),
        },
        span: span.clone(),
    }
}

fn type_error(hir: &PartialHir, error: solve::SolveError) -> ResolveError {
    ResolveError::with_span_code(
        error.render(&hir.engine),
        error.span,
        crate::diagnostic::DiagnosticCode::Type,
    )
}

/// The inferred function a deferred obligation can become a where clause of:
/// not the entrypoint, and not a function that already has a signature.
fn deferred_predicate_owner(
    hir: &PartialHir,
    id: super::constraints::ObligationId,
    root_entrypoint_id: Option<DefId>,
) -> Option<DefId> {
    match hir.constraint_store.owner(id) {
        Some(ConstraintOwner::Body(function_id))
            if root_entrypoint_id != Some(function_id)
                && hir
                    .functions
                    .get(&function_id)
                    .is_some_and(|function| function.generic_params.is_empty()) =>
        {
            Some(function_id)
        }
        _ => None,
    }
}

/// Final-solve errors, root causes first. A deferred obligation no function can
/// take as a where clause is a definite failure (`closure (I64) -> I64` is not
/// `FnMut I64, (Option _)`); the types it leaves unsolved make later sites
/// such as `result.println!` ambiguous. Those ambiguities are consequences, so
/// they are reported only when nothing definite failed. This is program-wide:
/// an ambiguity unrelated to the failure is also held back and appears on the
/// next compile, once the definite errors are fixed. Narrowing it to the
/// failure's own types would need obligation identity on `SolveError`.
fn final_solve_errors(
    hir: &PartialHir,
    errors: Vec<solve::SolveError>,
    deferred: &[super::constraints::ObligationId],
    root_entrypoint_id: Option<DefId>,
    generalizable: &HashSet<crate::ids::TypeVarId>,
) -> Vec<ResolveError> {
    let mut definite = deferred
        .iter()
        .copied()
        .filter_map(|id| match hir.constraint_store.constraint(id) {
            Some(super::constraints::Constraint::Trait {
                ty,
                bound,
                span,
                context,
            }) => {
                // As in `reject_ungeneralized_predicates`: only signature
                // variables of a generalized function can be abstracted.
                let abstractable = deferred_predicate_owner(hir, id, root_entrypoint_id).is_some()
                    && std::iter::once(ty).chain(&bound.type_args).all(|ty| {
                        !crate::type_services::visit::type_any(
                            &hir.engine.resolve(ty),
                            |ty| matches!(ty, Type::TypeVar(var) if !generalizable.contains(var)),
                        )
                    });
                (!abstractable).then(|| deferred_predicate_error(hir, ty, bound, span, context))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let (ambiguous, failed): (Vec<_>, Vec<_>) = errors
        .into_iter()
        .partition(solve::SolveError::is_ambiguity);
    definite.extend(failed);
    let reported = if definite.is_empty() {
        ambiguous
    } else {
        definite
    };
    reported
        .into_iter()
        .map(|error| type_error(hir, error))
        .collect()
}

/// Record each deferred obligation as a where clause of its owning inferred
/// function, so generalization can rewrite its free variables to generics.
fn attach_deferred_predicates(
    hir: &mut PartialHir,
    deferred: &[super::constraints::ObligationId],
    root_entrypoint_id: Option<DefId>,
) -> Result<Vec<AttachedPredicate>, Vec<ResolveError>> {
    let mut attached = Vec::new();
    let mut errors = Vec::new();
    for id in deferred.iter().copied() {
        let Some(super::constraints::Constraint::Trait {
            ty,
            bound,
            span,
            context,
        }) = hir.constraint_store.constraint(id).cloned()
        else {
            continue;
        };
        let error = deferred_predicate_error(hir, &ty, &bound, &span, &context);
        let Some(owner) = deferred_predicate_owner(hir, id, root_entrypoint_id) else {
            errors.push(type_error(hir, error));
            continue;
        };
        let predicate = crate::types::Predicate::Trait {
            subject: hir.engine.resolve(&ty),
            trait_id: bound.trait_id,
            args: bound
                .type_args
                .iter()
                .map(|arg| hir.engine.resolve(arg))
                .collect(),
        };
        if let Some(function) = hir.functions.get_mut(&owner) {
            if !function.generic_bounds.predicates.contains(&predicate) {
                function.generic_bounds.predicates.push(predicate);
            }
        }
        hir.constraint_store.set_state(id, ObligationState::Solved);
        attached.push(AttachedPredicate {
            function_id: owner,
            error,
        });
    }
    if errors.is_empty() {
        Ok(attached)
    } else {
        Err(errors)
    }
}

/// Generalization rewrites signature variables in attached predicates to
/// generics. A predicate that still mentions an inference variable could not be
/// abstracted (the variable is not part of the signature) and is a real error.
fn reject_ungeneralized_predicates(
    hir: &mut PartialHir,
    attached: Vec<AttachedPredicate>,
) -> Result<(), Vec<ResolveError>> {
    let mut errors = Vec::new();
    let mut reported = HashSet::new();
    for item in attached {
        let Some(function) = hir.functions.get(&item.function_id) else {
            continue;
        };
        let unresolved = function.generic_bounds.predicates.iter().any(|predicate| {
            let crate::types::Predicate::Trait { subject, args, .. } = predicate;
            std::iter::once(subject).chain(args).any(|ty| {
                crate::type_services::visit::type_any(ty, |ty| matches!(ty, Type::TypeVar(_)))
            })
        });
        if unresolved && reported.insert(item.function_id) {
            errors.push(type_error(hir, item.error));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

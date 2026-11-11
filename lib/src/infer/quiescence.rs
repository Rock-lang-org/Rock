use std::collections::{HashSet, VecDeque};

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
                    let assumptions = declaration_bounds(hir);
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
                        &assumptions,
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
                        if propagation_edge || obligation.depends_on_any(&changed) {
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
    let mut remaining_components = hir.inference_sccs.values().copied().collect::<Vec<_>>();
    remaining_components.sort();
    remaining_components.dedup();
    for component in remaining_components {
        if !components.contains(&component) {
            components.push(component);
        }
    }
    if components.is_empty() {
        components.push(DefId::new(hir.root_crate_id, crate::ids::LocalDefId(0)));
    }
    let mut first_component = true;
    let mut component_work = Vec::with_capacity(components.len());
    for component in components {
        let mut ids = hir
            .inference_sccs
            .iter()
            .filter_map(|(id, representative)| (*representative == component).then_some(*id))
            .collect::<Vec<_>>();
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
        authority::propagate_function_instances_for_owners_with_progress(hir, Some(owners))?;
    }

    let mut component_queue = (0..component_work.len()).collect::<VecDeque<_>>();
    while let Some(index) = component_queue.pop_front() {
        let generation = hir.engine.substitution_generation();
        let (_, owners, authority_obligations) = &mut component_work[index];
        solve_component_worklist(hir, owners, authority_obligations, false)?;
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
        solve_component_worklist(hir, owners, authority_obligations, false)?;
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
    for (_, owners, authority_obligations) in &mut component_work {
        solve_component_worklist(hir, &owners, authority_obligations, true)?;
        if authority_obligations.iter().any(|obligation| {
            obligation.kind != authority::AuthorityObligationKind::Propagation
                && obligation.state != ObligationState::Solved
        }) {
            let mut error =
                authority::pending_authority_error(hir, Some(&owners), authority_obligations);
            if let Some(context) = authority::pending_authority_context(&authority_obligations) {
                error.message = format!("{} ({context})", error.message);
            }
            return Err(vec![error]);
        }
    }

    for (_, owners, _) in &component_work {
        let assumptions = declaration_bounds(hir);
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
    }

    for (ids, _, _) in component_work {
        generalize::generalize_functions(hir, &ids, root_entrypoint_id);
    }
    Ok(())
}

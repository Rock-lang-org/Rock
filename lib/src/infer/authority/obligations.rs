use std::collections::{HashMap, HashSet};

use crate::hir::HirFunction;
use crate::infer::{ConstraintOwner, ObligationState, PartialHir};
use crate::lower::ResolveError;

use super::propagate::propagate_block;
use super::sites::{collect_authority_sites, materialize_authority_site};
use super::status::{authority_counts, authority_pass_result, owner_selected};
use super::type_vars::{collect_block_type_vars, collect_type_vars, constrained_signature_vars};
use super::{
    AuthorityObligation, AuthorityObligationId, AuthorityObligationKind, AuthorityPassResult,
};

pub(in crate::infer) fn authority_obligations(
    hir: &PartialHir,
    owners: &HashSet<ConstraintOwner>,
) -> Vec<AuthorityObligation> {
    let mut selected_owners = owners
        .iter()
        .copied()
        .filter(|owner| matches!(owner, ConstraintOwner::Body(_)))
        .collect::<Vec<_>>();
    selected_owners.sort();
    let mut obligations = Vec::with_capacity(selected_owners.len() * 2);
    for owner in selected_owners {
        let id = AuthorityObligationId(obligations.len() as u32);
        let mut propagation = AuthorityObligation {
            id,
            owner,
            kind: AuthorityObligationKind::Propagation,
            state: ObligationState::Pending,
            last_generation: u64::MAX,
            attempts: 0,
            context: function_for_owner(hir, owner)
                .map(|function| format!("call and type propagation for '{}'", function.name))
                .unwrap_or_else(|| "call and type propagation".to_string()),
            span: None,
            site: None,
            dependencies: HashSet::new(),
        };
        refresh_authority_dependencies(hir, &mut propagation);
        obligations.push(propagation);

        if let Some(function) = function_for_owner(hir, owner) {
            let mut sites = Vec::new();
            collect_authority_sites(&function.body, &hir.engine, &mut sites);
            for (kind, site, context, span, dependencies) in sites {
                let id = AuthorityObligationId(obligations.len() as u32);
                obligations.push(AuthorityObligation {
                    id,
                    owner,
                    kind,
                    state: ObligationState::Pending,
                    last_generation: u64::MAX,
                    attempts: 0,
                    context,
                    span: Some(span),
                    site: Some(site),
                    dependencies: dependencies
                        .into_iter()
                        .map(|var| {
                            hir.engine
                                .unresolved_type_var_representative_for_dependency(var)
                                .unwrap_or(var)
                        })
                        .collect(),
                });
            }
        }
    }
    obligations
}

pub(in crate::infer) fn run_authority_obligation(
    hir: &mut PartialHir,
    obligation: &mut AuthorityObligation,
    strict: bool,
    try_strict: bool,
) -> Result<AuthorityPassResult, Vec<ResolveError>> {
    if obligation.state != ObligationState::Pending {
        return Ok(AuthorityPassResult {
            progress: false,
            pending: false,
            ambiguous: obligation.state == ObligationState::Ambiguous,
        });
    }
    let generation = hir.engine.substitution_generation();
    if obligation.last_generation == generation {
        return Ok(AuthorityPassResult {
            progress: false,
            pending: true,
            ambiguous: false,
        });
    }
    let owners = HashSet::from([obligation.owner]);
    let previous_owner = hir.constraint_store.replace_owner(obligation.owner);
    let result = match obligation.kind {
        AuthorityObligationKind::Propagation => {
            propagate_function_instances_for_owners_with_progress(hir, Some(&owners))
        }
        AuthorityObligationKind::DeferredCall
        | AuthorityObligationKind::CallableCall
        | AuthorityObligationKind::MethodCall
        | AuthorityObligationKind::Field
        | AuthorityObligationKind::TryBranch
        | AuthorityObligationKind::FromResidual => materialize_authority_site(
            hir,
            obligation.owner,
            obligation.site.clone().expect("selection obligation site"),
            strict,
            try_strict,
        ),
    };
    hir.constraint_store.replace_owner(previous_owner);
    let result = result?;
    obligation.last_generation = generation;
    obligation.attempts = obligation.attempts.saturating_add(1);
    if obligation.kind != AuthorityObligationKind::Propagation {
        if result.ambiguous {
            obligation.state = ObligationState::Ambiguous;
        } else if !result.pending {
            obligation.state = ObligationState::Solved;
        }
    }
    refresh_authority_dependencies(hir, obligation);
    Ok(result)
}

fn refresh_authority_dependencies(hir: &PartialHir, obligation: &mut AuthorityObligation) {
    obligation.dependencies.clear();
    let ConstraintOwner::Body(_) = obligation.owner else {
        return;
    };
    let function = function_for_owner(hir, obligation.owner);
    let Some(function) = function else {
        return;
    };
    if let Some(selected_site) = obligation.site.as_ref() {
        let mut sites = Vec::new();
        collect_authority_sites(&function.body, &hir.engine, &mut sites);
        if let Some((_, _, _, _, dependencies)) = sites
            .into_iter()
            .find(|(_, site, ..)| site == selected_site)
        {
            obligation.dependencies = dependencies;
        }
    } else {
        for ty in function
            .params
            .iter()
            .map(|param| &param.ty)
            .chain(std::iter::once(&function.ret_type))
        {
            collect_type_vars(&hir.engine.resolve(ty), &mut obligation.dependencies);
        }
        collect_block_type_vars(&function.body, &hir.engine, &mut obligation.dependencies);
    }
    let dependencies = obligation.dependencies.drain().collect::<Vec<_>>();
    for var in dependencies {
        obligation.dependencies.insert(
            hir.engine
                .unresolved_type_var_representative_for_dependency(var)
                .unwrap_or(var),
        );
    }
}

pub(super) fn function_for_owner(hir: &PartialHir, owner: ConstraintOwner) -> Option<&HirFunction> {
    let ConstraintOwner::Body(id) = owner else {
        return None;
    };
    hir.functions
        .get(&id)
        .or_else(|| {
            hir.traits
                .values()
                .flat_map(|trait_def| trait_def.methods.values())
                .find(|method| method.id == id)
        })
        .or_else(|| {
            hir.impls
                .values()
                .flat_map(|imp| imp.methods.values())
                .find(|method| method.id == id)
        })
}

pub(in crate::infer) fn propagate_function_instances_for_owners_with_progress(
    hir: &mut PartialHir,
    owners: Option<&HashSet<ConstraintOwner>>,
) -> Result<AuthorityPassResult, Vec<ResolveError>> {
    let before_generation = hir.engine.rigid_substitution_generation();
    let before_counts = authority_counts(hir, owners);
    let mut constrained_vars = HashMap::new();
    // Callers that disagree keep the variable generic; it is generalized later.
    let conflicting = super::call_sites::conflicting_caller_vars(hir);
    for function in hir
        .functions
        .values()
        .chain(
            hir.traits
                .values()
                .flat_map(|trait_def| trait_def.methods.values()),
        )
        .chain(hir.impls.values().flat_map(|imp| imp.methods.values()))
    {
        let mut shared = constrained_signature_vars(hir, function);
        shared.retain(|var| !conflicting.contains(var));
        if function.generic_params.is_empty() {
            shared.extend(super::call_sites::field_receiver_vars(hir, function));
        }
        if !shared.is_empty() {
            constrained_vars.insert(function.id, shared);
        }
    }
    let mut errors = Vec::new();
    let mut function_ids = hir.functions.keys().copied().collect::<Vec<_>>();
    function_ids.sort();
    for id in &function_ids {
        if !owner_selected(owners, ConstraintOwner::Body(*id)) {
            continue;
        }
        if let Some(mut body) = hir.functions.get(id).map(|function| function.body.clone()) {
            propagate_block(hir, &mut body, &constrained_vars, &mut errors);
            if let Some(function) = hir.functions.get_mut(id) {
                let _ = hir.engine.unify(&function.ret_type, &body.ty);
                function.ret_type = hir.engine.resolve(&function.ret_type);
                function.body = body;
            }
        }
    }

    let mut impl_ids = hir.impls.keys().copied().collect::<Vec<_>>();
    impl_ids.sort();
    for impl_id in impl_ids {
        let mut method_names = hir.impls[&impl_id]
            .methods
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        method_names.sort();
        for method_name in method_names {
            let method_id = hir.impls[&impl_id].methods[&method_name].id;
            if !owner_selected(owners, ConstraintOwner::Body(method_id)) {
                continue;
            }
            if let Some(mut body) = hir
                .impls
                .get(&impl_id)
                .and_then(|imp| imp.methods.get(&method_name))
                .map(|method| method.body.clone())
            {
                propagate_block(hir, &mut body, &constrained_vars, &mut errors);
                if let Some(method) = hir
                    .impls
                    .get_mut(&impl_id)
                    .and_then(|imp| imp.methods.get_mut(&method_name))
                {
                    method.body = body;
                }
            }
        }
    }

    if errors.is_empty() {
        let mut result = authority_pass_result(hir, before_generation, before_counts, owners);
        // Instance propagation may create fresh call-site variables without
        // changing any authority; that is not observable progress for the
        // driver and must not keep waking the same call forever.
        result.progress = authority_counts(hir, owners) != before_counts;
        Ok(result)
    } else {
        Err(errors)
    }
}

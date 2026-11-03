use std::collections::HashMap;

use crate::hir::AcceptedHirProgram;
use crate::ids::DefId;
use crate::lower::ResolveError;

use super::partial_to_program::{canonical_names_for_hir, hir_program_from_partial};
use super::quiescence::drive_inference_to_quiescence;
use super::validation::validate_method_ids;
use super::{authority, constants, generalize, PartialHir, ResolvedHirProgram};

/// Finalize types: apply inference substitutions and generalize.
/// Consumes PartialHir and produces a fully-typed HirProgram.
pub fn finalize(hir: PartialHir) -> Result<ResolvedHirProgram, Vec<ResolveError>> {
    finalize_with_constant_methods(hir, HashMap::new())
}

pub(crate) fn finalize_with_crates(
    hir: PartialHir,
    ctx: &crate::crate_system::CrateContext,
) -> Result<ResolvedHirProgram, Vec<ResolveError>> {
    let mut summaries = HashMap::new();
    for dependency in ctx.extern_crates() {
        for imp in dependency.metadata().interface().impls.values() {
            for method in imp.methods.values() {
                if let Some(summary) = &method.constant_arithmetic {
                    summaries.insert(method.id, summary.clone());
                }
            }
        }
    }
    finalize_with_constant_methods(hir, summaries)
}

fn finalize_with_constant_methods(
    mut hir: PartialHir,
    summaries: HashMap<DefId, constants::ArithmeticSummary>,
) -> Result<ResolvedHirProgram, Vec<ResolveError>> {
    hir.validate_item_ids()?;
    validate_method_ids(&hir)?;
    drive_inference_to_quiescence(&mut hir)?;

    // Methods are not represented in the standalone function SCC map; finish
    // their component-local generalization after all body SCCs quiesce.
    generalize::generalize_methods_only(&mut hir);
    authority::finalize_method_instantiations(&mut hir)?;

    // Phase 6: finalize — replace all remaining TypeVars
    let errors = super::finalize::apply_finalization(&mut hir);
    if !errors.is_empty() {
        return Err(errors);
    }
    let canonical_names_by_id = canonical_names_for_hir(&hir);
    let normalization_env = hir.engine.normalization_env();
    let (program, resolver, current_def_ids, root_crate_id, local_def_ids, source_map) =
        hir_program_from_partial(hir, &canonical_names_by_id);
    let authority_errors = program.validate_method_authorities();
    if !authority_errors.is_empty() {
        return Err(authority_errors
            .into_iter()
            .map(ResolveError::non_source)
            .collect());
    }
    let summaries = constants::method_summaries(&program, summaries);
    let constant_errors = constants::validate(&program, &current_def_ids, &summaries);
    if !constant_errors.is_empty() {
        return Err(constant_errors);
    }
    let program = AcceptedHirProgram::try_from(program).map_err(|errors| {
        errors
            .into_iter()
            .map(ResolveError::non_source)
            .collect::<Vec<_>>()
    })?;
    Ok(ResolvedHirProgram::from_accepted_with_normalization_env(
        program,
        resolver,
        current_def_ids,
        root_crate_id,
        local_def_ids,
        normalization_env,
        source_map,
        summaries,
    ))
}

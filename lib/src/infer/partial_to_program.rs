use std::collections::{BTreeSet, HashMap};

use crate::collect::resolver::ResolverTables;
use crate::hir::{
    HirEnum, HirExtern, HirFunction, HirNameTables, HirProgram, HirStruct, HirTrait, HirTypeAlias,
};
use crate::ids::{DefId, IdGen, LocalDefId};

use super::PartialHir;

pub(super) fn canonical_names_for_hir(hir: &PartialHir) -> HashMap<DefId, String> {
    hir.resolver.item_names_by_id.clone()
}

fn hir_name_tables(
    resolver: &ResolverTables,
    functions: &HashMap<DefId, HirFunction>,
    structs: &HashMap<DefId, HirStruct>,
    enums: &HashMap<DefId, HirEnum>,
    traits: &HashMap<DefId, HirTrait>,
    externs: &HashMap<DefId, HirExtern>,
    type_aliases: &HashMap<DefId, HirTypeAlias>,
) -> HirNameTables {
    fn names_for<T>(resolver: &ResolverTables, ids: &HashMap<DefId, T>) -> HashMap<String, DefId> {
        let mut names = HashMap::new();
        for (name, id) in &resolver.item_paths {
            if ids.contains_key(id) {
                names.insert(name.clone(), *id);
            }
        }
        for (id, name) in &resolver.item_names_by_id {
            if ids.contains_key(id) {
                names.entry(name.clone()).or_insert(*id);
            }
        }
        for aliases in [
            &resolver.import_aliases,
            &resolver.export_aliases,
            &resolver.module_aliases,
        ] {
            for (name, id) in aliases {
                if ids.contains_key(id) {
                    names.entry(name.clone()).or_insert(*id);
                }
            }
        }
        names
    }

    HirNameTables {
        functions_by_name: names_for(resolver, functions),
        structs_by_name: names_for(resolver, structs),
        enums_by_name: names_for(resolver, enums),
        traits_by_name: names_for(resolver, traits),
        externs_by_name: names_for(resolver, externs),
        type_aliases_by_name: names_for(resolver, type_aliases),
    }
}

pub(super) fn hir_program_from_partial(
    hir: PartialHir,
    canonical_names_by_id: &HashMap<DefId, String>,
) -> (
    HirProgram,
    ResolverTables,
    BTreeSet<DefId>,
    crate::ids::CrateId,
    IdGen<LocalDefId>,
    crate::source_map::SemanticSourceMap,
) {
    let PartialHir {
        functions,
        structs,
        enums,
        traits,
        impls,
        externs,
        type_aliases,
        engine: _,
        function_type_vars: _,
        import_aliases: _,
        loaded_module_paths: _,
        constraint_store: _,
        resolver,
        current_def_ids,
        root_crate_id,
        local_def_ids,
        language_items,
        imported_effective_trait_methods,
        inference_sccs: _,
        inference_scc_order: _,
        source_map,
    } = hir;

    let names = hir_name_tables(
        &resolver,
        &functions,
        &structs,
        &enums,
        &traits,
        &externs,
        &type_aliases,
    );

    let mut program = HirProgram::from_id_parts_with_names_and_canonical_names(
        functions,
        structs,
        enums,
        traits,
        impls,
        externs,
        names,
        language_items,
        canonical_names_by_id,
    );
    program.type_aliases = type_aliases;
    program
        .indexes
        .effective_trait_methods
        .extend(imported_effective_trait_methods);

    (
        program,
        resolver,
        current_def_ids,
        root_crate_id,
        local_def_ids,
        source_map,
    )
}

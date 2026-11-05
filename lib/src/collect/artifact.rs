use super::member_ids::{collected_id_environment_from_index, populate_member_id_environment};
use super::source_discovery::{
    current_def_ids_from_item_index, source_modules_from_loaded_sources,
};
use super::validation::supertrait_operation_span;
use super::Declarations;
use super::{collector, context, language_items, DeclarationItems};
use crate::collect::item_index::{
    index_root_module_items_with_sources, source_module_map_from_loaded_modules, IndexingIds,
};
use crate::crate_system::CrateContext;
use crate::lower::ResolveError;
use std::collections::BTreeSet;

#[derive(Clone)]
pub struct ArtifactBootstrap {
    pub function_keys: BTreeSet<String>,
    pub struct_keys: BTreeSet<String>,
    pub enum_keys: BTreeSet<String>,
    pub trait_keys: BTreeSet<String>,
    pub externs_len: usize,
}

pub struct ArtifactDeclarations {
    pub declarations: Declarations,
    pub bootstrap: ArtifactBootstrap,
}

pub struct ArtifactHirDeclarations {
    pub declarations: Declarations,
    pub cross_crate_hir_bootstrap: ArtifactBootstrap,
}

pub fn collect_artifact_declarations(
    crate_ctx: &CrateContext,
    inject_prelude: bool,
    current_crate_name: &str,
) -> Result<ArtifactDeclarations, Vec<ResolveError>> {
    let source_crate = crate_ctx
        .source_crate(current_crate_name)
        .expect("artifact declaration collection requires loaded current source crate");
    let module = &source_crate.ast;
    let root_module_path = source_crate.root_module_path();
    let inject_prelude = inject_prelude && crate_ctx.has_extern_crate("std");
    let mut context = context::CollectContext::bootstrap_for_collection(
        inject_prelude,
        Some(current_crate_name),
        Some(&root_module_path),
    );

    context
        .module_file_cache
        .entry(root_module_path.clone())
        .or_insert_with(|| {
            let mut root_module = module.clone();
            root_module.filepath = Some(root_module_path.clone());
            root_module
        });
    for (qualified_name, path) in &source_crate.loaded_module_paths {
        if !context
            .loaded_module_paths
            .iter()
            .any(|(name, existing_path)| name == qualified_name && existing_path == path)
        {
            context
                .loaded_module_paths
                .push((qualified_name.clone(), path.clone()));
        }
    }
    for (path, cached_module) in &source_crate.file_cache {
        context
            .module_file_cache
            .entry(path.clone())
            .or_insert_with(|| cached_module.clone());
    }

    for dep in crate_ctx.extern_crates() {
        if crate_ctx.is_current_source_self_artifact(Some(current_crate_name), dep.name()) {
            continue;
        }

        context.register_extern_crate(dep);
    }

    let artifact_bootstrap = ArtifactBootstrap {
        function_keys: context.functions.keys().cloned().collect(),
        struct_keys: context.structs.keys().cloned().collect(),
        enum_keys: context.enums.keys().cloned().collect(),
        trait_keys: context.traits.keys().cloned().collect(),
        externs_len: context.externs.len(),
    };

    if inject_prelude {
        context.inject_loaded_prelude(crate_ctx);
    }
    let mut indexing_ids = IndexingIds::new_root();
    let source_modules = source_module_map_from_loaded_modules(
        module,
        Some(current_crate_name),
        &context.loaded_module_paths,
        &context.module_file_cache,
    );
    let item_index = index_root_module_items_with_sources(
        &mut indexing_ids,
        module,
        Some(current_crate_name),
        &source_modules,
    );
    let root_module_id = indexing_ids.root_module_id();
    let mut id_environment = collected_id_environment_from_index(&item_index, root_module_id);
    populate_member_id_environment(
        &mut id_environment,
        &mut indexing_ids,
        &item_index,
        module,
        Some(current_crate_name),
        &source_modules,
    );
    let member_def_ids = id_environment.member_def_ids();
    let provided = language_items::bind_current_language_items(
        module,
        &item_index,
        &id_environment,
        Some(current_crate_name),
        &source_modules,
    )?;
    let language_items = language_items::merge_provided_language_items(
        Some(current_crate_name),
        &provided,
        crate_ctx,
    )?;
    context.language_items = language_items.clone();
    context.register_declared_items(&item_index, root_module_id, Some(current_crate_name));

    let mut collector = collector::LocalCollector::new_with_id_environment(context, id_environment);
    collector.collect_crate_declarations(
        module,
        root_module_id,
        current_crate_name,
        current_crate_name == "std",
    );

    let context::LocalCollection {
        structs,
        enums,
        traits,
        impls,
        externs,
        functions,
        function_sigs,
        type_aliases,
        function_type_vars,
        infix_precedence,
        loaded_module_paths,
        type_vars,
        loaded_prelude_export_ids,
        module_file_cache,
        dependency_root_export_ids,
        language_items,
        resolver,
        mut errors,
    } = collector.finish(
        &item_index,
        indexing_ids.root_module_id(),
        Some(current_crate_name),
    );

    errors.extend(
        crate::types::validate_supertrait_graph(traits.values().map(|trait_def| {
            (
                trait_def.id,
                trait_def.name.as_str(),
                trait_def.predicates.as_slice(),
            )
        }))
        .into_iter()
        .map(|message| {
            ResolveError::from_optional_span_code(
                message.clone(),
                supertrait_operation_span(module, &message, &source_modules, &[]),
                crate::diagnostic::DiagnosticCode::Type,
            )
        }),
    );

    if !errors.is_empty() {
        return Err(errors);
    }

    let mut current_def_ids = current_def_ids_from_item_index(&item_index);
    current_def_ids.extend(member_def_ids);
    let items = DeclarationItems::from_named_maps_with_aliases(
        functions,
        function_sigs,
        structs,
        enums,
        traits,
        impls,
        externs,
        type_aliases,
        &resolver.item_names_by_id,
    )?;

    let source_map = crate::source_map::SemanticSourceMap::from_declarations(
        &item_index,
        module,
        &items,
        &source_modules,
        &resolver,
    );
    Ok(ArtifactDeclarations {
        declarations: Declarations {
            indexing_ids,
            item_index,
            resolver,
            current_def_ids,
            language_items,
            items,
            function_type_vars,
            infix_precedence,
            source_modules: source_modules_from_loaded_sources(
                &loaded_module_paths,
                &module_file_cache,
            ),
            loaded_module_paths,
            type_vars,
            inject_prelude,
            loaded_prelude_export_ids,
            module_file_cache,
            dependency_root_export_ids,
            source_map,
        },
        bootstrap: artifact_bootstrap,
    })
}

pub fn collect_artifact_hir_declarations(
    crate_ctx: &CrateContext,
    inject_prelude: bool,
    current_crate_name: &str,
) -> Result<ArtifactHirDeclarations, Vec<ResolveError>> {
    let artifact = collect_artifact_declarations(crate_ctx, inject_prelude, current_crate_name)?;

    Ok(ArtifactHirDeclarations {
        declarations: artifact.declarations,
        cross_crate_hir_bootstrap: artifact.bootstrap,
    })
}

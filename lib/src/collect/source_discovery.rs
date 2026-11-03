use super::context;
use crate::ast;
use crate::collect::item_index::ItemIndex;
use crate::lower::ResolveError;
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

pub(super) fn discover_source_modules_for_indexing(
    context: &mut context::CollectContext,
    module: &ast::Module,
) -> Result<(), Vec<ResolveError>> {
    let mut errors = Vec::new();
    discover_source_modules_in_context(context, module, None, &mut errors);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn discover_source_modules_in_context(
    context: &mut context::CollectContext,
    module: &ast::Module,
    local_prefix: Option<&str>,
    errors: &mut Vec<ResolveError>,
) {
    for top_level in &module.top_levels {
        match top_level {
            ast::TopLevel::Module(module_decl) => {
                let inner_module = &module_decl.0;
                if let Some(ref mod_name) = inner_module.name {
                    let next_prefix = local_prefix
                        .map(|prefix| format!("{}::{}", prefix, mod_name.name))
                        .unwrap_or_else(|| mod_name.name.clone());
                    discover_source_modules_in_context(
                        context,
                        inner_module,
                        Some(&next_prefix),
                        errors,
                    );
                } else {
                    discover_source_modules_in_context(context, inner_module, local_prefix, errors);
                }
            }
            ast::TopLevel::Mod(ident, _) => {
                discover_source_module_for_indexing(context, ident, local_prefix, errors);
            }
            _ => {}
        }
    }
}

fn discover_source_module_for_indexing(
    context: &mut context::CollectContext,
    ident: &ast::Ident,
    prefix: Option<&str>,
    errors: &mut Vec<ResolveError>,
) {
    let module_name = &ident.name;
    let effective_prefix = prefix
        .map(ToString::to_string)
        .or_else(|| context.current_crate_name.clone());
    let qualified_module_name = effective_prefix
        .as_deref()
        .map(|prefix| format!("{}::{}", prefix, module_name))
        .unwrap_or_else(|| module_name.clone());

    let Some(file_path) =
        context.loaded_module_path_for_prefix(module_name, effective_prefix.as_deref())
    else {
        errors.push(ResolveError::with_span(
            format!(
                "Module '{}' was not loaded by the source database",
                qualified_module_name
            ),
            ident.span.clone(),
        ));
        return;
    };

    let loaded_module =
        match context.load_module_with_prefix(module_name, effective_prefix.as_deref()) {
            Ok(module) => module,
            Err(err) => {
                errors.push(ResolveError::with_span(err, ident.span.clone()));
                return;
            }
        };

    let old_path = context.current_module_path.clone();
    context.current_module_path = file_path;
    discover_source_modules_in_context(
        context,
        &loaded_module,
        Some(&qualified_module_name),
        errors,
    );
    context.current_module_path = old_path;
}

pub(super) fn source_modules_from_loaded_sources(
    loaded_module_paths: &[(String, PathBuf)],
    module_file_cache: &HashMap<PathBuf, ast::Module>,
) -> crate::source_loader::SourceModuleSet {
    crate::source_loader::SourceModuleSet::from_modules(
        loaded_module_paths
            .iter()
            .enumerate()
            .filter_map(|(index, (qualified_name, path))| {
                let module = module_file_cache.get(path).cloned()?;
                Some(crate::source_loader::LoadedModule {
                    id: crate::source_loader::ModuleId(index),
                    qualified_name: qualified_name.clone(),
                    path: path.clone(),
                    canonical_path: path.clone(),
                    module,
                })
            }),
    )
}

pub(super) fn source_modules_for_lowering(
    source_graph: Option<&crate::source_loader::ModuleGraph>,
    loaded_module_paths: &[(String, PathBuf)],
    module_file_cache: &HashMap<PathBuf, ast::Module>,
) -> crate::source_loader::SourceModuleSet {
    let mut source_modules = source_graph
        .map(crate::source_loader::ModuleGraph::source_modules)
        .unwrap_or_default();

    for module in
        source_modules_from_loaded_sources(loaded_module_paths, module_file_cache).modules()
    {
        if source_modules.module_for_path(&module.path).is_none()
            && source_modules
                .module_by_qualified_name(&module.qualified_name)
                .is_none()
        {
            source_modules.insert(module.clone());
        }
    }

    source_modules
}

pub(super) fn current_def_ids_from_item_index(
    item_index: &ItemIndex,
) -> BTreeSet<crate::ids::DefId> {
    item_index.items().iter().map(|item| item.def_id).collect()
}

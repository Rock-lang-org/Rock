//! First-pass declaration collection.
//!
//! Gathers all top-level declarations (structs, enums, trait defs, function
//! signatures) without lowering function bodies. Produces `Declarations`
//! which is consumed by the `lower` stage.

mod artifact;
mod collector;
mod context;
mod declarations;
mod headers;
pub mod item_index;
mod language_items;
mod member_ids;
pub mod resolver;
mod scope;
mod source_discovery;
mod validation;

use crate::ast;
use crate::collect::item_index::{
    index_root_module_items_with_sources, source_module_map_from_graph,
    source_module_map_from_loaded_modules, IndexingIds, ItemIndex, ItemRecord, ItemSourceId,
};
use crate::collect::resolver::ResolverTables;
use crate::crate_artifact::ArtifactExport;
use crate::crate_system::CrateContext;
use crate::ids::DefId;
use crate::language_items::LanguageItems;
use crate::lower::ResolveError;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

pub use artifact::{
    collect_artifact_declarations, collect_artifact_hir_declarations, ArtifactBootstrap,
    ArtifactDeclarations, ArtifactHirDeclarations,
};
use member_ids::{collected_id_environment_from_index, populate_member_id_environment};
use source_discovery::{
    current_def_ids_from_item_index, discover_source_modules_for_indexing,
    source_modules_for_lowering,
};
use validation::{supertrait_operation_span, validate_supported_current_crate_ids};

pub use declarations::DeclarationItems;

pub(crate) use scope::DeclarationScope;

pub use scope::DeclarationTypeVars;

/// All top-level declarations gathered during the first pass.
/// Consumed by `lower::lower()`.
pub struct Declarations {
    /// IDs allocated for the currently indexed root crate/module.
    ///
    /// Later resolver work should use this context instead of reconstructing
    /// root IDs from literals or item-index internals.
    pub indexing_ids: IndexingIds,
    /// ID-backed index for source items discovered in the root module.
    ///
    pub item_index: ItemIndex,
    pub resolver: ResolverTables,
    pub current_def_ids: BTreeSet<crate::ids::DefId>,
    /// Current-source language item identities bound from explicit markers.
    pub language_items: LanguageItems<DefId>,
    /// Functions have headers but empty bodies until lower fills them in.
    pub items: DeclarationItems,
    pub function_type_vars: HashMap<DefId, HashSet<crate::ids::TypeVarId>>,
    pub infix_precedence: HashMap<String, u8>,
    /// Modules that need body lowering: (module_name, file_path)
    pub loaded_module_paths: Vec<(String, PathBuf)>,
    /// Collection-owned type variable allocator seed so lower can continue
    /// creating fresh type vars without ID conflicts.
    pub type_vars: DeclarationTypeVars,
    /// Whether to inject loaded prelude capabilities.
    pub inject_prelude: bool,
    pub loaded_prelude_export_ids: HashMap<String, ArtifactExport>,
    /// Pre-parsed crate sub-module ASTs from current source crate loading.
    /// Carried through to the lower phase so it can serve file loads from cache.
    pub module_file_cache: HashMap<std::path::PathBuf, crate::ast::Module>,
    pub source_modules: crate::source_loader::SourceModuleSet,
    pub dependency_root_export_ids: HashMap<String, HashMap<String, ArtifactExport>>,
    pub source_map: crate::source_map::SemanticSourceMap,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CollectedTraitMemberIds {
    methods: HashMap<String, crate::ids::DefId>,
    signatures: HashMap<String, crate::ids::DefId>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CollectedIdEnvironment {
    current_source_items: HashMap<ItemSourceId, ItemRecord>,
    current_source_module_paths: HashMap<Vec<String>, crate::ids::ModuleId>,
    trait_member_ids_by_owner: HashMap<crate::ids::DefId, CollectedTraitMemberIds>,
    impl_method_ids_by_owner: HashMap<crate::ids::DefId, HashMap<String, crate::ids::DefId>>,
}

impl CollectedIdEnvironment {
    pub(crate) fn item_at_source(
        &self,
        module_id: crate::ids::ModuleId,
        top_level_index: usize,
    ) -> Option<&ItemRecord> {
        self.current_source_items.get(&ItemSourceId {
            module_id,
            top_level_index: top_level_index.try_into().ok()?,
        })
    }

    pub(crate) fn child_module_id(
        &self,
        parent_id: crate::ids::ModuleId,
        name: &str,
    ) -> Option<crate::ids::ModuleId> {
        let mut path = self
            .current_source_module_paths
            .iter()
            .find_map(|(path, module_id)| (*module_id == parent_id).then(|| path.clone()))?;
        path.push(name.to_string());
        self.current_source_module_paths.get(&path).copied()
    }

    fn member_def_ids(&self) -> BTreeSet<crate::ids::DefId> {
        let mut ids = BTreeSet::new();
        for member_ids in self.trait_member_ids_by_owner.values() {
            ids.extend(member_ids.methods.values().copied());
            ids.extend(member_ids.signatures.values().copied());
        }
        for method_ids in self.impl_method_ids_by_owner.values() {
            ids.extend(method_ids.values().copied());
        }
        ids
    }
}

/// Run the first-pass declaration collection.
///
/// Builds collect-owned bootstrap state, gathers declarations, and extracts
/// the resulting state into `Declarations`.
pub fn collect(
    program: &ast::Program,
    crate_ctx: &CrateContext,
    inject_prelude: bool,
    current_crate_name: Option<&str>,
) -> Result<Declarations, Vec<ResolveError>> {
    collect_impl(program, None, crate_ctx, inject_prelude, current_crate_name)
}

pub fn collect_with_source_graph(
    program: &ast::Program,
    source_graph: &crate::source_loader::ModuleGraph,
    crate_ctx: &CrateContext,
    inject_prelude: bool,
    current_crate_name: Option<&str>,
) -> Result<Declarations, Vec<ResolveError>> {
    collect_impl(
        program,
        Some(source_graph),
        crate_ctx,
        inject_prelude,
        current_crate_name,
    )
}

fn collect_impl(
    program: &ast::Program,
    source_graph: Option<&crate::source_loader::ModuleGraph>,
    crate_ctx: &CrateContext,
    inject_prelude: bool,
    current_crate_name: Option<&str>,
) -> Result<Declarations, Vec<ResolveError>> {
    let inject_prelude = inject_prelude && crate_ctx.has_extern_crate("std");
    let mut context = context::CollectContext::bootstrap_for_collection(
        inject_prelude,
        current_crate_name,
        program.module.filepath.as_ref(),
    );
    if let Some(source_graph) = source_graph {
        context.seed_source_graph(source_graph);
    }

    context.register_crate_functions(crate_ctx);
    if inject_prelude {
        context.inject_loaded_prelude(crate_ctx);
    }
    discover_source_modules_for_indexing(&mut context, &program.module)?;

    let mut indexing_ids = IndexingIds::new_root();
    let source_modules = if let Some(source_graph) = source_graph {
        let mut source_modules = source_module_map_from_graph(source_graph);
        source_modules.extend(source_module_map_from_loaded_modules(
            &program.module,
            current_crate_name,
            &context.loaded_module_paths,
            &context.module_file_cache,
        ));
        source_modules
    } else {
        source_module_map_from_loaded_modules(
            &program.module,
            current_crate_name,
            &context.loaded_module_paths,
            &context.module_file_cache,
        )
    };
    let item_index = index_root_module_items_with_sources(
        &mut indexing_ids,
        &program.module,
        current_crate_name,
        &source_modules,
    );
    let root_module_id = indexing_ids.root_module_id();
    let mut id_environment = collected_id_environment_from_index(&item_index, root_module_id);
    populate_member_id_environment(
        &mut id_environment,
        &mut indexing_ids,
        &item_index,
        &program.module,
        current_crate_name,
        &source_modules,
    );
    let member_def_ids = id_environment.member_def_ids();
    let provided = language_items::bind_current_language_items(
        &program.module,
        &item_index,
        &id_environment,
        current_crate_name,
        &source_modules,
    )?;
    let language_items =
        language_items::merge_provided_language_items(current_crate_name, &provided, crate_ctx)?;
    context.language_items = language_items.clone();

    let mut collector = collector::LocalCollector::new_with_id_environment(context, id_environment);

    collector.collect_local_declarations(&program.module, root_module_id);
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
        current_crate_name,
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
                supertrait_operation_span(&program.module, &message, &source_modules, &[]),
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
        &program.module,
        &items,
        &source_modules,
        &resolver,
    );
    let decls = Declarations {
        indexing_ids,
        item_index,
        resolver,
        current_def_ids,
        language_items,
        items,
        function_type_vars,
        infix_precedence,
        source_modules: source_modules_for_lowering(
            source_graph,
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
    };
    validate_supported_current_crate_ids(&decls)?;

    Ok(decls)
}

#[cfg(test)]
mod tests;

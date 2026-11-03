//! First-pass declaration collection.
//!
//! Gathers all top-level declarations (structs, enums, trait defs, function
//! signatures) without lowering function bodies. Produces `Declarations`
//! which is consumed by the `lower` stage.

mod collector;
mod context;
mod declarations;
mod headers;
pub mod item_index;
mod language_items;
pub mod resolver;
mod scope;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

use crate::ast;
use crate::collect::item_index::{
    index_root_module_items_with_sources, source_module_map_from_graph,
    source_module_map_from_loaded_modules, IndexingIds, ItemIndex, ItemKind, ItemRecord,
    ItemSourceId,
};
use crate::collect::resolver::ResolverTables;
use crate::crate_artifact::ArtifactExport;
use crate::crate_system::CrateContext;
use crate::ids::DefId;
use crate::language_items::LanguageItems;
use crate::lower::ResolveError;

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

#[derive(Clone)]
pub struct ArtifactBootstrap {
    pub function_keys: BTreeSet<String>,
    pub struct_keys: BTreeSet<String>,
    pub enum_keys: BTreeSet<String>,
    pub trait_keys: BTreeSet<String>,
    pub externs_len: usize,
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

#[derive(Debug)]
struct TraitMemberSource {
    trait_id: crate::ids::DefId,
    trait_path: String,
    method_names: Vec<String>,
    signature_names: Vec<String>,
}

#[derive(Debug)]
struct ImplMemberSource {
    impl_id: crate::ids::DefId,
    method_names: Vec<String>,
}

fn canonicalize_import_alias_targets(
    import_aliases: &mut HashMap<String, String>,
    export_aliases: &HashMap<String, String>,
    current_crate_name: Option<&str>,
) {
    for target in import_aliases.values_mut() {
        if let Some(canonical_source) =
            resolve_export_alias_source(target.as_str(), export_aliases, current_crate_name)
        {
            *target = canonical_source;
        }
    }
}

fn resolve_export_alias_source(
    target: &str,
    export_aliases: &HashMap<String, String>,
    current_crate_name: Option<&str>,
) -> Option<String> {
    let direct = export_aliases.get(target).cloned();
    if direct.is_some() {
        return direct;
    }

    let crate_name = current_crate_name?;
    let root = target.split("::").next().unwrap_or(target);
    if root == crate_name {
        return None;
    }

    export_aliases
        .get(&format!("{}::{}", crate_name, target))
        .cloned()
}

pub struct ArtifactDeclarations {
    pub declarations: Declarations,
    pub bootstrap: ArtifactBootstrap,
}

pub struct ArtifactHirDeclarations {
    pub declarations: Declarations,
    pub cross_crate_hir_bootstrap: ArtifactBootstrap,
}

fn collected_id_environment_from_index(
    item_index: &ItemIndex,
    root_module_id: crate::ids::ModuleId,
) -> CollectedIdEnvironment {
    let current_source_items = item_index
        .items()
        .iter()
        .map(|item| (item.source, item.clone()))
        .collect();
    let mut current_source_module_paths = HashMap::from([(Vec::new(), root_module_id)]);
    while current_source_module_paths.len() < item_index.modules().len() {
        let mut inserted = false;
        for module in item_index.modules() {
            let (Some(parent), Some(name)) = (module.parent, module.name.as_ref()) else {
                continue;
            };
            let Some(parent_path) = current_source_module_paths
                .iter()
                .find_map(|(path, module_id)| (*module_id == parent).then(|| path.clone()))
            else {
                continue;
            };
            let mut path = parent_path;
            path.push(name.clone());
            inserted |= current_source_module_paths
                .insert(path, module.module_id)
                .is_none();
        }
        if !inserted {
            break;
        }
    }

    CollectedIdEnvironment {
        current_source_items,
        current_source_module_paths,
        trait_member_ids_by_owner: HashMap::new(),
        impl_method_ids_by_owner: HashMap::new(),
    }
}

fn populate_member_id_environment(
    env: &mut CollectedIdEnvironment,
    ids: &mut IndexingIds,
    item_index: &ItemIndex,
    module: &ast::Module,
    current_crate_name: Option<&str>,
    source_modules: &crate::collect::item_index::SourceModuleMap,
) {
    let root_module_id = item_index
        .module_id_by_path(&[])
        .expect("indexed collection must have a root module");
    let mut module_path = current_crate_name
        .map(|name| vec![name.to_string()])
        .unwrap_or_default();
    let mut trait_sources = Vec::new();
    let mut impl_sources = Vec::new();

    collect_member_sources_in_module(
        module,
        root_module_id,
        &mut module_path,
        source_modules,
        item_index,
        &mut trait_sources,
        &mut impl_sources,
    );

    trait_sources.sort_by(|left, right| {
        left.trait_path
            .cmp(&right.trait_path)
            .then(left.trait_id.cmp(&right.trait_id))
    });

    for source in trait_sources {
        let mut member_ids = CollectedTraitMemberIds::default();
        let mut shared_member_ids = HashMap::new();

        for method_name in sorted_names(source.method_names) {
            let id = *shared_member_ids
                .entry(method_name.clone())
                .or_insert_with(|| ids.fresh_def_id());
            member_ids.methods.insert(method_name, id);
        }

        for signature_name in sorted_names(source.signature_names) {
            let id = *shared_member_ids
                .entry(signature_name.clone())
                .or_insert_with(|| ids.fresh_def_id());
            member_ids.signatures.insert(signature_name, id);
        }

        env.trait_member_ids_by_owner
            .insert(source.trait_id, member_ids);
    }

    for source in impl_sources {
        let mut method_ids = HashMap::new();
        for method_name in sorted_names(source.method_names) {
            method_ids.insert(method_name, ids.fresh_def_id());
        }
        env.impl_method_ids_by_owner
            .insert(source.impl_id, method_ids);
    }
}

fn collect_member_sources_in_module(
    module: &ast::Module,
    module_id: crate::ids::ModuleId,
    module_path: &mut Vec<String>,
    source_modules: &crate::collect::item_index::SourceModuleMap,
    item_index: &ItemIndex,
    trait_sources: &mut Vec<TraitMemberSource>,
    impl_sources: &mut Vec<ImplMemberSource>,
) {
    for (top_level_index, top_level) in module.top_levels.iter().enumerate() {
        match top_level {
            ast::TopLevel::Module(module_decl) => {
                let inner = &module_decl.0;
                if let Some(module_name) = &inner.name {
                    let Some(child_module_id) =
                        item_index.child_module_id(module_id, &module_name.name)
                    else {
                        continue;
                    };
                    module_path.push(module_name.name.clone());
                    collect_member_sources_in_module(
                        inner,
                        child_module_id,
                        module_path,
                        source_modules,
                        item_index,
                        trait_sources,
                        impl_sources,
                    );
                    module_path.pop();
                }
            }
            ast::TopLevel::Mod(name, _) => {
                let Some(child_module_id) = item_index.child_module_id(module_id, &name.name)
                else {
                    continue;
                };
                module_path.push(name.name.clone());
                if let Some(source_module) = source_modules.get(&*module_path) {
                    collect_member_sources_in_module(
                        source_module,
                        child_module_id,
                        module_path,
                        source_modules,
                        item_index,
                        trait_sources,
                        impl_sources,
                    );
                }
                module_path.pop();
            }
            ast::TopLevel::TraitDecl(trait_decl) => {
                if let Some(owner) = item_index
                    .item_at_source(module_id, top_level_index)
                    .filter(|item| item.kind == ItemKind::Trait)
                {
                    trait_sources.push(TraitMemberSource {
                        trait_id: owner.def_id,
                        trait_path: qualified_member_owner_path(module_path, &trait_decl.name.name),
                        method_names: trait_decl
                            .methods
                            .keys()
                            .map(|ident| ident.name.clone())
                            .collect(),
                        signature_names: trait_decl
                            .signatures
                            .keys()
                            .map(|ident| ident.name.clone())
                            .collect(),
                    });
                }
            }
            ast::TopLevel::Impl(impl_decl) => {
                if let Some(owner) = item_index
                    .item_at_source(module_id, top_level_index)
                    .filter(|item| item.kind == ItemKind::Impl)
                {
                    impl_sources.push(ImplMemberSource {
                        impl_id: owner.def_id,
                        method_names: impl_decl
                            .methods
                            .keys()
                            .map(|ident| ident.name.clone())
                            .collect(),
                    });
                }
            }
            _ => {}
        }
    }
}

fn qualified_member_owner_path(module_path: &[String], name: &str) -> String {
    if module_path.is_empty() {
        name.to_string()
    } else {
        format!("{}::{}", module_path.join("::"), name)
    }
}

fn sorted_names(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names.dedup();
    names
}

fn discover_source_modules_for_indexing(
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

fn validate_supported_current_crate_ids(decls: &Declarations) -> Result<(), Vec<ResolveError>> {
    fn push_missing(errors: &mut Vec<ResolveError>, kind: &str, name: &str, id: crate::ids::DefId) {
        if id.crate_id == crate::ids::CrateId(u32::MAX) {
            errors.push(ResolveError::non_source(format!(
                "missing canonical {kind} identity for {name}"
            )));
        }
    }

    let mut errors = Vec::new();

    for function in decls.items.functions().values() {
        push_missing(&mut errors, "function", &function.name, function.id);
        for generic in &function.generic_params {
            push_missing(
                &mut errors,
                "function generic owner",
                &function.name,
                generic.id.owner,
            );
        }
    }
    for sig in decls.items.function_sigs().values() {
        push_missing(&mut errors, "function signature", &sig.name, sig.id);
        for generic in &sig.generic_params {
            push_missing(
                &mut errors,
                "signature generic owner",
                &sig.name,
                generic.id.owner,
            );
        }
    }
    for structure in decls.items.structs().values() {
        push_missing(&mut errors, "struct", &structure.name, structure.id);
    }
    for enumeration in decls.items.enums().values() {
        push_missing(&mut errors, "enum", &enumeration.name, enumeration.id);
    }
    for trait_def in decls.items.traits().values() {
        push_missing(&mut errors, "trait", &trait_def.name, trait_def.id);
        for (method_name, method) in &trait_def.methods {
            push_missing(
                &mut errors,
                "trait method",
                &format!("{}.{}", trait_def.name, method_name),
                method.id,
            );
        }
        for (signature_name, signature) in &trait_def.signatures {
            push_missing(
                &mut errors,
                "trait signature",
                &format!("{}.{}", trait_def.name, signature_name),
                signature.id,
            );
        }
    }
    for imp in decls.items.impls().values() {
        push_missing(&mut errors, "impl", &imp.type_name, imp.id);
        for (method_name, method) in &imp.methods {
            push_missing(
                &mut errors,
                "impl method",
                &format!("{}.{}", imp.type_name, method_name),
                method.id,
            );
        }
    }
    for ext in decls.items.externs().values() {
        push_missing(&mut errors, "extern", &ext.name, ext.id);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn supertrait_operation_span(
    module: &ast::Module,
    message: &str,
    source_modules: &crate::collect::item_index::SourceModuleMap,
    prefix: &[String],
) -> Option<crate::lexer::Span> {
    let owner = message.split('\'').nth(1).or_else(|| {
        message
            .strip_prefix("supertrait cycle detected: ")
            .and_then(|path| path.split(" -> ").next())
    });
    for top_level in &module.top_levels {
        if let ast::TopLevel::TraitDecl(trait_decl) = top_level {
            if owner.is_some_and(|owner| {
                owner == trait_decl.name.name
                    || owner.rsplit("::").next() == Some(trait_decl.name.name.as_str())
            }) {
                if let Some(span) = trait_decl
                    .where_clauses
                    .iter()
                    .find_map(|clause| clause.trait_bound.as_ref().map(ast::ParseType::span))
                {
                    return Some(span);
                }
                return Some(trait_decl.name.span.clone());
            }
        }
    }

    for top_level in &module.top_levels {
        let ast::TopLevel::Module(ast::ModuleDecl(inline)) = top_level else {
            continue;
        };
        let Some(name) = inline.name.as_ref() else {
            continue;
        };
        let mut path = prefix.to_vec();
        path.push(name.name.clone());
        if let Some(span) = supertrait_operation_span(inline, message, source_modules, &path) {
            return Some(span);
        }
    }
    for top_level in &module.top_levels {
        let ast::TopLevel::Mod(ident, _) = top_level else {
            continue;
        };
        let mut path = prefix.to_vec();
        path.push(ident.name.clone());
        let Some(child) = source_modules.get(&path) else {
            continue;
        };
        if let Some(span) = supertrait_operation_span(child, message, source_modules, &path) {
            return Some(span);
        }
    }
    None
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

fn source_modules_from_loaded_sources(
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

fn source_modules_for_lowering(
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

fn current_def_ids_from_item_index(item_index: &ItemIndex) -> BTreeSet<crate::ids::DefId> {
    item_index.items().iter().map(|item| item.def_id).collect()
}

#[cfg(test)]
pub(crate) struct CurrentCrateCanonicalFixture {
    pub program: ast::Program,
    pub graph: crate::source_loader::ModuleGraph,
    temp_dir: PathBuf,
}

#[cfg(test)]
impl Drop for CurrentCrateCanonicalFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.temp_dir);
    }
}

#[cfg(test)]
pub(crate) fn current_crate_canonical_fixture() -> CurrentCrateCanonicalFixture {
    let temp_dir = std::env::temp_dir().join(format!(
        "rock_collect_canonical_paths_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let root_path = temp_dir.join("main.rk");
    let io_path = temp_dir.join("io.rk");
    std::fs::write(&root_path, "mod io\n").unwrap();
    std::fs::write(&io_path, "struct Writer\n").unwrap();
    let config = crate::Config {
        entry_file: root_path.clone(),
        no_std: true,
        no_prelude: true,
        current_crate_name: Some("test".to_string()),
        ..crate::Config::default()
    };
    let mut db = crate::source_loader::SourceDatabase::new();
    let graph = db.load_entry(root_path.clone(), &config).unwrap();

    CurrentCrateCanonicalFixture {
        program: ast::Program {
            module: ast::Module {
                name: None,
                top_levels: vec![
                    ast::TopLevel::StructDecl(ast::StructDecl {
                        name: ast::ParseTypeInner {
                            name: "RootThing".to_string(),
                            generics: vec![],
                            span: crate::lexer::Span::test(),
                        },
                        generic_params: vec![],
                        fields: vec![],
                        exported: false,
                    }),
                    ast::TopLevel::Module(ast::ModuleDecl(ast::Module {
                        name: Some(ast::Ident {
                            name: "math".to_string(),
                            span: crate::lexer::Span::test(),
                        }),
                        top_levels: vec![ast::TopLevel::StructDecl(ast::StructDecl {
                            name: ast::ParseTypeInner {
                                name: "Vector".to_string(),
                                generics: vec![],
                                span: crate::lexer::Span::test(),
                            },
                            generic_params: vec![],
                            fields: vec![],
                            exported: false,
                        })],
                        is_inline: true,
                        filepath: None,
                    })),
                    ast::TopLevel::Mod(
                        ast::Ident {
                            name: "io".to_string(),
                            span: crate::lexer::Span::test(),
                        },
                        false,
                    ),
                ],
                is_inline: false,
                filepath: Some(root_path),
            },
        },
        graph,
        temp_dir,
    }
}

#[cfg(test)]
mod tests;

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
mod objects;
pub mod resolver;
mod scope;
pub(crate) mod sizedness;

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
    let inject_prelude = inject_prelude && crate_ctx.has_extern_crate("stdlib");
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
    let inject_prelude = inject_prelude && crate_ctx.has_extern_crate("stdlib");
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
        current_crate_name == "stdlib",
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
mod tests {
    use super::*;

    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use crate::ast::{
        FunctionDecl, FunctionSig, Ident, Impl, LambdaArrowKind, LambdaDecl, Module, ModuleDecl,
        ParseType, ParseTypeInner, Pattern, PatternKind, Program, StructDecl, TopLevel,
    };
    use crate::crate_artifact::{ArtifactCrateInterface, ArtifactExport};
    use crate::hir::{
        HirBlock, HirEnum, HirFunction, HirFunctionSig, HirMethodLocation, HirStruct, HirTrait,
    };
    use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId, ModuleId, VariantId};
    use crate::language_items::{
        DropLanguageItems, LanguageItems, SizedLanguageItems, TryLanguageItems,
    };
    use crate::lexer::Span;
    use crate::products::{
        CompilerProducts, ProductCrateIdentity, ProductDefId, ProductLinkData,
        ProductSourceFingerprint,
    };
    use crate::type_services::kind::Kind;
    use crate::types::{GenericParamDecl, GenericParamId, Type};

    fn type_inner(name: &str) -> ParseTypeInner {
        ParseTypeInner {
            name: name.to_string(),
            generics: vec![],
            span: Span::test(),
        }
    }

    fn generic_type_inner(name: &str, generics: Vec<ParseType>) -> ParseTypeInner {
        ParseTypeInner {
            name: name.to_string(),
            generics,
            span: Span::test(),
        }
    }

    fn generic_type(name: &str, generics: Vec<ParseType>) -> ParseType {
        ParseType::Type(generic_type_inner(name, generics))
    }

    fn struct_field(name: &str, ty: ParseType) -> crate::ast::StructDeclField {
        crate::ast::StructDeclField {
            name: ident(name),
            ty,
            public: false,
            default: None,
        }
    }

    fn ident(name: &str) -> Ident {
        Ident {
            name: name.to_string(),
            span: Span::test(),
        }
    }

    fn program_with_struct(name: &str) -> Program {
        Program {
            module: Module {
                name: None,
                top_levels: vec![TopLevel::StructDecl(StructDecl {
                    name: type_inner(name),
                    generic_params: vec![],
                    fields: vec![],
                    exported: false,
                })],
                is_inline: false,
                filepath: None,
            },
        }
    }

    fn inline_module(name: &str, top_levels: Vec<TopLevel>) -> TopLevel {
        TopLevel::Module(ModuleDecl(Module {
            name: Some(ident(name)),
            top_levels,
            is_inline: true,
            filepath: None,
        }))
    }

    fn load_source_program(
        entry: PathBuf,
        source: &str,
        current_crate_name: Option<&str>,
    ) -> (Program, crate::source_loader::ModuleGraph) {
        std::fs::write(&entry, source).unwrap();
        let config = crate::Config {
            entry_file: entry.clone(),
            no_std: true,
            no_prelude: true,
            current_crate_name: current_crate_name.map(ToString::to_string),
            ..crate::Config::default()
        };
        let mut db = crate::source_loader::SourceDatabase::new();
        let graph = db.load_entry(entry, &config).unwrap();
        let program = Program {
            module: graph.root_module().clone(),
        };
        (program, graph)
    }

    #[test]
    fn collect_uses_source_graph_for_source_backed_modules() {
        let temp_dir =
            std::env::temp_dir().join(format!("rock_collect_source_graph_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let entry = temp_dir.join("main.rk");
        std::fs::write(&entry, "mod util\n> util::answer\nmain = -> answer!\n").unwrap();
        std::fs::write(temp_dir.join("util.rk"), "answer = -> 42\n< answer\n").unwrap();

        let config = crate::Config {
            entry_file: entry.clone(),
            no_std: true,
            no_prelude: true,
            current_crate_name: Some("demo".to_string()),
            ..crate::Config::default()
        };
        let mut db = crate::source_loader::SourceDatabase::new();
        let graph = db.load_entry(entry.clone(), &config).unwrap();
        std::fs::remove_file(temp_dir.join("util.rk")).unwrap();
        let program = crate::ast::Program {
            module: graph.root_module().clone(),
        };

        let decls = collect_with_source_graph(
            &program,
            &graph,
            &crate::crate_system::CrateContext::new(),
            false,
            Some("demo"),
        )
        .expect("collection should use preloaded graph instead of reading util.rk again");

        assert!(function_named(&decls, "demo::util::answer").is_some());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn collect_uses_source_graph_path_for_directory_modules() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_collect_source_graph_dir_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(temp_dir.join("util")).unwrap();
        let entry = temp_dir.join("main.rk");
        std::fs::write(&entry, "mod util\n> util::answer\nmain = -> answer!\n").unwrap();
        std::fs::write(temp_dir.join("util/mod.rk"), "answer = -> 42\n< answer\n").unwrap();

        let config = crate::Config {
            entry_file: entry.clone(),
            no_std: true,
            no_prelude: true,
            current_crate_name: Some("demo".to_string()),
            ..crate::Config::default()
        };
        let mut db = crate::source_loader::SourceDatabase::new();
        let graph = db.load_entry(entry, &config).unwrap();
        assert!(!temp_dir.join("util.rk").exists());
        let util_mod_path = temp_dir.join("util/mod.rk");
        let util_flat_path = temp_dir.join("util.rk");
        let program = crate::ast::Program {
            module: graph.root_module().clone(),
        };

        let decls = collect_with_source_graph(
            &program,
            &graph,
            &crate::crate_system::CrateContext::new(),
            false,
            Some("demo"),
        )
        .expect("collection should use graph path for util/mod.rk");

        assert!(function_named(&decls, "demo::util::answer").is_some());
        assert!(decls
            .loaded_module_paths
            .iter()
            .any(|(name, path)| name == "demo::util" && path == &util_mod_path));
        assert!(!decls
            .loaded_module_paths
            .iter()
            .any(|(name, path)| name == "demo::util" && path == &util_flat_path));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    fn ident_expr(name: &str) -> crate::ast::Expression {
        crate::ast::Expression::UnaryExpr(crate::ast::UnaryExpr::PrimaryExpr(
            crate::ast::PrimaryExpr {
                operand: crate::ast::Operand::Ident(crate::ast::IdentifierPath {
                    path: vec![crate::ast::IdentOrType::Ident(ident(name))],
                }),
                secondaries: None,
                type_annotation: None,
            },
        ))
    }

    fn function_decl_with_body_expr(
        name: &str,
        expr: crate::ast::Expression,
    ) -> crate::ast::FunctionDecl {
        crate::ast::FunctionDecl {
            name: ident(name),
            lambda: crate::ast::LambdaDecl {
                parameters: vec![],
                body: crate::ast::Block {
                    statements: vec![crate::ast::Statement::Expression(expr)],
                },
                arrow_kind: crate::ast::LambdaArrowKind::Normal,
                span: crate::lexer::Span::test(),
            },
            self_receiver: None,
            is_unsafe: false,
            exported: false,
        }
    }

    fn function_decl(name: &str) -> FunctionDecl {
        FunctionDecl {
            name: ident(name),
            lambda: LambdaDecl {
                parameters: vec![],
                body: crate::ast::Block { statements: vec![] },
                arrow_kind: LambdaArrowKind::Normal,
                span: crate::lexer::Span::test(),
            },
            self_receiver: None,
            is_unsafe: false,
            exported: false,
        }
    }

    fn ident_pattern(name: &str) -> Pattern {
        Pattern {
            binding: None,
            kind: PatternKind::Ident(crate::ast::IdentPattern {
                name: ident(name),
                mut_: false,
            }),
        }
    }

    fn generic_param_decl(name: &str) -> crate::ast::GenericParamDecl {
        crate::ast::GenericParamDecl {
            unsized_bound: None,
            name: ident(name),
            kind: None,
            span: crate::lexer::Span::test(),
        }
    }

    fn function_sig(name: &str, sig: ParseType) -> FunctionSig {
        FunctionSig {
            name: ident(name),
            sig,
            where_clauses: vec![],
            self_receiver: None,
            is_unsafe: false,
            exported: false,
        }
    }

    fn single_param_function_decl(name: &str, param: &str) -> FunctionDecl {
        FunctionDecl {
            name: ident(name),
            lambda: LambdaDecl {
                parameters: vec![ident_pattern(param)],
                body: crate::ast::Block { statements: vec![] },
                arrow_kind: LambdaArrowKind::Normal,
                span: crate::lexer::Span::test(),
            },
            self_receiver: None,
            is_unsafe: false,
            exported: false,
        }
    }

    fn generic_declaration_program() -> Program {
        let generic_t = || ParseType::Type(type_inner("T"));
        let inner_t = || generic_type("Inner", vec![generic_t()]);
        let outer_t = || generic_type("Outer", vec![generic_t()]);
        let choice_inner_t = || generic_type("Choice", vec![inner_t()]);
        let choice_outer_t = || generic_type("Choice", vec![outer_t()]);

        Program {
            module: Module {
                name: None,
                top_levels: vec![
                    TopLevel::StructDecl(StructDecl {
                        name: generic_type_inner("Inner", vec![]),
                        generic_params: vec![generic_param_decl("T")],
                        fields: vec![struct_field("value", generic_t())],
                        exported: false,
                    }),
                    TopLevel::EnumDecl(crate::ast::EnumDecl {
                        name: generic_type_inner("Choice", vec![generic_t()]),
                        variants: vec![crate::ast::EnumVariant {
                            name: type_inner("Some"),
                            fields: crate::ast::NamedFieldsOrTypesList::TypesList(vec![inner_t()]),
                        }],
                        exported: false,
                        language_items: Default::default(),
                    }),
                    TopLevel::StructDecl(StructDecl {
                        name: generic_type_inner("Outer", vec![]),
                        generic_params: vec![generic_param_decl("T")],
                        fields: vec![struct_field("choice", choice_inner_t())],
                        exported: false,
                    }),
                    TopLevel::TraitDecl(crate::ast::TraitDecl {
                        where_clauses: Vec::new(),
                        name: generic_type_inner("Mapper", vec![]),
                        generic_params: vec![generic_param_decl("T")],
                        for_: None,
                        associated_types: vec![],
                        methods: HashMap::new(),
                        signatures: HashMap::from([(
                            ident("map"),
                            function_sig(
                                "map",
                                ParseType::Function(vec![outer_t(), choice_outer_t()]),
                            ),
                        )]),
                        exported: false,
                        language_items: Default::default(),
                    }),
                    TopLevel::FunctionSig(function_sig(
                        "identity",
                        ParseType::Function(vec![outer_t(), choice_outer_t()]),
                    )),
                    TopLevel::FunctionDecl(single_param_function_decl("identity", "value")),
                    TopLevel::Impl(Impl {
                        name: generic_type_inner("Outer", vec![generic_t()]),
                        for_: None,
                        associated_types: vec![],
                        methods: HashMap::from([(ident("new"), function_decl("new"))]),
                        signatures: HashMap::new(),
                        where_clauses: vec![],
                    }),
                ],
                is_inline: false,
                filepath: None,
            },
        }
    }

    fn assert_generic_param_descriptor(
        params: &[GenericParamDecl],
        owner: DefId,
        index: u32,
        name: &str,
    ) {
        assert_eq!(
            params,
            &[GenericParamDecl::new(
                GenericParamId { owner, index },
                name,
                Kind::Type,
            )]
        );
    }

    fn assert_generic_declaration_ids(decls: &Declarations) {
        let root_module_id = decls.indexing_ids.root_module_id();
        let item_id = |index| {
            decls
                .item_index
                .item_at_source(root_module_id, index)
                .unwrap()
                .def_id
        };
        let inner_id = item_id(0);
        let choice_id = item_id(1);
        let outer_id = item_id(2);
        let mapper_id = item_id(3);
        let function_id = item_id(5);
        let impl_id = item_id(6);

        let inner = decls.items.structs().get(&inner_id).unwrap();
        let choice = decls.items.enums().get(&choice_id).unwrap();
        let outer = decls.items.structs().get(&outer_id).unwrap();
        let mapper = decls.items.traits().get(&mapper_id).unwrap();
        let function = decls.items.functions().get(&function_id).unwrap();
        let impl_ = decls.items.impls().get(&impl_id).unwrap();

        assert_eq!(inner.id, inner_id);
        assert_eq!(choice.id, choice_id);
        assert_eq!(outer.id, outer_id);
        assert_eq!(mapper.id, mapper_id);
        assert_eq!(function.id, function_id);
        assert_eq!(impl_.id, impl_id);

        assert_generic_param_descriptor(&inner.generic_params, inner_id, 0, "T");
        assert_generic_param_descriptor(&choice.generic_params, choice_id, 0, "T");
        assert_generic_param_descriptor(&outer.generic_params, outer_id, 0, "T");
        assert_generic_param_descriptor(&mapper.generic_params, mapper_id, 0, "T");
        assert_generic_param_descriptor(&function.generic_params, function_id, 0, "T");
        assert_generic_param_descriptor(&impl_.type_generics, impl_id, 0, "T");
        assert!(mapper
            .signatures
            .get("map")
            .expect("generic trait method signature should be collected")
            .generic_params
            .is_empty());

        assert_eq!(inner.generic_params[0].name, choice.generic_params[0].name);
        assert_ne!(inner.generic_params[0], choice.generic_params[0]);
        assert_ne!(inner.generic_params[0].id, choice.generic_params[0].id);

        let crate::hir::HirVariantFields::Positional(choice_fields) = &choice.variants[0].fields
        else {
            panic!("generic choice variant should retain its positional payload");
        };
        assert_eq!(
            choice_fields,
            &[Type::Struct {
                id: inner_id,
                args: vec![Type::Generic(GenericParamId {
                    owner: choice_id,
                    index: 0,
                })],
            }]
        );
        assert_eq!(
            outer.fields[0].ty,
            Type::Enum {
                id: choice_id,
                args: vec![Type::Struct {
                    id: inner_id,
                    args: vec![Type::Generic(GenericParamId {
                        owner: outer_id,
                        index: 0,
                    })],
                }],
            }
        );
        assert_eq!(
            impl_.receiver_pattern,
            crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
                id: outer_id,
                args: vec![Type::Generic(GenericParamId {
                    owner: impl_id,
                    index: 0,
                })],
            })
        );
        assert!(decls.current_def_ids.contains(&mapper.signatures["map"].id));
        assert!(decls.current_def_ids.contains(&impl_.methods["new"].id));
    }

    fn hir_function(name: &str) -> HirFunction {
        HirFunction {
            id: DefId::new(CrateId(0), LocalDefId(0)),
            name: name.to_string(),
            generic_params: vec![],
            generic_bounds: HashMap::new().into(),
            params: vec![],
            ret_type: Type::Unit,
            body: HirBlock {
                stmts: vec![],
                ty: Type::Unit,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        }
    }

    fn test_function(id: DefId, name: &str) -> HirFunction {
        HirFunction {
            id,
            name: name.to_string(),
            generic_params: vec![],
            generic_bounds: HashMap::new().into(),
            params: vec![],
            ret_type: Type::Unit,
            body: HirBlock {
                stmts: vec![],
                ty: Type::Unit,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        }
    }

    #[test]
    fn declaration_items_reject_key_payload_id_mismatch() {
        let key = DefId::new(CrateId(0), LocalDefId(1));
        let payload_id = DefId::new(CrateId(0), LocalDefId(2));
        let errors = DeclarationItems::from_id_maps(
            HashMap::from([(key, test_function(payload_id, "answer"))]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
        )
        .unwrap_err();

        assert!(errors[0].message.contains("function declaration key"));
    }

    #[test]
    fn collection_outputs_structs_keyed_by_canonical_def_id() {
        let declarations = collect(
            &program_with_struct("Point"),
            &crate::crate_system::CrateContext::new(),
            false,
            None,
        )
        .expect("collection should succeed");
        let point_id = DefId::new(CrateId(0), LocalDefId(0));

        assert_eq!(declarations.items.structs()[&point_id].id, point_id);
    }

    #[test]
    fn local_collector_uses_source_records_before_current_id_repairs() {
        let source_path = PathBuf::from("/test/source.rk");
        let source_module = Module {
            name: Some(ident("source")),
            top_levels: vec![
                TopLevel::InfixOperator(7, "++".to_string()),
                TopLevel::StructDecl(StructDecl {
                    name: type_inner("Same"),
                    generic_params: vec![],
                    fields: vec![],
                    exported: false,
                }),
            ],
            is_inline: false,
            filepath: Some(source_path.clone()),
        };
        let module = Module {
            name: None,
            top_levels: vec![
                TopLevel::InfixOperator(7, "++".to_string()),
                TopLevel::StructDecl(StructDecl {
                    name: type_inner("Same"),
                    generic_params: vec![],
                    fields: vec![],
                    exported: false,
                }),
                inline_module(
                    "inline",
                    vec![TopLevel::StructDecl(StructDecl {
                        name: type_inner("Same"),
                        generic_params: vec![],
                        fields: vec![],
                        exported: false,
                    })],
                ),
                TopLevel::Mod(ident("source"), false),
                TopLevel::Impl(inherent_impl("Same", "first")),
                TopLevel::Impl(inherent_impl("Same", "second")),
            ],
            is_inline: false,
            filepath: None,
        };
        let mut source_modules = HashMap::new();
        source_modules.insert(
            vec!["test".to_string(), "source".to_string()],
            source_module,
        );
        let mut indexing_ids = IndexingIds::new_root();
        let root_module_id = indexing_ids.root_module_id();
        let item_index = index_root_module_items_with_sources(
            &mut indexing_ids,
            &module,
            Some("test"),
            &source_modules,
        );
        let mut id_environment = collected_id_environment_from_index(&item_index, root_module_id);
        populate_member_id_environment(
            &mut id_environment,
            &mut indexing_ids,
            &item_index,
            &module,
            Some("test"),
            &source_modules,
        );

        let mut context =
            context::CollectContext::bootstrap_for_collection(false, Some("test"), None);
        context
            .loaded_module_paths
            .push(("test::source".to_string(), source_path.clone()));
        context.module_file_cache.insert(
            source_path,
            source_modules[&vec!["test".to_string(), "source".to_string()]].clone(),
        );
        let mut collector =
            collector::LocalCollector::new_with_id_environment(context, id_environment);
        collector.collect_local_declarations(&module, root_module_id);
        let collected = collector.finish(&item_index, root_module_id, Some("test"));

        assert_eq!(
            collected.structs["Same"].id,
            item_index.item_at_source(root_module_id, 1).unwrap().def_id
        );
        let inline_module_id = item_index
            .module_id_by_path(&["inline".to_string()])
            .unwrap();
        assert_eq!(
            collected.structs["inline::Same"].id,
            item_index
                .item_at_source(inline_module_id, 0)
                .unwrap()
                .def_id
        );
        let source_module_id = item_index
            .module_id_by_path(&["source".to_string()])
            .unwrap();
        assert_eq!(
            collected.structs["test::source::Same"].id,
            item_index
                .item_at_source(source_module_id, 1)
                .unwrap()
                .def_id
        );
        assert_eq!(
            collected
                .impls
                .iter()
                .map(|impl_| impl_.id)
                .collect::<Vec<_>>(),
            vec![
                item_index.item_at_source(root_module_id, 4).unwrap().def_id,
                item_index.item_at_source(root_module_id, 5).unwrap().def_id,
            ]
        );
    }

    fn hir_trait(id: DefId, name: &str, method_names: &[&str]) -> HirTrait {
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id,
            name: name.to_string(),
            generic_params: vec![],
            associated_types: vec![],
            methods: method_names
                .iter()
                .map(|method_name| (method_name.to_string(), hir_function(method_name)))
                .collect(),
            signatures: HashMap::new(),
        }
    }

    fn inherent_impl(type_name: &str, method_name: &str) -> Impl {
        Impl {
            name: type_inner(type_name),
            for_: None,
            associated_types: vec![],
            methods: HashMap::from([(ident(method_name), function_decl(method_name))]),
            signatures: HashMap::new(),
            where_clauses: vec![],
        }
    }

    fn trait_decl(name: &str) -> TopLevel {
        TopLevel::TraitDecl(crate::ast::TraitDecl {
            where_clauses: Vec::new(),
            name: type_inner(name),
            generic_params: vec![],
            for_: None,
            associated_types: vec![],
            methods: HashMap::new(),
            signatures: HashMap::new(),
            exported: false,
            language_items: Default::default(),
        })
    }

    fn trait_impl(type_name: &str, trait_name: &str, method_name: &str) -> Impl {
        Impl {
            name: type_inner(trait_name),
            for_: Some(crate::ast::ParseType::Type(type_inner(type_name))),
            associated_types: vec![],
            methods: HashMap::from([(ident(method_name), function_decl(method_name))]),
            signatures: HashMap::new(),
            where_clauses: vec![],
        }
    }

    fn is_invalid_current_crate_id(id: DefId) -> bool {
        id.crate_id == CrateId(u32::MAX)
    }

    fn function_named<'a>(decls: &'a Declarations, name: &str) -> Option<&'a HirFunction> {
        decls.items.functions().values().find(|function| {
            decls
                .resolver
                .item_names_by_id
                .get(&function.id)
                .map(String::as_str)
                == Some(name)
                || function.name == name
        })
    }

    fn function_at_canonical_name<'a>(
        decls: &'a Declarations,
        canonical_name: &str,
    ) -> Option<&'a HirFunction> {
        decls
            .resolver
            .item_paths
            .get(canonical_name)
            .and_then(|id| decls.items.functions().get(id))
    }

    fn function_sig_named<'a>(decls: &'a Declarations, name: &str) -> Option<&'a HirFunctionSig> {
        decls.items.function_sigs().values().find(|signature| {
            decls
                .resolver
                .item_names_by_id
                .get(&signature.id)
                .map(String::as_str)
                == Some(name)
                || signature.name == name
        })
    }

    fn struct_named<'a>(decls: &'a Declarations, name: &str) -> Option<&'a HirStruct> {
        decls.items.structs().values().find(|structure| {
            decls
                .resolver
                .item_names_by_id
                .get(&structure.id)
                .map(String::as_str)
                == Some(name)
                || structure.name == name
        })
    }

    fn enum_named<'a>(decls: &'a Declarations, name: &str) -> Option<&'a HirEnum> {
        decls.items.enums().values().find(|enumeration| {
            decls
                .resolver
                .item_names_by_id
                .get(&enumeration.id)
                .map(String::as_str)
                == Some(name)
                || enumeration.name == name
        })
    }

    fn trait_named<'a>(decls: &'a Declarations, name: &str) -> Option<&'a HirTrait> {
        decls.items.traits().values().find(|trait_def| {
            decls
                .resolver
                .item_names_by_id
                .get(&trait_def.id)
                .map(String::as_str)
                == Some(name)
                || trait_def.name == name
        })
    }

    fn assert_no_invalid_current_crate_ids(decls: &Declarations) {
        for function in decls.items.functions().values() {
            let name = &function.name;
            assert!(
                !is_invalid_current_crate_id(function.id),
                "function {name} kept invalid current-crate id {:?}",
                function.id
            );
            for generic in &function.generic_params {
                let generic_id = generic.id;
                assert!(
                    !is_invalid_current_crate_id(generic_id.owner),
                    "function {name} generic {:?} kept invalid owner",
                    generic_id
                );
            }
        }

        for sig in decls.items.function_sigs().values() {
            let name = &sig.name;
            assert!(
                !is_invalid_current_crate_id(sig.id),
                "signature {name} kept invalid current-crate id {:?}",
                sig.id
            );
            for generic in &sig.generic_params {
                let generic_id = generic.id;
                assert!(
                    !is_invalid_current_crate_id(generic_id.owner),
                    "signature {name} generic {:?} kept invalid owner",
                    generic_id
                );
            }
        }

        for strukt in decls.items.structs().values() {
            let name = &strukt.name;
            assert!(
                !is_invalid_current_crate_id(strukt.id),
                "struct {name} kept invalid current-crate id {:?}",
                strukt.id
            );
        }

        for enum_ in decls.items.enums().values() {
            let name = &enum_.name;
            assert!(
                !is_invalid_current_crate_id(enum_.id),
                "enum {name} kept invalid current-crate id {:?}",
                enum_.id
            );
        }

        for trait_def in decls.items.traits().values() {
            let name = &trait_def.name;
            assert!(
                !is_invalid_current_crate_id(trait_def.id),
                "trait {name} kept invalid current-crate id {:?}",
                trait_def.id
            );
            for (method_name, method) in &trait_def.methods {
                assert!(
                    !is_invalid_current_crate_id(method.id),
                    "trait method {name}.{method_name} kept invalid id {:?}",
                    method.id
                );
            }
            for (sig_name, sig) in &trait_def.signatures {
                assert!(
                    !is_invalid_current_crate_id(sig.id),
                    "trait signature {name}.{sig_name} kept invalid id {:?}",
                    sig.id
                );
                for generic in &sig.generic_params {
                    let generic_id = generic.id;
                    assert!(
                        !is_invalid_current_crate_id(generic_id.owner),
                        "trait signature {name}.{sig_name} generic {:?} kept invalid owner",
                        generic_id
                    );
                }
            }
        }

        for imp in decls.items.impls().values() {
            assert!(
                !is_invalid_current_crate_id(imp.id),
                "impl for {} kept invalid id {:?}",
                imp.type_name,
                imp.id
            );
            for (method_name, method) in &imp.methods {
                assert!(
                    !is_invalid_current_crate_id(method.id),
                    "impl method {}.{} kept invalid id {:?}",
                    imp.type_name,
                    method_name,
                    method.id
                );
            }
        }

        for ext in decls.items.externs().values() {
            assert!(
                !is_invalid_current_crate_id(ext.id),
                "extern {} kept invalid id {:?}",
                ext.name,
                ext.id
            );
        }
    }

    #[test]
    fn collect_assigns_supported_current_crate_ids_before_lowering() {
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![
                    TopLevel::FunctionSig(function_sig(
                        "identity",
                        ParseType::Function(vec![ParseType::Type(type_inner("I64"))]),
                    )),
                    TopLevel::FunctionDecl(single_param_function_decl("identity", "value")),
                    TopLevel::StructDecl(StructDecl {
                        name: type_inner("Box"),
                        generic_params: vec![],
                        fields: vec![],
                        exported: false,
                    }),
                    TopLevel::EnumDecl(crate::ast::EnumDecl {
                        name: type_inner("Maybe"),
                        variants: vec![crate::ast::EnumVariant {
                            name: type_inner("None"),
                            fields: crate::ast::NamedFieldsOrTypesList::TypesList(vec![]),
                        }],
                        exported: false,
                        language_items: Default::default(),
                    }),
                    TopLevel::TraitDecl(crate::ast::TraitDecl {
                        where_clauses: Vec::new(),
                        name: type_inner("Show"),
                        generic_params: vec![],
                        for_: None,
                        associated_types: vec![],
                        methods: HashMap::from([(ident("show"), function_decl("show"))]),
                        signatures: HashMap::from([(
                            ident("convert"),
                            function_sig(
                                "convert",
                                ParseType::Function(vec![
                                    ParseType::Type(type_inner("T")),
                                    ParseType::Type(type_inner("T")),
                                ]),
                            ),
                        )]),
                        exported: false,
                        language_items: Default::default(),
                    }),
                    TopLevel::Impl(trait_impl("Box", "Show", "show")),
                    TopLevel::Extern(function_sig(
                        "puts",
                        ParseType::Function(vec![ParseType::Type(type_inner("I32"))]),
                    )),
                ],
                is_inline: false,
                filepath: None,
            },
        };

        let decls = collect(&program, &CrateContext::new(), false, Some("test"))
            .expect("collect assigns supported current-crate ids");

        assert_no_invalid_current_crate_ids(&decls);
    }

    #[test]
    fn collect_generic_param_descriptor_fields_from_declarations() {
        let decls = collect(
            &generic_declaration_program(),
            &CrateContext::new(),
            false,
            Some("test"),
        )
        .expect("generic declaration collection should use indexed IDs during construction");

        assert_generic_declaration_ids(&decls);
    }

    #[test]
    fn collect_errors_when_trait_impl_defines_unknown_associated_type() {
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![
                    TopLevel::StructDecl(StructDecl {
                        name: type_inner("Box"),
                        generic_params: vec![],
                        fields: vec![],
                        exported: false,
                    }),
                    TopLevel::TraitDecl(crate::ast::TraitDecl {
                        where_clauses: Vec::new(),
                        name: type_inner("Iterable"),
                        generic_params: vec![],
                        for_: None,
                        associated_types: vec![crate::ast::AssociatedTypeDecl {
                            name: ident("Item"),
                            kind: None,
                        }],
                        methods: HashMap::new(),
                        signatures: HashMap::new(),
                        exported: false,
                        language_items: Default::default(),
                    }),
                    TopLevel::Impl(Impl {
                        name: type_inner("Iterable"),
                        for_: Some(ParseType::Type(type_inner("Box"))),
                        associated_types: vec![crate::ast::AssociatedTypeDef {
                            name: ident("Output"),
                            kind: None,
                            ty: ParseType::Type(type_inner("I64")),
                        }],
                        methods: HashMap::new(),
                        signatures: HashMap::new(),
                        where_clauses: vec![],
                    }),
                ],
                is_inline: false,
                filepath: None,
            },
        };

        let errors = match collect(&program, &CrateContext::new(), false, Some("test")) {
            Ok(_) => panic!("unknown trait associated type should be a collection error"),
            Err(errors) => errors,
        };

        assert!(errors
            .iter()
            .any(|err| err.message.contains("unknown associated type 'Output'")));
    }

    fn add_artifact_extern_crate(
        crate_ctx: &mut CrateContext,
        crate_name: &str,
        interface: ArtifactCrateInterface,
        resolver: crate::collect::resolver::ResolverTables,
        prelude_export_ids: BTreeMap<String, ArtifactExport>,
    ) {
        crate_ctx
            .add_extern_crate(crate::crate_system::ExternCrateRecord::new(
                CrateId(1),
                crate_name.to_string(),
                crate::crate_system::ExternCrateMetadata::new(
                    interface,
                    resolver,
                    prelude_export_ids,
                ),
                crate::crate_system::ExternCrateBodies::default(),
                crate::crate_system::ExternCrateLink::metadata_only(BTreeMap::new()),
            ))
            .unwrap();
    }

    fn add_artifact_extern_crate_with_language_items(
        crate_ctx: &mut CrateContext,
        crate_name: &str,
        interface: ArtifactCrateInterface,
        resolver: crate::collect::resolver::ResolverTables,
        language_items: LanguageItems<DefId>,
    ) {
        crate_ctx
            .add_extern_crate(crate::crate_system::ExternCrateRecord::new(
                CrateId(1),
                crate_name.to_string(),
                crate::crate_system::ExternCrateMetadata::new(interface, resolver, BTreeMap::new())
                    .with_language_items(language_items),
                crate::crate_system::ExternCrateBodies::default(),
                crate::crate_system::ExternCrateLink::metadata_only(BTreeMap::new()),
            ))
            .unwrap();
    }

    #[test]
    fn collect_records_dependency_import_alias_by_canonical_def_id() {
        let answer_id = DefId::new(CrateId(7), LocalDefId(11));
        let mut answer = hir_function("answer");
        answer.id = answer_id;

        let mut interface = ArtifactCrateInterface::default();
        interface.insert_function("dep::io::answer".to_string(), answer);
        interface.root_export_ids.insert(
            "answer".to_string(),
            ArtifactExport {
                source: "dep::io::answer".to_string(),
                id: answer_id,
            },
        );

        let mut resolver = crate::collect::resolver::ResolverTables::default();
        resolver
            .item_paths
            .insert("dep::io::answer".to_string(), answer_id);
        resolver
            .item_names_by_id
            .insert(answer_id, "dep::io::answer".to_string());

        let mut crate_ctx = CrateContext::new();
        add_artifact_extern_crate(&mut crate_ctx, "dep", interface, resolver, BTreeMap::new());
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![TopLevel::Import(crate::ast::Path::Ident(
                    crate::ast::IdentifierPath {
                        path: vec![
                            crate::ast::IdentOrType::Ident(ident("dep")),
                            crate::ast::IdentOrType::Ident(ident("answer")),
                        ],
                    },
                ))],
                is_inline: false,
                filepath: None,
            },
        };

        let decls = collect(&program, &crate_ctx, false, Some("test"))
            .expect("collect should record dependency import alias IDs");

        assert!(function_named(&decls, "dep::io::answer").is_some());
        assert!(function_named(&decls, "dep::answer").is_none());
        assert_eq!(
            decls.resolver.import_aliases.get("answer"),
            Some(&answer_id)
        );
        assert_eq!(
            decls.resolver.item_names_by_id.get(&answer_id),
            Some(&"dep::io::answer".to_string())
        );
    }

    #[test]
    fn collect_records_canonical_names_for_artifact_payloads_without_dependency_resolver_names() {
        let answer_id = DefId::new(CrateId(7), LocalDefId(11));
        let mut answer = hir_function("answer");
        answer.id = answer_id;

        let mut interface = ArtifactCrateInterface::default();
        interface.insert_function("dep::answer".to_string(), answer);

        let mut crate_ctx = CrateContext::new();
        add_artifact_extern_crate(
            &mut crate_ctx,
            "dep",
            interface,
            crate::collect::resolver::ResolverTables::default(),
            BTreeMap::new(),
        );
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![],
                is_inline: false,
                filepath: None,
            },
        };

        let decls = collect(&program, &crate_ctx, false, Some("test"))
            .expect("collection should retain artifact declarations");

        assert_eq!(
            decls.resolver.item_names_by_id.get(&answer_id),
            Some(&"dep::answer".to_string())
        );
        assert!(crate::lower::Lowerer::from_declarations(decls).is_ok());
    }

    #[test]
    fn collect_records_module_local_artifact_root_import_alias_by_canonical_def_id() {
        let drop_id = DefId::new(CrateId(7), LocalDefId(13));

        let mut interface = ArtifactCrateInterface::default();
        interface.insert_trait(
            "dep::drop::Drop".to_string(),
            hir_trait(drop_id, "Drop", &[]),
        );
        interface.root_export_ids.insert(
            "Drop".to_string(),
            ArtifactExport {
                source: "dep::drop::Drop".to_string(),
                id: drop_id,
            },
        );

        let mut resolver = crate::collect::resolver::ResolverTables::default();
        resolver
            .item_paths
            .insert("dep::drop::Drop".to_string(), drop_id);
        resolver
            .item_names_by_id
            .insert(drop_id, "dep::drop::Drop".to_string());

        let mut crate_ctx = CrateContext::new();
        add_artifact_extern_crate(&mut crate_ctx, "dep", interface, resolver, BTreeMap::new());
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![inline_module(
                    "inner",
                    vec![TopLevel::Import(crate::ast::Path::Ident(
                        crate::ast::IdentifierPath {
                            path: vec![
                                crate::ast::IdentOrType::Ident(ident("dep")),
                                crate::ast::IdentOrType::Ident(ident("Drop")),
                            ],
                        },
                    ))],
                )],
                is_inline: false,
                filepath: None,
            },
        };

        let decls = collect(&program, &crate_ctx, false, Some("test"))
            .expect("collect should record module-local artifact import alias IDs");

        assert_eq!(decls.resolver.import_aliases.get("Drop"), None);
        assert_eq!(
            decls.resolver.import_aliases.get("inner::Drop"),
            Some(&drop_id)
        );
        assert_eq!(
            decls.resolver.item_names_by_id.get(&drop_id),
            Some(&"dep::drop::Drop".to_string())
        );
    }

    #[test]
    fn resolver_contract_keeps_same_name_module_items_qualified_by_id() {
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![
                    inline_module(
                        "left",
                        vec![TopLevel::StructDecl(StructDecl {
                            name: type_inner("Thing"),
                            generic_params: vec![],
                            fields: vec![],
                            exported: true,
                        })],
                    ),
                    inline_module(
                        "right",
                        vec![TopLevel::StructDecl(StructDecl {
                            name: type_inner("Thing"),
                            generic_params: vec![],
                            fields: vec![],
                            exported: true,
                        })],
                    ),
                ],
                is_inline: false,
                filepath: None,
            },
        };

        let decls = collect(&program, &CrateContext::new(), false, Some("test"))
            .expect("same-name module items should collect");
        let left_id = struct_named(&decls, "left::Thing").unwrap().id;
        let right_id = struct_named(&decls, "right::Thing").unwrap().id;

        assert_ne!(left_id, right_id);
        assert_eq!(decls.resolver.item_paths.get("left::Thing"), Some(&left_id));
        assert_eq!(
            decls.resolver.item_paths.get("test::left::Thing"),
            Some(&left_id)
        );
        assert_eq!(
            decls.resolver.item_paths.get("right::Thing"),
            Some(&right_id)
        );
        assert_eq!(decls.resolver.item_paths.get("Thing"), None);
    }

    #[test]
    fn resolver_contract_keeps_unresolved_imports_out_of_alias_metadata() {
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![TopLevel::Import(crate::ast::Path::Ident(
                    crate::ast::IdentifierPath {
                        path: vec![
                            crate::ast::IdentOrType::Ident(ident("missing")),
                            crate::ast::IdentOrType::Ident(ident("value")),
                        ],
                    },
                ))],
                is_inline: false,
                filepath: None,
            },
        };

        let decls = collect(&program, &CrateContext::new(), false, Some("test"))
            .expect("collection should leave unresolved imports for lowering diagnostics");

        assert_eq!(decls.resolver.import_aliases.get("value"), None);
        assert_eq!(decls.resolver.item_paths.get("missing::value"), None);
    }

    #[test]
    fn resolver_contract_preserves_inline_function_signature_metadata_by_qualified_id() {
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![inline_module(
                    "inner",
                    vec![
                        TopLevel::FunctionSig(function_sig(
                            "id",
                            ParseType::Function(vec![ParseType::Type(type_inner("I64"))]),
                        )),
                        TopLevel::FunctionDecl(function_decl("id")),
                    ],
                )],
                is_inline: false,
                filepath: None,
            },
        };

        let decls = collect(&program, &CrateContext::new(), false, Some("test"))
            .expect("inline signature-backed function should collect");
        let function = function_at_canonical_name(&decls, "inner::id")
            .expect("qualified inline function should be collected");

        assert_eq!(function.ret_type, Type::I64);
        assert_eq!(function.name, "id");
        assert_eq!(
            decls
                .resolver
                .item_names_by_id
                .get(&function.id)
                .map(String::as_str),
            Some("inner::id")
        );
        assert!(decls.function_type_vars.contains_key(&function.id));
        assert_eq!(decls.function_type_vars.len(), 1);
        assert_eq!(
            decls.resolver.item_paths.get("inner::id"),
            Some(&function.id)
        );
    }

    #[test]
    fn resolver_contract_preserves_inline_function_body_type_vars_by_qualified_id() {
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![inline_module(
                    "inner",
                    vec![TopLevel::FunctionDecl(single_param_function_decl(
                        "identity", "value",
                    ))],
                )],
                is_inline: false,
                filepath: None,
            },
        };

        let decls = collect(&program, &CrateContext::new(), false, Some("test"))
            .expect("inline function should collect");
        let function = function_at_canonical_name(&decls, "inner::identity")
            .expect("qualified inline function should be collected");

        assert_eq!(function.name, "identity");
        assert_eq!(
            decls
                .resolver
                .item_names_by_id
                .get(&function.id)
                .map(String::as_str),
            Some("inner::identity")
        );
        assert!(decls.function_type_vars.contains_key(&function.id));
        assert_eq!(decls.function_type_vars.len(), 1);
        assert_eq!(
            decls.resolver.item_paths.get("inner::identity"),
            Some(&function.id)
        );
    }

    #[test]
    fn resolver_contract_preserves_root_function_metadata_when_inline_name_collides() {
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![
                    TopLevel::FunctionSig(function_sig(
                        "identity",
                        ParseType::Function(vec![ParseType::Type(type_inner("I64"))]),
                    )),
                    TopLevel::FunctionDecl(function_decl("identity")),
                    inline_module(
                        "inner",
                        vec![TopLevel::FunctionDecl(single_param_function_decl(
                            "identity", "value",
                        ))],
                    ),
                ],
                is_inline: false,
                filepath: None,
            },
        };

        let decls = collect(&program, &CrateContext::new(), false, Some("test"))
            .expect("same-name root and inline functions should collect");
        let root = function_at_canonical_name(&decls, "identity")
            .expect("root function should remain collected");
        let inner = function_at_canonical_name(&decls, "inner::identity")
            .expect("qualified inline function should be collected");

        assert_ne!(root.id, inner.id);
        assert_eq!(root.params.len(), 0);
        assert_eq!(inner.params.len(), 1);
        assert_eq!(root.ret_type, Type::I64);
        assert_eq!(root.name, "identity");
        assert_eq!(inner.name, "identity");
        assert_eq!(
            decls
                .resolver
                .item_names_by_id
                .get(&inner.id)
                .map(String::as_str),
            Some("inner::identity")
        );
        assert!(decls.function_type_vars.contains_key(&root.id));
        assert!(decls.function_type_vars.contains_key(&inner.id));
        assert_eq!(decls.resolver.item_paths.get("identity"), Some(&root.id));
        assert_eq!(
            decls.resolver.item_paths.get("inner::identity"),
            Some(&inner.id)
        );
    }

    #[test]
    fn collect_records_artifact_glob_import_alias_by_export_def_id() {
        let answer_id = DefId::new(CrateId(7), LocalDefId(12));
        let mut answer = hir_function("answer");
        answer.id = answer_id;

        let mut interface = ArtifactCrateInterface::default();
        interface.insert_function("dep::io::answer".to_string(), answer);
        interface.root_export_ids.insert(
            "answer".to_string(),
            ArtifactExport {
                source: "dep::io::answer".to_string(),
                id: answer_id,
            },
        );

        let mut resolver = crate::collect::resolver::ResolverTables::default();
        resolver
            .item_paths
            .insert("dep::io::answer".to_string(), answer_id);
        resolver
            .item_names_by_id
            .insert(answer_id, "dep::io::answer".to_string());

        let mut crate_ctx = CrateContext::new();
        add_artifact_extern_crate(&mut crate_ctx, "dep", interface, resolver, BTreeMap::new());
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![TopLevel::GlobImport(vec!["dep".to_string()])],
                is_inline: false,
                filepath: None,
            },
        };

        let decls = collect(&program, &crate_ctx, false, None)
            .expect("collect should record artifact glob import alias IDs");

        assert_eq!(
            decls.resolver.import_aliases.get("answer"),
            Some(&answer_id)
        );
        assert_eq!(
            decls.resolver.item_names_by_id.get(&answer_id),
            Some(&"dep::io::answer".to_string())
        );
    }

    #[test]
    fn collect_records_stdlib_prelude_alias_by_canonical_def_id() {
        let print_id = DefId::new(CrateId(8), LocalDefId(3));
        let mut print = hir_function("print");
        print.id = print_id;

        let mut interface = ArtifactCrateInterface::default();
        interface.insert_function("stdlib::prelude::print".to_string(), print);

        let mut resolver = crate::collect::resolver::ResolverTables::default();
        resolver
            .item_paths
            .insert("stdlib::prelude::print".to_string(), print_id);
        resolver
            .item_names_by_id
            .insert(print_id, "stdlib::prelude::print".to_string());
        let prelude_export_ids = BTreeMap::from([(
            "print".to_string(),
            ArtifactExport {
                source: "stdlib::prelude::print".to_string(),
                id: print_id,
            },
        )]);

        let mut crate_ctx = CrateContext::new();
        add_artifact_extern_crate(
            &mut crate_ctx,
            "stdlib",
            interface,
            resolver,
            prelude_export_ids,
        );
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![],
                is_inline: false,
                filepath: None,
            },
        };

        let decls = collect(&program, &crate_ctx, true, Some("test"))
            .expect("collect should record stdlib prelude alias IDs");

        assert_eq!(decls.resolver.import_aliases.get("print"), Some(&print_id));
        assert_eq!(
            decls.resolver.item_names_by_id.get(&print_id),
            Some(&"stdlib::prelude::print".to_string())
        );
    }

    #[test]
    fn collect_preserves_non_stdlib_prelude_capability_without_unqualified_injection() {
        let util_id = DefId::new(CrateId(9), LocalDefId(3));
        let mut util = hir_function("util");
        util.id = util_id;

        let mut interface = ArtifactCrateInterface::default();
        interface.insert_function("dep::prelude::util".to_string(), util);

        let mut resolver = crate::collect::resolver::ResolverTables::default();
        resolver
            .item_paths
            .insert("dep::prelude::util".to_string(), util_id);
        resolver
            .item_names_by_id
            .insert(util_id, "dep::prelude::util".to_string());
        let prelude_export_ids = BTreeMap::from([(
            "util".to_string(),
            ArtifactExport {
                source: "dep::prelude::util".to_string(),
                id: util_id,
            },
        )]);

        let mut crate_ctx = CrateContext::new();
        add_artifact_extern_crate(
            &mut crate_ctx,
            "dep",
            interface,
            resolver,
            prelude_export_ids,
        );
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![],
                is_inline: false,
                filepath: None,
            },
        };

        let decls = collect(&program, &crate_ctx, true, Some("test"))
            .expect("collect should preserve non-stdlib prelude providers without injection");

        assert!(decls.resolver.import_aliases.get("util").is_none());
        assert!(decls.items.functions().contains_key(&util_id));
        assert_eq!(decls.items.functions().len(), 1);
    }

    #[test]
    fn collect_context_rejects_source_backed_dependency_crate() {
        let mut crate_ctx = CrateContext::new();
        crate_ctx.register_crate(
            crate::crate_system::CrateManifest {
                crate_: crate::crate_system::CrateConfig {
                    name: "dep".to_string(),
                    version: "0.1.0".to_string(),
                    no_std: false,
                },
                lib: crate::crate_system::LibConfig {
                    path: "lib.rk".to_string(),
                },
                dependencies: None,
            },
            PathBuf::from("/dep"),
            Module {
                name: None,
                top_levels: vec![TopLevel::StructDecl(StructDecl {
                    name: type_inner("DepThing"),
                    generic_params: vec![],
                    fields: vec![],
                    exported: false,
                })],
                is_inline: false,
                filepath: None,
            },
        );

        let mut context =
            context::CollectContext::bootstrap_for_collection(false, Some("test"), None);
        context.register_crate_functions(&crate_ctx);

        assert!(crate_ctx.source_crate("dep").is_some());
        assert!(!context.structs.contains_key("dep::DepThing"));
        assert!(!context
            .loaded_module_paths
            .iter()
            .any(|(name, _)| name == "dep"));
        assert!(context.errors.is_empty());
    }

    #[test]
    fn collect_context_records_canonical_aliases_for_dependency_crates() {
        let mut context =
            context::CollectContext::bootstrap_for_collection(false, Some("test"), None);
        let writer_id = DefId::new(CrateId(0), LocalDefId(0));
        context.structs.insert(
            "dep::io::Writer".to_string(),
            HirStruct {
                id: writer_id,
                name: "Writer".to_string(),
                generic_params: vec![],
                fields: vec![],
            },
        );

        let path = crate::ast::Path::Ident(crate::ast::IdentifierPath {
            path: vec![
                crate::ast::IdentOrType::Ident(ident("dep")),
                crate::ast::IdentOrType::Ident(ident("io")),
                crate::ast::IdentOrType::Ident(ident("Writer")),
            ],
        });
        context.handle_import(&path, false, None);

        let collection =
            context.into_local_collection(&ItemIndex::new(), ModuleId(0), Some("test"));
        assert!(collection.structs.contains_key("dep::io::Writer"));
        assert!(!collection.structs.contains_key("Writer"));
        assert_eq!(
            collection.resolver.import_aliases.get("Writer").copied(),
            Some(writer_id)
        );
        assert_eq!(
            collection.resolver.canonical_name(writer_id),
            Some("dep::io::Writer")
        );
    }

    #[test]
    fn collect_context_bootstraps_interface_stdlib_prelude() {
        let print_id = DefId::new(CrateId(0), LocalDefId(0));
        let mut interface = ArtifactCrateInterface::default();
        interface.insert_function(
            "stdlib::print".to_string(),
            HirFunction {
                id: print_id,
                name: "print".to_string(),
                generic_params: vec![],
                generic_bounds: HashMap::new().into(),
                params: vec![],
                ret_type: Type::Unit,
                body: HirBlock {
                    stmts: vec![],
                    ty: Type::Unit,
                },
                is_curried: false,
                is_method: false,
                self_receiver: None,
                is_unsafe: false,
            },
        );

        let mut crate_ctx = CrateContext::new();
        crate_ctx
            .add_extern_crate(crate::crate_system::ExternCrateRecord::new(
                CrateId(1),
                "stdlib".to_string(),
                crate::crate_system::ExternCrateMetadata::new(
                    interface,
                    crate::collect::resolver::ResolverTables::default(),
                    BTreeMap::from([(
                        "print".to_string(),
                        ArtifactExport {
                            source: "stdlib::print".to_string(),
                            id: print_id,
                        },
                    )]),
                ),
                crate::crate_system::ExternCrateBodies::default(),
                crate::crate_system::ExternCrateLink::metadata_only(BTreeMap::new()),
            ))
            .unwrap();

        let mut context =
            context::CollectContext::bootstrap_for_collection(true, Some("test"), None);
        context.register_crate_functions(&crate_ctx);
        context.inject_loaded_prelude(&crate_ctx);

        assert_eq!(
            context.import_aliases.get("print"),
            Some(&"stdlib::print".to_string())
        );
        assert!(context.functions.contains_key("stdlib::print"));
        assert!(!context.functions.contains_key("print"));
    }

    #[test]
    fn collect_artifact_declarations_skips_current_crate_during_dependency_bootstrap() {
        let io_path = PathBuf::from("/dep/io.rk");
        let dep_module = Module {
            name: None,
            top_levels: vec![
                TopLevel::StructDecl(StructDecl {
                    name: type_inner("Point"),
                    generic_params: vec![],
                    fields: vec![],
                    exported: false,
                }),
                TopLevel::Impl(inherent_impl("Point", "origin")),
                TopLevel::Mod(ident("io"), true),
            ],
            is_inline: false,
            filepath: Some(PathBuf::from("/dep/lib.rk")),
        };
        let dep_io_module = Module {
            name: Some(ident("io")),
            top_levels: vec![TopLevel::StructDecl(StructDecl {
                name: type_inner("Writer"),
                generic_params: vec![],
                fields: vec![],
                exported: false,
            })],
            is_inline: false,
            filepath: Some(io_path.clone()),
        };

        let mut support_interface = ArtifactCrateInterface::default();
        support_interface.insert_struct(
            "support::Helper".to_string(),
            crate::hir::HirStruct {
                id: DefId::new(CrateId(0), LocalDefId(0)),
                name: "Helper".to_string(),
                generic_params: vec![],
                fields: vec![],
            },
        );

        let mut crate_ctx = CrateContext::new();
        crate_ctx.register_crate(
            crate::crate_system::CrateManifest {
                crate_: crate::crate_system::CrateConfig {
                    name: "dep".to_string(),
                    version: "0.1.0".to_string(),
                    no_std: false,
                },
                lib: crate::crate_system::LibConfig {
                    path: "lib.rk".to_string(),
                },
                dependencies: None,
            },
            PathBuf::from("/dep"),
            dep_module.clone(),
        );
        let source_crate = crate_ctx.source_crate_mut("dep").unwrap();
        source_crate
            .loaded_module_paths
            .push(("dep".to_string(), PathBuf::from("/dep/lib.rk")));
        source_crate
            .loaded_module_paths
            .push(("dep::io".to_string(), io_path.clone()));
        source_crate
            .file_cache
            .insert(io_path.clone(), dep_io_module);
        add_artifact_extern_crate(
            &mut crate_ctx,
            "support",
            support_interface,
            crate::collect::resolver::ResolverTables::default(),
            BTreeMap::new(),
        );

        let decls = collect_artifact_declarations(&crate_ctx, false, "dep")
            .expect("artifact declaration collection should succeed");
        let decls = decls.declarations;

        let point_id = decls.item_index.defs_named("Point")[0];
        assert!(decls.items.structs().contains_key(&point_id));
        assert_eq!(decls.items.structs().len(), 2);

        let point_impls: Vec<_> = decls
            .items
            .impls()
            .values()
            .filter(|imp| imp.type_name == "Point" && imp.trait_name.is_none())
            .collect();
        assert_eq!(point_impls.len(), 1);
        assert_eq!(point_impls[0].methods.len(), 1);
        assert!(point_impls[0].methods.contains_key("origin"));
        assert!(decls.module_file_cache.contains_key(&io_path));
        assert!(struct_named(&decls, "dep::io::Writer").is_some());

        let point = decls.item_index.get(point_id).unwrap();
        assert_eq!(point.module_id, decls.indexing_ids.root_module_id());
        assert_eq!(point.def_id.crate_id, decls.indexing_ids.root_crate_id());

        let writer_id = decls.item_index.defs_named("Writer")[0];
        let writer = decls.item_index.get(writer_id).unwrap();
        assert_ne!(writer.module_id, decls.indexing_ids.root_module_id());
    }

    #[test]
    fn collect_artifact_declarations_preserves_indexed_inline_and_impl_ids() {
        let mut crate_ctx = CrateContext::new();
        crate_ctx.register_crate(
            crate::crate_system::CrateManifest {
                crate_: crate::crate_system::CrateConfig {
                    name: "dep".to_string(),
                    version: "0.1.0".to_string(),
                    no_std: false,
                },
                lib: crate::crate_system::LibConfig {
                    path: "lib.rk".to_string(),
                },
                dependencies: None,
            },
            PathBuf::from("/dep"),
            Module {
                name: None,
                top_levels: vec![
                    TopLevel::InfixOperator(7, "++".to_string()),
                    TopLevel::StructDecl(StructDecl {
                        name: type_inner("Same"),
                        generic_params: vec![],
                        fields: vec![],
                        exported: false,
                    }),
                    inline_module(
                        "inline",
                        vec![TopLevel::StructDecl(StructDecl {
                            name: type_inner("Same"),
                            generic_params: vec![],
                            fields: vec![],
                            exported: true,
                        })],
                    ),
                    TopLevel::GlobExport(vec!["inline".to_string()]),
                    TopLevel::Impl(inherent_impl("Same", "first")),
                    TopLevel::Impl(inherent_impl("Same", "second")),
                ],
                is_inline: false,
                filepath: Some(PathBuf::from("/dep/lib.rk")),
            },
        );

        let decls = collect_artifact_declarations(&crate_ctx, false, "dep")
            .expect("artifact declaration collection should preserve source identities")
            .declarations;
        let root_module_id = decls.indexing_ids.root_module_id();
        let inline_module_id = decls
            .item_index
            .module_id_by_path(&["inline".to_string()])
            .unwrap();

        assert!(decls.items.structs().contains_key(
            &decls
                .item_index
                .item_at_source(root_module_id, 1)
                .unwrap()
                .def_id
        ));
        assert!(decls.items.structs().contains_key(
            &decls
                .item_index
                .item_at_source(inline_module_id, 0)
                .unwrap()
                .def_id
        ));
        assert_eq!(
            decls
                .items
                .impls()
                .values()
                .map(|impl_| impl_.id)
                .collect::<BTreeSet<_>>(),
            [
                decls
                    .item_index
                    .item_at_source(root_module_id, 4)
                    .unwrap()
                    .def_id,
                decls
                    .item_index
                    .item_at_source(root_module_id, 5)
                    .unwrap()
                    .def_id,
            ]
            .into_iter()
            .collect()
        );
    }

    #[test]
    fn collect_artifact_declarations_builds_generic_nominal_and_owner_ids_from_item_records() {
        let mut crate_ctx = CrateContext::new();
        crate_ctx.register_crate(
            crate::crate_system::CrateManifest {
                crate_: crate::crate_system::CrateConfig {
                    name: "dep".to_string(),
                    version: "0.1.0".to_string(),
                    no_std: false,
                },
                lib: crate::crate_system::LibConfig {
                    path: "lib.rk".to_string(),
                },
                dependencies: None,
            },
            PathBuf::from("/dep"),
            generic_declaration_program().module,
        );

        let decls = collect_artifact_declarations(&crate_ctx, false, "dep")
            .expect("artifact collection should use indexed IDs during construction")
            .declarations;

        assert_generic_declaration_ids(&decls);
    }

    #[test]
    fn collect_artifact_declarations_preserves_function_signatures_with_indexed_ids() {
        let mut crate_ctx = CrateContext::new();
        crate_ctx.register_crate(
            crate::crate_system::CrateManifest {
                crate_: crate::crate_system::CrateConfig {
                    name: "dep".to_string(),
                    version: "0.1.0".to_string(),
                    no_std: false,
                },
                lib: crate::crate_system::LibConfig {
                    path: "lib.rk".to_string(),
                },
                dependencies: None,
            },
            PathBuf::from("/dep"),
            Module {
                name: None,
                top_levels: vec![TopLevel::FunctionSig(function_sig(
                    "declared_only",
                    ParseType::Function(vec![ParseType::Type(type_inner("I64"))]),
                ))],
                is_inline: false,
                filepath: Some(PathBuf::from("/dep/lib.rk")),
            },
        );

        let decls = collect_artifact_declarations(&crate_ctx, false, "dep")
            .expect("artifact declaration collection should preserve signatures")
            .declarations;
        let signature = function_sig_named(&decls, "declared_only")
            .expect("qualified signature should be collected");

        assert_eq!(
            signature.id,
            decls.item_index.defs_named("declared_only")[0]
        );
        assert!(decls.current_def_ids.contains(&signature.id));
        assert_no_invalid_current_crate_ids(&decls);
    }

    #[test]
    fn collect_artifact_declarations_expands_inline_module_glob_exports_with_qualified_prefix() {
        let inner_path = PathBuf::from("/dep/outer/inner.rk");
        let mut crate_ctx = CrateContext::new();
        crate_ctx.register_crate(
            crate::crate_system::CrateManifest {
                crate_: crate::crate_system::CrateConfig {
                    name: "dep".to_string(),
                    version: "0.1.0".to_string(),
                    no_std: false,
                },
                lib: crate::crate_system::LibConfig {
                    path: "lib.rk".to_string(),
                },
                dependencies: None,
            },
            PathBuf::from("/dep"),
            Module {
                name: None,
                top_levels: vec![
                    inline_module(
                        "outer",
                        vec![
                            TopLevel::Mod(ident("inner"), false),
                            TopLevel::GlobExport(vec!["inner".to_string()]),
                        ],
                    ),
                    TopLevel::GlobExport(vec!["outer".to_string()]),
                ],
                is_inline: false,
                filepath: Some(PathBuf::from("/dep/lib.rk")),
            },
        );
        let source_crate = crate_ctx.source_crate_mut("dep").unwrap();
        source_crate
            .loaded_module_paths
            .push(("dep::outer::inner".to_string(), inner_path.clone()));
        source_crate.file_cache.insert(
            inner_path.clone(),
            Module {
                name: Some(ident("inner")),
                top_levels: vec![TopLevel::StructDecl(StructDecl {
                    name: type_inner("Thing"),
                    generic_params: vec![],
                    fields: vec![],
                    exported: true,
                })],
                is_inline: false,
                filepath: Some(inner_path),
            },
        );

        let decls = collect_artifact_declarations(&crate_ctx, false, "dep")
            .expect("artifact declaration collection should expand nested glob exports")
            .declarations;
        let thing_id = decls.item_index.defs_named("Thing")[0];

        assert!(decls.items.structs().contains_key(&thing_id));
        assert_eq!(
            decls.resolver.item_paths.get("dep::outer::inner::Thing"),
            Some(&thing_id)
        );
        assert_eq!(
            decls.resolver.export_aliases.get("dep::outer::Thing"),
            Some(&thing_id)
        );
    }

    #[test]
    fn collect_artifact_declarations_uses_source_crate_loaded_module_paths() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_collect_artifact_source_paths_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(temp_dir.join("src").join("util")).unwrap();
        std::fs::write(
            temp_dir.join("rock.toml"),
            "[crate]\nname = \"dep\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"src/lib.rk\"\n",
        )
        .unwrap();
        std::fs::write(temp_dir.join("src").join("lib.rk"), "mod util\n< util::*\n").unwrap();
        std::fs::write(
            temp_dir.join("src").join("util").join("mod.rk"),
            "< mod sub\n< sub::*\n",
        )
        .unwrap();
        std::fs::write(
            temp_dir.join("src").join("util").join("sub.rk"),
            "< struct Writer\n",
        )
        .unwrap();

        let mut crate_ctx = CrateContext::new();
        crate_ctx
            .load_crate_from_dir(temp_dir.clone())
            .expect("source crate should load");

        let decls = collect_artifact_declarations(&crate_ctx, false, "dep")
            .expect("artifact declaration collection should use source graph module paths")
            .declarations;

        assert!(struct_named(&decls, "dep::util::sub::Writer").is_some());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn collect_artifact_declarations_does_not_collect_unreferenced_same_named_sibling_module() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_collect_artifact_same_named_sibling_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(temp_dir.join("src").join("b")).unwrap();
        std::fs::write(
            temp_dir.join("rock.toml"),
            "[crate]\nname = \"dep\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"src/lib.rk\"\n",
        )
        .unwrap();
        std::fs::write(
            temp_dir.join("src").join("lib.rk"),
            "mod a\nmod b\n< b::a::*\n",
        )
        .unwrap();
        std::fs::write(temp_dir.join("src").join("a.rk"), "< struct Wrong\n").unwrap();
        std::fs::write(temp_dir.join("src").join("b").join("mod.rk"), "mod a\n").unwrap();
        std::fs::write(
            temp_dir.join("src").join("b").join("a.rk"),
            "< struct Right\n",
        )
        .unwrap();

        let mut crate_ctx = CrateContext::new();
        crate_ctx
            .load_crate_from_dir(temp_dir.clone())
            .expect("source crate should load");

        let decls = collect_artifact_declarations(&crate_ctx, false, "dep")
            .expect("artifact declaration collection should succeed")
            .declarations;

        assert!(struct_named(&decls, "dep::b::a::Right").is_some());
        assert!(struct_named(&decls, "dep::a::Wrong").is_none());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn collect_artifact_declarations_preserves_child_exports_when_parent_reference_key_collides() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_collect_artifact_export_collision_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(temp_dir.join("src").join("b")).unwrap();
        std::fs::write(
            temp_dir.join("rock.toml"),
            "[crate]\nname = \"dep\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"src/lib.rk\"\n",
        )
        .unwrap();
        std::fs::write(
            temp_dir.join("src").join("lib.rk"),
            "mod b\n< b::a::Thing\n",
        )
        .unwrap();
        std::fs::write(
            temp_dir.join("src").join("b").join("mod.rk"),
            "mod a\nmod c\n< c::*\n",
        )
        .unwrap();
        std::fs::write(
            temp_dir.join("src").join("b").join("a.rk"),
            "< struct Thing\n",
        )
        .unwrap();
        std::fs::write(
            temp_dir.join("src").join("b").join("c.rk"),
            "< struct Thing\n",
        )
        .unwrap();

        let mut crate_ctx = CrateContext::new();
        crate_ctx
            .load_crate_from_dir(temp_dir.clone())
            .expect("source crate should load");

        let decls = collect_artifact_declarations(&crate_ctx, false, "dep")
            .expect("artifact declaration collection should succeed")
            .declarations;

        assert!(struct_named(&decls, "dep::b::a::Thing").is_some());
        assert!(struct_named(&decls, "dep::b::c::Thing").is_some());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn collect_artifact_declarations_capture_dependency_bootstrap_for_cross_crate_hir() {
        let support_identity_id = DefId::new(CrateId(7), LocalDefId(42));
        let dep_module = Module {
            name: None,
            top_levels: vec![
                TopLevel::FunctionDecl(FunctionDecl {
                    name: ident("identity"),
                    lambda: LambdaDecl {
                        parameters: vec![],
                        body: crate::ast::Block { statements: vec![] },
                        arrow_kind: LambdaArrowKind::Normal,
                        span: crate::lexer::Span::test(),
                    },
                    self_receiver: None,
                    is_unsafe: false,
                    exported: true,
                }),
                TopLevel::StructDecl(StructDecl {
                    name: type_inner("Box"),
                    generic_params: vec![],
                    fields: vec![],
                    exported: false,
                }),
                TopLevel::TraitDecl(crate::ast::TraitDecl {
                    where_clauses: Vec::new(),
                    name: type_inner("Show"),
                    generic_params: vec![],
                    for_: None,
                    associated_types: vec![],
                    methods: HashMap::from([(ident("show"), function_decl("show"))]),
                    signatures: HashMap::new(),
                    exported: false,
                    language_items: Default::default(),
                }),
                TopLevel::Impl(trait_impl("Box", "Show", "show")),
            ],
            is_inline: false,
            filepath: Some(PathBuf::from("/dep/lib.rk")),
        };

        let mut support_interface = ArtifactCrateInterface::default();
        support_interface.insert_function(
            "support::identity".to_string(),
            HirFunction {
                id: support_identity_id,
                name: "identity".to_string(),
                generic_params: vec![],
                generic_bounds: HashMap::new().into(),
                params: vec![],
                ret_type: Type::Unit,
                body: HirBlock {
                    stmts: vec![],
                    ty: Type::Unit,
                },
                is_curried: false,
                is_method: false,
                self_receiver: None,
                is_unsafe: false,
            },
        );
        support_interface.insert_struct(
            "support::Helper".to_string(),
            crate::hir::HirStruct {
                id: DefId::new(CrateId(0), LocalDefId(0)),
                name: "Helper".to_string(),
                generic_params: vec![],
                fields: vec![],
            },
        );

        let mut crate_ctx = CrateContext::new();
        crate_ctx.register_crate(
            crate::crate_system::CrateManifest {
                crate_: crate::crate_system::CrateConfig {
                    name: "dep".to_string(),
                    version: "0.1.0".to_string(),
                    no_std: false,
                },
                lib: crate::crate_system::LibConfig {
                    path: "lib.rk".to_string(),
                },
                dependencies: None,
            },
            PathBuf::from("/dep"),
            dep_module,
        );
        add_artifact_extern_crate(
            &mut crate_ctx,
            "support",
            support_interface,
            crate::collect::resolver::ResolverTables::default(),
            BTreeMap::new(),
        );

        let artifact = collect_artifact_hir_declarations(&crate_ctx, false, "dep")
            .expect("artifact HIR declaration collection should succeed");

        assert!(artifact
            .declarations
            .items
            .functions()
            .contains_key(&support_identity_id));
        assert!(artifact
            .declarations
            .items
            .structs()
            .values()
            .any(|structure| structure.name == "Helper"));
        assert!(artifact
            .cross_crate_hir_bootstrap
            .function_keys
            .contains("support::identity"));
        assert!(artifact
            .cross_crate_hir_bootstrap
            .struct_keys
            .contains("support::Helper"));

        let decls = &artifact.declarations;
        let trait_id = decls.item_index.defs_named("Show")[0];
        let trait_def = decls.items.traits().get(&trait_id).unwrap();
        let impl_ = decls
            .items
            .impls()
            .values()
            .find(|imp| imp.type_name == "Box" && imp.trait_name.as_deref() == Some("Show"))
            .expect("current crate artifact impl should be collected");
        let impl_id = impl_.id;
        let trait_method_id = trait_def.methods["show"].id;
        let impl_method_id = impl_.methods["show"].id;

        assert_ne!(trait_method_id, trait_id);
        assert_ne!(impl_method_id, impl_id);
        assert_ne!(trait_method_id, impl_method_id);
        assert_eq!(impl_.methods["show"].id, impl_method_id);
        assert!(decls.current_def_ids.contains(&trait_method_id));
        assert!(decls.current_def_ids.contains(&impl_method_id));
    }

    #[test]
    fn generated_current_marked_sized_impl_is_emitted_to_products() {
        let sized_id = DefId::new(CrateId(7), LocalDefId(1));
        let mut interface = ArtifactCrateInterface::default();
        interface.insert_trait(
            "core::StaticLayout".to_string(),
            hir_trait(sized_id, "StaticLayout", &[]),
        );
        let mut resolver = crate::collect::resolver::ResolverTables::default();
        resolver
            .item_paths
            .insert("core::StaticLayout".to_string(), sized_id);
        resolver
            .item_names_by_id
            .insert(sized_id, "core::StaticLayout".to_string());
        let mut crate_ctx = CrateContext::new();
        add_artifact_extern_crate_with_language_items(
            &mut crate_ctx,
            "core",
            interface,
            resolver,
            LanguageItems {
                sized: Some(SizedLanguageItems { trait_id: sized_id }),
                ..LanguageItems::default()
            },
        );

        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![TopLevel::StructDecl(StructDecl {
                    name: type_inner("Point"),
                    generic_params: vec![],
                    fields: vec![],
                    exported: false,
                })],
                is_inline: false,
                filepath: None,
            },
        };
        let decls =
            collect(&program, &crate_ctx, false, Some("demo")).expect("collection should succeed");
        let partial = crate::lower::program::lower_from_declarations(
            &program,
            decls,
            &crate_ctx,
            Some("demo"),
        )
        .expect("lowering should auto-impl the marked trait");
        let resolved = crate::infer::finalize(partial).expect("inference should finalize");
        let sized_impl = resolved
            .program
            .impls_in_order()
            .map(|(_, imp)| imp)
            .find(|imp| {
                imp.type_name == "Point" && imp.trait_name.as_deref() == Some("StaticLayout")
            })
            .expect("Point should have generated marked Sized impl");
        assert_eq!(sized_impl.trait_id, Some(sized_id));

        let products = CompilerProducts::from_resolved_hir(
            ProductCrateIdentity::local("demo".to_string()),
            &resolved,
            Vec::new(),
            std::collections::BTreeMap::new(),
            ProductSourceFingerprint::default(),
            ProductLinkData::default(),
        )
        .expect("generated marked Sized HIR has valid product language items");

        assert!(products
            .interface
            .impls
            .contains_key(&ProductDefId::from(sized_impl.id)));
    }

    #[test]
    fn collect_carries_indexing_ids_used_by_item_index() {
        let program = program_with_struct("Point");
        let crate_ctx = CrateContext::new();

        let decls = match collect(&program, &crate_ctx, false, Some("test")) {
            Ok(decls) => decls,
            Err(_) => panic!("collect should succeed"),
        };

        assert_eq!(decls.indexing_ids.root_crate_id(), CrateId(0));
        assert_eq!(decls.indexing_ids.root_module_id(), ModuleId(0));

        let point_id = decls.item_index.defs_named("Point")[0];
        let point = decls.item_index.get(point_id).unwrap();

        assert_eq!(point.def_id, DefId::new(CrateId(0), LocalDefId(0)));
        assert_eq!(point.def_id.crate_id, decls.indexing_ids.root_crate_id());
        assert_eq!(point.module_id, decls.indexing_ids.root_module_id());
    }

    #[test]
    fn collect_indexes_loaded_source_backed_module_bodies() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_collect_item_index_{}_{}",
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
        std::fs::write(&io_path, "struct Writer\n").unwrap();
        let (program, graph) = load_source_program(root_path, "mod io\n", Some("test"));
        let crate_ctx = CrateContext::new();

        let decls = collect_with_source_graph(&program, &graph, &crate_ctx, false, Some("test"))
            .expect("collect should load and index io module");

        let writer = decls
            .item_index
            .get(decls.item_index.defs_named("Writer")[0])
            .unwrap();
        assert_eq!(writer.module_id, ModuleId(1));
        assert_eq!(writer.def_id, DefId::new(CrateId(0), LocalDefId(1)));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn collect_indexes_source_backed_module_nested_inside_inline_module() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_collect_nested_item_index_{}_{}",
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
        std::fs::write(&io_path, "struct Writer\n").unwrap();
        let (_, graph) = load_source_program(root_path.clone(), "mod io\n", Some("math"));
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![inline_module(
                    "math",
                    vec![TopLevel::Mod(ident("io"), false)],
                )],
                is_inline: false,
                filepath: Some(root_path),
            },
        };
        let crate_ctx = CrateContext::new();

        let decls = collect_with_source_graph(&program, &graph, &crate_ctx, false, Some("test"))
            .expect("collect should load and index nested io module");

        let writer = decls
            .item_index
            .get(decls.item_index.defs_named("Writer")[0])
            .unwrap();
        assert_eq!(writer.module_id, ModuleId(2));
        assert_eq!(writer.def_id, DefId::new(CrateId(0), LocalDefId(2)));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn collect_indexes_current_crate_prefixed_source_backed_module_inside_inline_module() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_collect_current_crate_nested_item_index_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let root_path = temp_dir.join("main.rk");
        let math_path = temp_dir.join("math.rk");
        let io_path = temp_dir.join("io.rk");
        std::fs::write(&math_path, "mod io\n").unwrap();
        std::fs::write(&io_path, "struct Writer\n").unwrap();
        let (_, graph) = load_source_program(root_path.clone(), "mod math\n", Some("test"));
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![inline_module(
                    "math",
                    vec![TopLevel::Mod(ident("io"), false)],
                )],
                is_inline: false,
                filepath: Some(root_path),
            },
        };
        assert!(graph
            .loaded_module_paths()
            .iter()
            .any(|(name, path)| name == "test::math::io" && path == &io_path));

        let decls =
            collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
                .expect("collect should use current-crate-prefixed graph module path");

        assert!(!decls
            .loaded_module_paths
            .iter()
            .any(|(name, _)| name == "math::io"));
        assert_eq!(decls.item_index.defs_named("Writer").len(), 1);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn collect_module_defines_local_collector() {
        let context = context::CollectContext::bootstrap_for_collection(false, None, None);
        let _ = super::collector::LocalCollector::new(context);
    }

    #[test]
    fn collect_gathers_local_inline_and_source_backed_declarations_without_lowerer_collect() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_collect_local_only_{}_{}",
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
        std::fs::write(&io_path, "struct Writer\n").unwrap();
        let (_, graph) = load_source_program(root_path.clone(), "mod io\n", Some("test"));
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![
                    TopLevel::StructDecl(StructDecl {
                        name: type_inner("RootThing"),
                        generic_params: vec![],
                        fields: vec![],
                        exported: false,
                    }),
                    inline_module(
                        "math",
                        vec![TopLevel::StructDecl(StructDecl {
                            name: type_inner("Vector"),
                            generic_params: vec![],
                            fields: vec![],
                            exported: false,
                        })],
                    ),
                    TopLevel::Mod(ident("io"), false),
                ],
                is_inline: false,
                filepath: Some(root_path),
            },
        };

        let decls =
            collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
                .expect("collect should gather local declarations");

        assert!(struct_named(&decls, "RootThing").is_some());
        assert!(struct_named(&decls, "Vector").is_some());
        assert!(struct_named(&decls, "test::io::Writer").is_some());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn collect_remaps_signature_backed_function_generics_to_function_owner() {
        let generic = ParseType::Type(type_inner("A"));
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![
                    TopLevel::FunctionSig(function_sig(
                        "id",
                        ParseType::Function(vec![generic.clone(), generic]),
                    )),
                    TopLevel::FunctionDecl(single_param_function_decl("id", "value")),
                ],
                is_inline: false,
                filepath: None,
            },
        };

        let decls = collect(&program, &CrateContext::new(), false, Some("test"))
            .expect("collect should merge function signature and declaration");
        let function =
            function_named(&decls, "id").expect("signature-backed function should be collected");
        let generic_id = GenericParamId {
            owner: function.id,
            index: 0,
        };

        assert_eq!(
            function.generic_params,
            vec![GenericParamDecl::type_param(generic_id, "A")]
        );
        assert_eq!(function.params[0].ty, Type::Generic(generic_id));
        assert_eq!(function.ret_type, Type::Generic(generic_id));
    }

    #[test]
    fn collect_ignores_source_backed_external_dependency_consumption() {
        let mut crate_ctx = CrateContext::new();
        crate_ctx.register_crate(
            crate::crate_system::CrateManifest {
                crate_: crate::crate_system::CrateConfig {
                    name: "dep".to_string(),
                    version: "0.1.0".to_string(),
                    no_std: false,
                },
                lib: crate::crate_system::LibConfig {
                    path: "lib.rk".to_string(),
                },
                dependencies: None,
            },
            std::path::PathBuf::from("/dep"),
            Module {
                name: None,
                top_levels: vec![TopLevel::StructDecl(StructDecl {
                    name: type_inner("DepThing"),
                    generic_params: vec![],
                    fields: vec![],
                    exported: false,
                })],
                is_inline: false,
                filepath: None,
            },
        );

        let program = program_with_struct("Point");
        let decls = collect(&program, &crate_ctx, false, Some("test"))
            .expect("source-backed external dependency should not be registered as an extern");

        assert!(crate_ctx.source_crate("dep").is_some());
        assert!(struct_named(&decls, "dep::DepThing").is_none());
    }

    #[test]
    fn collect_accepts_artifact_dependency_without_registering_source_module_path() {
        let mut context = context::CollectContext::new();
        let mut crate_ctx = CrateContext::new();
        crate_ctx
            .add_extern_crate(crate::crate_system::ExternCrateRecord::new(
                CrateId(1),
                "dep".to_string(),
                crate::crate_system::ExternCrateMetadata::new(
                    ArtifactCrateInterface::default(),
                    crate::collect::resolver::ResolverTables::default(),
                    BTreeMap::new(),
                ),
                crate::crate_system::ExternCrateBodies::default(),
                crate::crate_system::ExternCrateLink::metadata_only(BTreeMap::new()),
            ))
            .unwrap();

        context.register_extern_crate(crate_ctx.extern_crate("dep").unwrap());

        assert!(context.errors.is_empty());
        assert!(context.loaded_module_paths.is_empty());
    }

    #[test]
    fn collect_registers_artifact_dependency_from_extern_store() {
        let mut crate_ctx = CrateContext::new();
        let id = DefId::new(CrateId(7), LocalDefId(1));
        let mut interface = crate::crate_artifact::ArtifactCrateInterface::default();
        let mut answer = hir_function("answer");
        answer.id = id;
        interface.insert_function("dep::answer".to_string(), answer);
        interface.root_export_ids.insert(
            "answer".to_string(),
            ArtifactExport {
                source: "dep::answer".to_string(),
                id,
            },
        );
        let mut resolver = crate::collect::resolver::ResolverTables::default();
        resolver.item_paths.insert("dep::answer".to_string(), id);
        resolver
            .item_names_by_id
            .insert(id, "dep::answer".to_string());

        crate_ctx
            .add_extern_crate(crate::crate_system::ExternCrateRecord::new(
                CrateId(7),
                "dep".to_string(),
                crate::crate_system::ExternCrateMetadata::new(interface, resolver, BTreeMap::new()),
                crate::crate_system::ExternCrateBodies::default(),
                crate::crate_system::ExternCrateLink::metadata_only(BTreeMap::new()),
            ))
            .unwrap();

        let mut context =
            context::CollectContext::bootstrap_for_collection(false, Some("test"), None);
        context.register_crate_functions(&crate_ctx);

        assert!(context.functions.contains_key("dep::answer"));
        assert_eq!(
            context
                .dependency_root_export_ids
                .get("dep")
                .and_then(|exports| exports.get("answer"))
                .map(|export| export.source.as_str()),
            Some("dep::answer")
        );
    }

    #[test]
    fn collect_registers_artifact_dependency_through_provider_capabilities() {
        let mut crate_ctx = CrateContext::new();
        let mut interface = crate::crate_artifact::ArtifactCrateInterface::default();
        let id = DefId::new(CrateId(7), LocalDefId(1));
        let mut answer = hir_function("answer");
        answer.id = id;
        interface.insert_function("dep::answer".to_string(), answer);
        interface.root_export_ids.insert(
            "answer".to_string(),
            ArtifactExport {
                source: "dep::answer".to_string(),
                id,
            },
        );

        add_artifact_extern_crate(
            &mut crate_ctx,
            "dep",
            interface,
            crate::collect::resolver::ResolverTables::default(),
            BTreeMap::new(),
        );

        let mut context =
            context::CollectContext::bootstrap_for_collection(false, Some("test"), None);
        context.register_crate_functions(&crate_ctx);

        assert!(context.functions.contains_key("dep::answer"));
        assert_eq!(
            context
                .dependency_root_export_ids
                .get("dep")
                .and_then(|exports| exports.get("answer"))
                .map(|export| export.source.as_str()),
            Some("dep::answer")
        );
    }

    #[test]
    fn collect_keeps_item_index_and_local_declarations_in_sync() {
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![
                    TopLevel::StructDecl(StructDecl {
                        name: type_inner("Point"),
                        generic_params: vec![],
                        fields: vec![],
                        exported: false,
                    }),
                    inline_module(
                        "math",
                        vec![TopLevel::StructDecl(StructDecl {
                            name: type_inner("Vector"),
                            generic_params: vec![],
                            fields: vec![],
                            exported: false,
                        })],
                    ),
                ],
                is_inline: false,
                filepath: None,
            },
        };

        let decls = collect(&program, &CrateContext::new(), false, Some("test"))
            .expect("collect should succeed");

        assert!(struct_named(&decls, "Point").is_some());
        assert!(struct_named(&decls, "Vector").is_some());
        assert_eq!(decls.item_index.defs_named("Point").len(), 1);
        assert_eq!(decls.item_index.defs_named("Vector").len(), 1);
    }

    #[test]
    fn collect_assigns_canonical_def_ids_to_named_function_and_type_headers() {
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![
                    TopLevel::FunctionDecl(function_decl("root_fn")),
                    TopLevel::StructDecl(StructDecl {
                        name: type_inner("Point"),
                        generic_params: vec![],
                        fields: vec![],
                        exported: false,
                    }),
                    TopLevel::EnumDecl(crate::ast::EnumDecl {
                        name: type_inner("Color"),
                        variants: vec![],
                        exported: false,
                        language_items: Default::default(),
                    }),
                    TopLevel::TraitDecl(crate::ast::TraitDecl {
                        where_clauses: Vec::new(),
                        name: type_inner("Show"),
                        generic_params: vec![],
                        for_: None,
                        associated_types: vec![],
                        methods: HashMap::new(),
                        signatures: HashMap::new(),
                        exported: false,
                        language_items: Default::default(),
                    }),
                    inline_module(
                        "math",
                        vec![TopLevel::FunctionDecl(function_decl("vector_len"))],
                    ),
                ],
                is_inline: false,
                filepath: None,
            },
        };

        let decls = collect(&program, &CrateContext::new(), false, Some("test"))
            .expect("collect should assign canonical ids to named headers");

        assert_eq!(
            function_named(&decls, "root_fn").unwrap().id,
            *decls.resolver.item_paths.get("root_fn").unwrap()
        );
        assert_eq!(
            struct_named(&decls, "Point").unwrap().id,
            *decls.resolver.item_paths.get("Point").unwrap()
        );
        assert_eq!(
            enum_named(&decls, "Color").unwrap().id,
            *decls.resolver.item_paths.get("Color").unwrap()
        );
        assert_eq!(
            trait_named(&decls, "Show").unwrap().id,
            *decls.resolver.item_paths.get("Show").unwrap()
        );
        assert_eq!(
            function_named(&decls, "math::vector_len").unwrap().id,
            *decls.resolver.item_paths.get("math::vector_len").unwrap()
        );
    }

    #[test]
    fn collect_keeps_named_item_ids_separate_from_impl_ids() {
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![
                    TopLevel::StructDecl(StructDecl {
                        name: type_inner("Foo"),
                        generic_params: vec![],
                        fields: vec![],
                        exported: false,
                    }),
                    trait_decl("Display"),
                    trait_decl("Debug"),
                    TopLevel::Impl(inherent_impl("Foo", "new")),
                    TopLevel::Impl(trait_impl("Foo", "Display", "fmt")),
                    TopLevel::Impl(trait_impl("Foo", "Debug", "fmt")),
                ],
                is_inline: false,
                filepath: None,
            },
        };

        let decls = collect(&program, &CrateContext::new(), false, Some("test"))
            .expect("collect should assign canonical ids");

        let struct_id = decls
            .item_index
            .items()
            .iter()
            .find(|item| {
                item.kind == crate::collect::item_index::ItemKind::Struct && item.name == "Foo"
            })
            .map(|item| item.def_id)
            .expect("Foo struct should be indexed");
        let display_trait_id = decls
            .item_index
            .items()
            .iter()
            .find(|item| {
                item.kind == crate::collect::item_index::ItemKind::Trait && item.name == "Display"
            })
            .map(|item| item.def_id)
            .expect("Display trait should be indexed");
        let impl_ids = decls
            .item_index
            .items()
            .iter()
            .filter(|item| item.kind == crate::collect::item_index::ItemKind::Impl)
            .map(|item| item.def_id)
            .collect::<Vec<_>>();

        assert_eq!(decls.resolver.item_paths.get("Foo"), Some(&struct_id));
        assert_eq!(
            decls.resolver.item_paths.get("Display"),
            Some(&display_trait_id)
        );
        assert_eq!(impl_ids.len(), 3);
        assert_eq!(
            decls.items.impls().keys().copied().collect::<BTreeSet<_>>(),
            impl_ids.into_iter().collect()
        );
    }

    #[test]
    fn collect_assigns_unique_def_ids_to_local_trait_and_impl_methods() {
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![
                    TopLevel::StructDecl(StructDecl {
                        name: type_inner("Box"),
                        generic_params: vec![],
                        fields: vec![],
                        exported: false,
                    }),
                    TopLevel::TraitDecl(crate::ast::TraitDecl {
                        where_clauses: Vec::new(),
                        name: type_inner("Show"),
                        generic_params: vec![],
                        for_: None,
                        associated_types: vec![],
                        methods: HashMap::from([(ident("show"), function_decl("show"))]),
                        signatures: HashMap::new(),
                        exported: false,
                        language_items: Default::default(),
                    }),
                    TopLevel::Impl(trait_impl("Box", "Show", "show")),
                ],
                is_inline: false,
                filepath: None,
            },
        };

        let decls = collect(&program, &CrateContext::new(), false, Some("test"))
            .expect("collect should assign canonical method ids");
        let trait_id = trait_named(&decls, "Show").unwrap().id;
        let impl_ = decls.items.impls().values().next().unwrap();
        let impl_id = impl_.id;
        let trait_method_id = trait_named(&decls, "Show").unwrap().methods["show"].id;
        let impl_method_id = impl_.methods["show"].id;

        assert_ne!(trait_method_id, trait_id);
        assert_ne!(impl_method_id, impl_id);
        assert_ne!(trait_method_id, impl_method_id);
        assert!(!decls
            .resolver
            .item_names_by_id
            .contains_key(&trait_method_id));
        assert!(!decls
            .resolver
            .item_names_by_id
            .contains_key(&impl_method_id));
        assert_eq!(impl_.methods["show"].id, impl_method_id);
        assert!(decls.current_def_ids.contains(&trait_method_id));
        assert!(decls.current_def_ids.contains(&impl_method_id));
    }

    #[test]
    fn collect_current_def_ids_do_not_include_overwritten_member_ids() {
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![
                    TopLevel::StructDecl(StructDecl {
                        name: type_inner("Box"),
                        generic_params: vec![],
                        fields: vec![],
                        exported: false,
                    }),
                    TopLevel::TraitDecl(crate::ast::TraitDecl {
                        where_clauses: Vec::new(),
                        name: type_inner("Show"),
                        generic_params: vec![],
                        for_: None,
                        associated_types: vec![],
                        methods: HashMap::from([(ident("show"), function_decl("show"))]),
                        signatures: HashMap::from([(
                            ident("convert"),
                            function_sig(
                                "convert",
                                ParseType::Function(vec![ParseType::Type(type_inner("I64"))]),
                            ),
                        )]),
                        exported: false,
                        language_items: Default::default(),
                    }),
                    TopLevel::Impl(trait_impl("Box", "Show", "show")),
                ],
                is_inline: false,
                filepath: None,
            },
        };

        let decls = collect(&program, &CrateContext::new(), false, Some("test"))
            .expect("collect should assign explicit member ids");
        let mut used_ids = BTreeSet::new();
        used_ids.extend(decls.item_index.items().iter().map(|item| item.def_id));
        for trait_def in decls.items.traits().values() {
            used_ids.extend(trait_def.methods.values().map(|method| method.id));
            used_ids.extend(trait_def.signatures.values().map(|sig| sig.id));
        }
        for imp in decls.items.impls().values() {
            used_ids.extend(imp.methods.values().map(|method| method.id));
        }

        let orphan_ids = decls
            .current_def_ids
            .difference(&used_ids)
            .copied()
            .collect::<Vec<_>>();
        assert!(
            orphan_ids.is_empty(),
            "current_def_ids contains IDs not used by collected declarations: {orphan_ids:?}"
        );
    }

    #[test]
    fn collect_assigns_inline_module_trait_and_impl_member_ids() {
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![
                    trait_decl("Show"),
                    inline_module(
                        "math",
                        vec![
                            TopLevel::StructDecl(StructDecl {
                                name: type_inner("Box"),
                                generic_params: vec![],
                                fields: vec![],
                                exported: false,
                            }),
                            TopLevel::TraitDecl(crate::ast::TraitDecl {
                                where_clauses: Vec::new(),
                                name: type_inner("Show"),
                                generic_params: vec![],
                                for_: None,
                                associated_types: vec![],
                                methods: HashMap::from([(ident("show"), function_decl("show"))]),
                                signatures: HashMap::new(),
                                exported: false,
                                language_items: Default::default(),
                            }),
                            TopLevel::Impl(trait_impl("Box", "Show", "show")),
                        ],
                    ),
                ],
                is_inline: false,
                filepath: None,
            },
        };

        let decls = collect(&program, &CrateContext::new(), false, Some("test"))
            .expect("inline module trait and impl members should use indexed IDs");
        let trait_def = decls
            .items
            .traits()
            .values()
            .find(|trait_def| trait_def.methods.contains_key("show"))
            .expect("trait should be collected");
        let impl_def = decls
            .items
            .impls()
            .values()
            .find(|imp| imp.trait_name.as_deref() == Some("Show"))
            .expect("impl should be collected");

        assert!(decls.current_def_ids.contains(&trait_def.id));
        assert!(decls.current_def_ids.contains(&impl_def.id));
        assert!(decls
            .current_def_ids
            .contains(&trait_def.methods["show"].id));
        assert!(decls.current_def_ids.contains(&impl_def.methods["show"].id));
    }

    #[test]
    fn collect_builds_reverse_canonical_tables_for_current_crate_items() {
        let fixture = current_crate_canonical_fixture();
        let decls = collect_with_source_graph(
            &fixture.program,
            &fixture.graph,
            &CrateContext::new(),
            false,
            Some("test"),
        )
        .expect("collect should build canonical resolver tables");

        let root_thing_id = decls.item_index.defs_named("RootThing")[0];
        let vector_id = decls.item_index.defs_named("Vector")[0];
        let writer_id = decls.item_index.defs_named("Writer")[0];
        let io_module_def_id = decls.item_index.defs_named("io")[0];

        let math_module_id = *decls
            .resolver
            .module_paths
            .get("math")
            .expect("math module should be indexed");
        let io_module_id = *decls
            .resolver
            .module_paths
            .get("test::io")
            .expect("test::io module should be indexed");

        assert_eq!(
            decls.resolver.item_paths.get("math"),
            Some(&decls.item_index.defs_named("math")[0])
        );
        assert_eq!(
            decls.resolver.item_paths.get("test::io"),
            Some(&io_module_def_id)
        );

        assert_eq!(
            decls
                .resolver
                .module_names_by_id
                .get(&math_module_id)
                .map(String::as_str),
            Some("math")
        );
        assert_eq!(
            decls
                .resolver
                .module_names_by_id
                .get(&io_module_id)
                .map(String::as_str),
            Some("test::io")
        );

        assert_eq!(
            decls.resolver.item_paths.get("RootThing"),
            Some(&root_thing_id)
        );
        assert_eq!(
            decls.resolver.item_paths.get("math::Vector"),
            Some(&vector_id)
        );
        assert_eq!(
            decls.resolver.item_paths.get("test::io::Writer"),
            Some(&writer_id)
        );

        assert_eq!(
            decls
                .resolver
                .item_names_by_id
                .get(&root_thing_id)
                .map(String::as_str),
            Some("RootThing")
        );
        assert_eq!(
            decls
                .resolver
                .item_names_by_id
                .get(&vector_id)
                .map(String::as_str),
            Some("math::Vector")
        );
        assert_eq!(
            decls
                .resolver
                .item_names_by_id
                .get(&writer_id)
                .map(String::as_str),
            Some("test::io::Writer")
        );
        assert_eq!(
            decls
                .resolver
                .item_names_by_id
                .get(&io_module_def_id)
                .map(String::as_str),
            Some("test::io")
        );
    }

    #[test]
    fn collect_resolves_local_import_aliases_to_canonical_def_ids() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_collect_canonical_import_aliases_{}_{}",
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
        std::fs::write(
            &io_path,
            "writer_name: I64\nwriter_name = -> 0\n< writer_name\n",
        )
        .unwrap();

        let (program, graph) =
            load_source_program(root_path, "mod io\n> io::writer_name\n", Some("test"));

        let decls =
            collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
                .expect("collect should resolve canonical import aliases");

        let expected = *decls
            .resolver
            .item_paths
            .get("test::io::writer_name")
            .expect("canonical item path should resolve");
        assert_eq!(
            decls.resolver.import_aliases.get("writer_name"),
            Some(&expected)
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn collect_skips_non_explicit_glob_import_aliases_in_canonical_import_table() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_collect_non_explicit_glob_aliases_{}_{}",
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
        std::fs::write(
            &io_path,
            "writer_name: I64\nwriter_name = -> 0\n< writer_name\n",
        )
        .unwrap();

        let (program, graph) = load_source_program(root_path, "mod io\n> io::*\n", Some("test"));

        let decls =
            collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
                .expect(
                    "collect should preserve legacy glob aliases but skip canonical alias entries",
                );

        assert!(!decls.resolver.import_aliases.contains_key("writer_name"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn collect_resolves_explicit_type_import_aliases_to_canonical_def_ids() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_collect_canonical_type_import_aliases_{}_{}",
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
        std::fs::write(&io_path, "struct Writer\n").unwrap();

        let (program, graph) =
            load_source_program(root_path, "mod io\n> io::Writer\n", Some("test"));

        let decls =
            collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
                .expect("collect should resolve canonical type import aliases");

        let expected = *decls
            .resolver
            .item_paths
            .get("test::io::Writer")
            .expect("canonical item path should resolve");
        assert!(decls.items.structs().contains_key(&expected));
        assert_eq!(decls.items.structs().len(), 1);
        assert_eq!(decls.resolver.import_aliases.get("Writer"), Some(&expected));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn collect_resolves_export_function_aliases_to_canonical_def_ids() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_collect_canonical_export_aliases_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let root_path = temp_dir.join("main.rk");
        let math_path = temp_dir.join("math.rk");
        let io_path = temp_dir.join("io.rk");
        std::fs::write(&math_path, "mod io\n< io::writer_name\n").unwrap();
        std::fs::write(&io_path, "writer_name: I64\nwriter_name = -> 0\n").unwrap();

        let (program, graph) = load_source_program(root_path, "mod math\n", Some("test"));

        let decls =
            collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
                .expect("collect should resolve canonical export aliases");

        assert!(decls
            .resolver
            .item_paths
            .contains_key("test::math::io::writer_name"));
        assert!(!decls
            .resolver
            .item_paths
            .contains_key("test::math::writer_name"));

        let expected = *decls
            .resolver
            .item_paths
            .get("test::math::io::writer_name")
            .expect("canonical item path should resolve");
        assert_eq!(
            decls.resolver.export_aliases.get("test::math::writer_name"),
            Some(&expected)
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn collect_resolves_export_type_aliases_to_canonical_def_ids() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_collect_canonical_export_type_aliases_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let root_path = temp_dir.join("main.rk");
        let math_path = temp_dir.join("math.rk");
        let io_path = temp_dir.join("io.rk");
        std::fs::write(&math_path, "mod io\n< io::Writer\n").unwrap();
        std::fs::write(&io_path, "struct Writer\n").unwrap();

        let (program, graph) = load_source_program(root_path, "mod math\n", Some("test"));

        let decls =
            collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
                .expect("collect should resolve canonical export type aliases");

        assert!(struct_named(&decls, "test::math::Writer").is_none());
        assert!(decls
            .resolver
            .item_paths
            .contains_key("test::math::io::Writer"));
        assert!(!decls.resolver.item_paths.contains_key("test::math::Writer"));

        let expected = *decls
            .resolver
            .item_paths
            .get("test::math::io::Writer")
            .expect("canonical item path should resolve");
        assert_eq!(
            decls.resolver.export_aliases.get("test::math::Writer"),
            Some(&expected)
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn lowered_hir_indexes_import_and_export_aliases_by_canonical_def_id() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_lower_canonical_alias_indexes_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let root_path = temp_dir.join("main.rk");
        let math_path = temp_dir.join("math.rk");
        let io_path = temp_dir.join("io.rk");
        std::fs::write(&math_path, "mod io\n< io::writer_name\n< io::Writer\n").unwrap();
        std::fs::write(
            &io_path,
            "writer_name: I64\nwriter_name = -> 0\nstruct Writer\n",
        )
        .unwrap();

        let (program, graph) = load_source_program(
            root_path,
            "mod math\n> math::writer_name\n> math::Writer\n",
            Some("test"),
        );

        let decls =
            collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
                .expect("collect should resolve import and export aliases");

        let writer_name_id = *decls
            .resolver
            .item_paths
            .get("test::math::io::writer_name")
            .expect("canonical exported function should resolve");
        let writer_id = *decls
            .resolver
            .item_paths
            .get("test::math::io::Writer")
            .expect("canonical exported struct should resolve");
        assert_eq!(
            decls.resolver.export_aliases.get("test::math::writer_name"),
            Some(&writer_name_id)
        );
        assert_eq!(
            decls.resolver.export_aliases.get("test::math::Writer"),
            Some(&writer_id)
        );
        assert_eq!(
            decls.resolver.import_aliases.get("writer_name"),
            Some(&writer_name_id)
        );
        assert_eq!(
            decls.resolver.import_aliases.get("Writer"),
            Some(&writer_id)
        );

        let partial = crate::lower::program::lower_from_declarations(
            &program,
            decls,
            &CrateContext::new(),
            Some("test"),
        )
        .expect("lowering should preserve canonical alias DefIds");
        let resolved = crate::infer::finalize(partial)
            .expect("inference should finalize canonical alias indexes");

        assert_eq!(
            resolved
                .program
                .indexes
                .functions_by_id
                .get(&writer_name_id),
            Some(&"test::math::io::writer_name".to_string())
        );
        assert_eq!(
            resolved.program.indexes.structs_by_id.get(&writer_id),
            Some(&"test::math::io::Writer".to_string())
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn lowered_hir_indexes_trait_defaults_and_impl_methods_by_def_id() {
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![
                    TopLevel::StructDecl(StructDecl {
                        name: type_inner("Box"),
                        generic_params: vec![],
                        fields: vec![],
                        exported: false,
                    }),
                    TopLevel::TraitDecl(crate::ast::TraitDecl {
                        where_clauses: Vec::new(),
                        name: type_inner("Show"),
                        generic_params: vec![],
                        for_: None,
                        associated_types: vec![],
                        methods: HashMap::from([(ident("show"), function_decl("show"))]),
                        signatures: HashMap::new(),
                        exported: false,
                        language_items: Default::default(),
                    }),
                    TopLevel::Impl(trait_impl("Box", "Show", "show")),
                ],
                is_inline: false,
                filepath: None,
            },
        };

        let decls = collect(&program, &CrateContext::new(), false, Some("test"))
            .expect("collect should assign method DefIds");
        let trait_default_id = trait_named(&decls, "Show")
            .and_then(|trait_def| trait_def.methods.get("show"))
            .map(|method| method.id)
            .expect("trait default method should have a DefId");
        let trait_id = trait_named(&decls, "Show")
            .map(|trait_def| trait_def.id)
            .expect("trait should have a DefId");
        let impl_id = decls
            .items
            .impls()
            .values()
            .next()
            .map(|imp| imp.id)
            .expect("impl should have a DefId");
        let impl_method_id = decls
            .items
            .impls()
            .values()
            .next()
            .and_then(|imp| imp.methods.get("show"))
            .map(|method| method.id)
            .expect("impl method should have a DefId");

        let partial = crate::lower::program::lower_from_declarations(
            &program,
            decls,
            &CrateContext::new(),
            Some("test"),
        )
        .expect("lowering should preserve method DefIds");
        let resolved = crate::infer::finalize(partial).expect("inference should finalize methods");

        assert_eq!(
            resolved
                .program
                .indexes
                .methods_by_id
                .get(&trait_default_id),
            Some(&HirMethodLocation::TraitDefault {
                trait_id,
                method_id: trait_default_id,
                method_name: "show".to_string(),
            })
        );
        assert_eq!(
            resolved.program.indexes.methods_by_id.get(&impl_method_id),
            Some(&HirMethodLocation::ImplMethod {
                impl_id,
                method_id: impl_method_id,
                method_name: "show".to_string(),
            })
        );
    }

    #[test]
    fn lower_from_declarations_rejects_source_backed_external_dependency_consumption() {
        let program = program_with_struct("Point");
        let decls = collect(&program, &CrateContext::new(), false, Some("test"))
            .expect("collection without dependencies should succeed");

        let mut crate_ctx = CrateContext::new();
        crate_ctx.register_crate(
            crate::crate_system::CrateManifest {
                crate_: crate::crate_system::CrateConfig {
                    name: "dep".to_string(),
                    version: "0.1.0".to_string(),
                    no_std: false,
                },
                lib: crate::crate_system::LibConfig {
                    path: "lib.rk".to_string(),
                },
                dependencies: None,
            },
            PathBuf::from("/dep"),
            Module {
                name: None,
                top_levels: vec![TopLevel::FunctionDecl(function_decl("identity"))],
                is_inline: false,
                filepath: None,
            },
        );

        let errors = match crate::lower::program::lower_from_declarations(
            &program,
            decls,
            &crate_ctx,
            Some("test"),
        ) {
            Ok(_) => panic!("source-backed external dependency lowering should fail"),
            Err(errors) => errors,
        };

        let source_backed_rejections = errors
            .iter()
            .filter(|error| {
                error.message.contains(
                    "source-backed external dependency 'dep' is not supported during lowering",
                )
            })
            .count();
        assert_eq!(source_backed_rejections, 1);
    }

    #[test]
    fn collect_preserves_import_aliases_infix_precedence_and_function_signatures() {
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![
                    TopLevel::FunctionDecl(crate::ast::FunctionDecl {
                        name: ident("sum"),
                        lambda: crate::ast::LambdaDecl {
                            parameters: vec![],
                            body: crate::ast::Block { statements: vec![] },
                            arrow_kind: crate::ast::LambdaArrowKind::Normal,
                            span: crate::lexer::Span::test(),
                        },
                        self_receiver: None,
                        is_unsafe: false,
                        exported: false,
                    }),
                    TopLevel::Import(crate::ast::Path::Ident(crate::ast::IdentifierPath {
                        path: vec![crate::ast::IdentOrType::Ident(ident("sum"))],
                    })),
                    TopLevel::InfixOperator(7, "++".to_string()),
                    TopLevel::FunctionSig(crate::ast::FunctionSig {
                        name: ident("declared_only"),
                        sig: crate::ast::ParseType::Unit(crate::lexer::Span::test()),
                        where_clauses: vec![],
                        self_receiver: None,
                        is_unsafe: false,
                        exported: false,
                    }),
                ],
                is_inline: false,
                filepath: None,
            },
        };

        let decls = collect(&program, &CrateContext::new(), false, Some("test"))
            .expect("collect should preserve declaration-side bookkeeping");

        let sum_id = function_named(&decls, "sum").unwrap().id;
        assert_eq!(decls.resolver.import_aliases.get("sum"), Some(&sum_id));
        assert_eq!(decls.infix_precedence.get("++"), Some(&7));
        assert!(function_sig_named(&decls, "declared_only").is_some());
    }

    #[test]
    fn collect_assigns_canonical_id_to_signature_backed_function_header() {
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![
                    TopLevel::FunctionSig(FunctionSig {
                        name: ident("identity"),
                        sig: ParseType::Function(vec![
                            ParseType::Type(type_inner("T")),
                            ParseType::Type(type_inner("T")),
                        ]),
                        where_clauses: vec![],
                        self_receiver: None,
                        is_unsafe: false,
                        exported: false,
                    }),
                    TopLevel::FunctionDecl(FunctionDecl {
                        name: ident("identity"),
                        lambda: LambdaDecl {
                            parameters: vec![Pattern {
                                binding: None,
                                kind: PatternKind::Ident(crate::ast::IdentPattern {
                                    name: ident("value"),
                                    mut_: false,
                                }),
                            }],
                            body: crate::ast::Block { statements: vec![] },
                            arrow_kind: LambdaArrowKind::Normal,
                            span: crate::lexer::Span::test(),
                        },
                        self_receiver: None,
                        is_unsafe: false,
                        exported: false,
                    }),
                ],
                is_inline: false,
                filepath: None,
            },
        };

        let decls = collect(&program, &CrateContext::new(), false, Some("test"))
            .expect("collect should assign canonical IDs");
        let function =
            function_named(&decls, "identity").expect("identity function should be collected");

        assert_ne!(function.id, DefId::new(CrateId(0), LocalDefId(0)));
        assert!(decls.current_def_ids.contains(&function.id));
        assert!(
            function
                .generic_params
                .iter()
                .map(|generic| generic.id)
                .all(|generic_id| generic_id.owner == function.id),
            "signature-backed generic owners should use the canonical function ID"
        );
    }

    #[test]
    fn collect_preserves_source_backed_module_function_qualification_and_import_aliases() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_collect_source_backed_function_alias_{}_{}",
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
        std::fs::write(
            &io_path,
            "writer_name: I64\nwriter_name = -> 0\n< writer_name\n",
        )
        .unwrap();

        let (program, graph) =
            load_source_program(root_path, "mod io\n> io::writer_name\n", Some("test"));

        let decls =
            collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
                .expect("collect should preserve source-backed module function aliases");

        assert!(function_named(&decls, "test::io::writer_name").is_some());
        let writer_id = function_named(&decls, "test::io::writer_name").unwrap().id;
        assert_eq!(
            decls.resolver.import_aliases.get("writer_name"),
            Some(&writer_id)
        );
        assert!(function_named(&decls, "io::writer_name").is_none());
        assert!(decls
            .loaded_module_paths
            .iter()
            .any(|(name, path)| name == "test::io" && path == &io_path));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn collect_resolves_source_backed_forward_sibling_trait_imports_in_impls() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_collect_source_backed_forward_trait_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let root_path = temp_dir.join("main.rk");
        std::fs::write(
            temp_dir.join("early.rk"),
            "> demo::traits::Marker\n< struct Thing\nimpl Marker for Thing\n    @mark = -> 0\n",
        )
        .unwrap();
        std::fs::write(
            temp_dir.join("traits.rk"),
            "< trait Marker\n    @mark: I64\n",
        )
        .unwrap();

        let (program, graph) = load_source_program(
            root_path,
            "< mod early\n< mod traits\n< early::Thing\n",
            Some("demo"),
        );

        let decls =
            collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("demo"))
                .expect(
                    "collection should resolve later sibling trait imports before impl headers",
                );
        assert!(trait_named(&decls, "demo::traits::Marker").is_some());
        let trait_id = trait_named(&decls, "demo::traits::Marker").unwrap().id;
        let marker_impl = decls
            .items
            .impls()
            .values()
            .find(|imp| imp.type_name == "Thing" && imp.trait_name.as_deref() == Some("Marker"))
            .expect("Marker impl for Thing should be collected");

        assert_eq!(marker_impl.trait_id, Some(trait_id));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn collect_preserves_source_backed_parent_export_references_for_nested_modules() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_collect_source_backed_parent_export_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(temp_dir.join("b")).unwrap();

        let root_path = temp_dir.join("main.rk");
        std::fs::write(temp_dir.join("b").join("mod.rk"), "mod a\n").unwrap();
        std::fs::write(temp_dir.join("b").join("a.rk"), "< struct Thing\n").unwrap();

        let (program, graph) =
            load_source_program(root_path, "mod b\n< b::a::Thing\n", Some("test"));

        let decls =
            collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
                .expect("collect should preserve source-backed parent export references");

        assert!(struct_named(&decls, "test::b::a::Thing").is_some());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn collect_keeps_nested_source_backed_imports_out_of_global_aliases() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_collect_nested_source_backed_context_{}_{}",
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
        std::fs::write(
            &io_path,
            "writer_name: I64\nwriter_name = -> 0\n< writer_name\n",
        )
        .unwrap();

        let (_, graph) = load_source_program(root_path.clone(), "mod io\n", Some("math"));
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![inline_module(
                    "math",
                    vec![
                        TopLevel::Mod(ident("io"), false),
                        TopLevel::Import(crate::ast::Path::Ident(crate::ast::IdentifierPath {
                            path: vec![
                                crate::ast::IdentOrType::Ident(ident("io")),
                                crate::ast::IdentOrType::Ident(ident("writer_name")),
                            ],
                        })),
                    ],
                )],
                is_inline: false,
                filepath: Some(root_path),
            },
        };

        let decls =
            collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
                .expect("collect should preserve nested source-backed module context");

        assert!(function_named(&decls, "math::io::writer_name").is_some());
        assert!(function_named(&decls, "test::io::writer_name").is_none());
        assert!(function_named(&decls, "test::math::io::writer_name").is_none());
        assert!(!decls.resolver.import_aliases.contains_key("writer_name"));
        assert_eq!(
            decls
                .loaded_module_paths
                .iter()
                .filter(|(name, path)| name == "math::io" && path == &io_path)
                .count(),
            1
        );
        assert_eq!(
            decls
                .loaded_module_paths
                .iter()
                .filter(|(name, path)| name == "test::io" && path == &io_path)
                .count(),
            0
        );
        assert!(decls
            .loaded_module_paths
            .iter()
            .any(|(name, path)| name == "math::io" && path == &io_path));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn lower_from_declarations_resolves_nested_source_backed_imports_inside_inline_module_bodies() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_lower_nested_source_backed_context_{}_{}",
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
        std::fs::write(
            &io_path,
            "writer_name: I64\nwriter_name = -> 0\n< writer_name\n",
        )
        .unwrap();
        let (_, graph) = load_source_program(root_path.clone(), "mod io\n", Some("math"));

        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![inline_module(
                    "math",
                    vec![
                        TopLevel::Mod(ident("io"), false),
                        TopLevel::Import(crate::ast::Path::Ident(crate::ast::IdentifierPath {
                            path: vec![
                                crate::ast::IdentOrType::Ident(ident("io")),
                                crate::ast::IdentOrType::Ident(ident("writer_name")),
                            ],
                        })),
                        TopLevel::FunctionDecl(function_decl_with_body_expr(
                            "use_writer",
                            ident_expr("writer_name"),
                        )),
                    ],
                )],
                is_inline: false,
                filepath: Some(root_path),
            },
        };

        let decls =
            collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
                .expect("collect should preserve nested source-backed module context");
        assert!(!decls.resolver.import_aliases.contains_key("writer_name"));
        let writer_id = function_named(&decls, "math::io::writer_name").unwrap().id;
        let use_writer_id = function_named(&decls, "math::use_writer").unwrap().id;

        let partial = crate::lower::program::lower_from_declarations(
            &program,
            decls,
            &CrateContext::new(),
            Some("test"),
        )
        .expect("lowering should resolve nested source-backed imports module-locally");

        let function = partial
            .functions
            .get(&use_writer_id)
            .expect("inline module body should lower under its qualified name");

        match function.body.stmts.as_slice() {
            [crate::hir::HirStmt::Expr(expr)] => match &expr.kind {
                crate::hir::HirExprKind::ResolvedVar(reference) => {
                    assert_eq!(reference.name, "math::io::writer_name");
                    assert_eq!(
                        reference.target,
                        crate::hir::HirVarTarget::Function(writer_id)
                    );
                }
                other => panic!("expected lowered import to be a resolved var, got {other:?}"),
            },
            other => panic!("expected single expression body, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn lower_from_declarations_resolves_nested_imported_types_inside_inline_module_bodies() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_lower_nested_source_backed_type_context_{}_{}",
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
        std::fs::write(&io_path, "< struct Writer\n").unwrap();
        let (_, graph) = load_source_program(root_path.clone(), "mod io\n", Some("math"));

        let writer_instance = crate::ast::Expression::UnaryExpr(
            crate::ast::UnaryExpr::PrimaryExpr(crate::ast::PrimaryExpr {
                operand: crate::ast::Operand::Instance(crate::ast::Instance {
                    name: crate::ast::TypePath {
                        path: vec![crate::ast::IdentOrType::Type(crate::ast::ParseType::Type(
                            type_inner("Writer"),
                        ))],
                    },
                    fields: HashMap::new(),
                }),
                secondaries: None,
                type_annotation: None,
            }),
        );

        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![inline_module(
                    "math",
                    vec![
                        TopLevel::Mod(ident("io"), false),
                        TopLevel::Import(crate::ast::Path::Type(crate::ast::TypePath {
                            path: vec![
                                crate::ast::IdentOrType::Ident(ident("io")),
                                crate::ast::IdentOrType::Type(crate::ast::ParseType::Type(
                                    type_inner("Writer"),
                                )),
                            ],
                        })),
                        TopLevel::FunctionDecl(function_decl_with_body_expr(
                            "make_writer",
                            writer_instance,
                        )),
                    ],
                )],
                is_inline: false,
                filepath: Some(root_path),
            },
        };

        let decls =
            collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
                .expect("collect should preserve nested source-backed module type context");

        let make_writer_id = function_named(&decls, "math::make_writer").unwrap().id;
        let writer_id = struct_named(&decls, "math::io::Writer").unwrap().id;
        let partial = crate::lower::program::lower_from_declarations(
            &program,
            decls,
            &CrateContext::new(),
            Some("test"),
        )
        .expect("lowering should resolve nested imported types module-locally");

        let function = partial
            .functions
            .get(&make_writer_id)
            .expect("inline module body should lower under its qualified name");

        assert_eq!(
            function.ret_type,
            crate::types::Type::Struct {
                id: writer_id,
                args: vec![],
            }
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn collect_keeps_root_qualified_local_imports_stable_inside_nested_modules() {
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![
                    inline_module(
                        "math",
                        vec![TopLevel::StructDecl(StructDecl {
                            name: type_inner("Vector"),
                            generic_params: vec![],
                            fields: vec![],
                            exported: false,
                        })],
                    ),
                    inline_module(
                        "inner",
                        vec![TopLevel::Import(crate::ast::Path::Type(
                            crate::ast::TypePath {
                                path: vec![
                                    crate::ast::IdentOrType::Ident(ident("math")),
                                    crate::ast::IdentOrType::Type(crate::ast::ParseType::Type(
                                        type_inner("Vector"),
                                    )),
                                ],
                            },
                        ))],
                    ),
                ],
                is_inline: false,
                filepath: None,
            },
        };

        let decls = collect(&program, &CrateContext::new(), false, Some("test"))
            .expect("collect should preserve root-qualified local imports in nested modules");

        assert!(struct_named(&decls, "Vector").is_some());
        assert!(struct_named(&decls, "math::Vector").is_some());
        assert!(struct_named(&decls, "inner::math::Vector").is_none());
    }

    #[test]
    fn collect_keeps_inline_qualified_aliases_in_local_namespace() {
        let program = Program {
            module: Module {
                name: None,
                top_levels: vec![inline_module(
                    "math",
                    vec![
                        TopLevel::StructDecl(StructDecl {
                            name: type_inner("Vector"),
                            generic_params: vec![],
                            fields: vec![],
                            exported: false,
                        }),
                        TopLevel::FunctionDecl(crate::ast::FunctionDecl {
                            name: ident("sum"),
                            lambda: crate::ast::LambdaDecl {
                                parameters: vec![],
                                body: crate::ast::Block { statements: vec![] },
                                arrow_kind: crate::ast::LambdaArrowKind::Normal,
                                span: crate::lexer::Span::test(),
                            },
                            self_receiver: None,
                            is_unsafe: false,
                            exported: false,
                        }),
                    ],
                )],
                is_inline: false,
                filepath: None,
            },
        };

        let decls = collect(&program, &CrateContext::new(), false, Some("test"))
            .expect("collect should preserve inline qualified aliases");

        assert!(struct_named(&decls, "Vector").is_some());
        assert!(struct_named(&decls, "math::Vector").is_some());
        assert!(struct_named(&decls, "test::math::Vector").is_none());
        assert!(function_named(&decls, "sum").is_some());
        assert!(function_named(&decls, "math::sum").is_some());
        assert!(function_named(&decls, "test::math::sum").is_none());
    }

    fn parse_language_item_source(source: &str) -> Program {
        crate::parser::parse_string(source, &crate::Config::default())
            .expect("language item source should parse")
    }

    #[test]
    fn collection_rejects_supertrait_cycles_with_user_facing_path() {
        let program = crate::parser::parse_string(
            "trait First for F _ where F: Second\n\ntrait Second for F _ where F: First\n",
            &crate::Config::default(),
        )
        .expect("cyclic supertraits should parse before collection validation");

        let errors = match collect(&program, &CrateContext::new(), false, Some("test")) {
            Ok(_) => panic!("supertrait cycle should be rejected"),
            Err(errors) => errors,
        };
        let message = errors
            .iter()
            .map(|error| error.message.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            message.contains("supertrait cycle detected"),
            "unexpected diagnostics: {message}"
        );
        assert!(message.contains("First"));
        assert!(message.contains("Second"));
        assert!(!message.contains("trait#"));
    }

    #[test]
    fn collection_rejects_missing_supertrait_by_source_name() {
        let program = crate::parser::parse_string(
            "trait Child for F _ where F: Missing\n",
            &crate::Config::default(),
        )
        .expect("missing supertrait should parse before collection validation");

        let errors = match collect(&program, &CrateContext::new(), false, Some("test")) {
            Ok(_) => panic!("missing supertrait should be rejected"),
            Err(errors) => errors,
        };

        assert!(errors.iter().any(|error| error
            .message
            .contains("unknown trait 'Missing' in where clause")));
    }

    #[test]
    fn typed_predicate_constructor_declaration_sets_generic_kind_and_obligation() {
        let program = crate::parser::parse_string(
            "trait Functor for F _\n\napply_f: F A -> F A where F _: Functor\n",
            &crate::Config::default(),
        )
        .expect("constructor predicate source should parse");

        let declarations = collect(&program, &CrateContext::new(), false, Some("test"))
            .expect("constructor predicate should collect");
        let signature = function_sig_named(&declarations, "apply_f").unwrap();
        let functor = trait_named(&declarations, "Functor").unwrap();
        let constructor = signature
            .generic_params
            .iter()
            .find(|param| param.name == "F")
            .expect("F generic descriptor");

        assert_eq!(
            constructor.kind,
            crate::type_services::kind::Kind::arrow(
                crate::type_services::kind::Kind::Type,
                crate::type_services::kind::Kind::Type,
            )
        );
        assert_eq!(
            signature.generic_bounds.predicates,
            vec![crate::types::Predicate::Trait {
                subject: Type::Generic(constructor.id),
                trait_id: functor.id,
                args: Vec::new(),
            }]
        );
    }

    fn collect_language_item_program(program: &Program) -> Result<Declarations, Vec<ResolveError>> {
        collect(&program, &CrateContext::new(), false, Some("test"))
    }

    fn collect_language_item_source(source: &str) -> Result<Declarations, Vec<ResolveError>> {
        collect_language_item_program(&parse_language_item_source(source))
    }

    fn language_item_provider_context(
        providers: impl IntoIterator<Item = (&'static str, LanguageItems<DefId>)>,
    ) -> CrateContext {
        let mut crate_ctx = CrateContext::new();
        for (index, (name, language_items)) in providers.into_iter().enumerate() {
            crate_ctx
                .add_extern_crate(crate::crate_system::ExternCrateRecord::new(
                    CrateId((index + 1) as u32),
                    name.to_string(),
                    crate::crate_system::ExternCrateMetadata::empty_for_test()
                        .with_language_items(language_items),
                    crate::crate_system::ExternCrateBodies::default(),
                    crate::crate_system::ExternCrateLink::metadata_only(BTreeMap::new()),
                ))
                .expect("test language item provider should register");
        }
        crate_ctx
    }

    fn marker_span_bounds(
        program: &Program,
        top_level_index: usize,
        member_index: Option<usize>,
    ) -> (usize, usize) {
        let markers = match &program.module.top_levels[top_level_index] {
            TopLevel::TraitDecl(trait_decl) => &trait_decl.language_items,
            TopLevel::EnumDecl(enum_decl) => &enum_decl.language_items,
            _ => panic!("expected marked trait or enum"),
        };
        let span = match member_index {
            Some(member_index) => &markers.members[member_index].marker.span,
            None => &markers.root.as_ref().expect("expected root marker").span,
        };
        (span.start, span.end)
    }

    fn error_details(errors: Vec<ResolveError>) -> Vec<(String, Option<(usize, usize)>)> {
        errors
            .into_iter()
            .map(|error| {
                let span = error.span().map(|span| (span.start, span.end));
                (error.message, span)
            })
            .collect()
    }

    #[test]
    fn collect_binds_renamed_sized_trait_to_its_indexed_id() {
        let decls = collect_language_item_source("lang sized\n< trait StaticLayout\n")
            .expect("renamed sized root should collect");
        let indexed_id = decls
            .item_index
            .item_at_source(decls.indexing_ids.root_module_id(), 0)
            .expect("sized trait should be indexed")
            .def_id;

        assert_eq!(decls.language_items.sized.unwrap().trait_id, indexed_id);
    }

    #[test]
    fn collect_binds_renamed_index_mut_bundle_by_id() {
        let program = parse_language_item_source(
            r#"lang index
< trait ReadAt Key
    lang output
    type ReadValue
    lang method
    @read_at: Key -> &Self::ReadValue

lang index_mut
< trait WriteAt Key
    lang output
    type WriteValue
    lang method
    ^@write_at: Key -> &mut Self::WriteValue
"#,
        );
        let decls = collect(&program, &CrateContext::new(), false, Some("core"))
            .expect("renamed Index and IndexMut roots should collect");
        let index_mut = decls
            .language_items
            .index_mut
            .expect("IndexMut bundle should be bound");

        assert_eq!(
            decls.resolver.item_paths.get("core::WriteAt"),
            Some(&index_mut.trait_id),
        );
        assert_ne!(index_mut.method_id, index_mut.trait_id);
    }

    #[test]
    fn collect_language_items_merges_stdlib_metadata_without_prelude_injection() {
        let sized_id = DefId::new(CrateId(7), LocalDefId(11));
        let crate_ctx = language_item_provider_context([(
            "stdlib",
            LanguageItems {
                sized: Some(SizedLanguageItems { trait_id: sized_id }),
                ..LanguageItems::default()
            },
        )]);

        let decls = collect(
            &parse_language_item_source("struct App\n"),
            &crate_ctx,
            false,
            Some("app"),
        )
        .expect("explicit stdlib metadata should merge without prelude injection");

        assert_eq!(
            decls.language_items.sized.map(|items| items.trait_id),
            Some(sized_id),
        );
    }

    #[test]
    fn collect_artifact_declarations_merges_explicit_dependency_language_items() {
        let sized_id = DefId::new(CrateId(7), LocalDefId(11));
        let mut crate_ctx = CrateContext::new();
        crate_ctx.register_crate(
            crate::crate_system::CrateManifest {
                crate_: crate::crate_system::CrateConfig {
                    name: "app".to_string(),
                    version: "0.1.0".to_string(),
                    no_std: false,
                },
                lib: crate::crate_system::LibConfig {
                    path: "lib.rk".to_string(),
                },
                dependencies: None,
            },
            PathBuf::from("/app"),
            parse_language_item_source("struct App\n").module,
        );
        crate_ctx
            .add_extern_crate(crate::crate_system::ExternCrateRecord::new(
                CrateId(7),
                "tools".to_string(),
                crate::crate_system::ExternCrateMetadata::empty_for_test().with_language_items(
                    LanguageItems {
                        sized: Some(SizedLanguageItems { trait_id: sized_id }),
                        ..LanguageItems::default()
                    },
                ),
                crate::crate_system::ExternCrateBodies::default(),
                crate::crate_system::ExternCrateLink::metadata_only(BTreeMap::new()),
            ))
            .expect("test language item provider should register");

        let decls = collect_artifact_declarations(&crate_ctx, false, "app")
            .expect("artifact collection should merge explicit metadata")
            .declarations;

        assert_eq!(
            decls.language_items.sized.map(|items| items.trait_id),
            Some(sized_id),
        );
    }

    fn artifact_context_with_same_name_language_item_record(
        source: &str,
        language_items: LanguageItems<DefId>,
    ) -> CrateContext {
        let mut crate_ctx = CrateContext::new();
        crate_ctx.register_crate(
            crate::crate_system::CrateManifest {
                crate_: crate::crate_system::CrateConfig {
                    name: "app".to_string(),
                    version: "0.1.0".to_string(),
                    no_std: false,
                },
                lib: crate::crate_system::LibConfig {
                    path: "lib.rk".to_string(),
                },
                dependencies: None,
            },
            PathBuf::from("/app"),
            parse_language_item_source(source).module,
        );
        crate_ctx
            .add_extern_crate(crate::crate_system::ExternCrateRecord::new(
                CrateId(7),
                "app".to_string(),
                crate::crate_system::ExternCrateMetadata::empty_for_test()
                    .with_language_items(language_items),
                crate::crate_system::ExternCrateBodies::default(),
                crate::crate_system::ExternCrateLink::metadata_only(BTreeMap::new()),
            ))
            .expect("test self artifact record should register");
        crate_ctx
    }

    #[test]
    fn collect_artifact_declarations_excludes_same_name_artifact_language_items() {
        let crate_ctx = artifact_context_with_same_name_language_item_record(
            "lang sized\n< trait SourceSized\n",
            LanguageItems {
                sized: Some(SizedLanguageItems {
                    trait_id: DefId::new(CrateId(7), LocalDefId(11)),
                }),
                ..LanguageItems::default()
            },
        );

        let decls = collect_artifact_declarations(&crate_ctx, false, "app")
            .expect("current source provider should not conflict with its artifact record")
            .declarations;
        let current_sized_id = decls.item_index.defs_named("SourceSized")[0];

        assert_eq!(
            decls.language_items.sized.map(|items| items.trait_id),
            Some(current_sized_id),
        );
    }

    #[test]
    fn collect_artifact_declarations_does_not_use_same_name_artifact_language_items() {
        let crate_ctx = artifact_context_with_same_name_language_item_record(
            "struct App\n",
            LanguageItems {
                sized: Some(SizedLanguageItems {
                    trait_id: DefId::new(CrateId(7), LocalDefId(11)),
                }),
                ..LanguageItems::default()
            },
        );

        let decls = collect_artifact_declarations(&crate_ctx, false, "app")
            .expect("same-name artifact metadata should not become a dependency provider")
            .declarations;

        assert_eq!(decls.language_items.sized, None);
    }

    #[test]
    fn collect_excludes_same_name_extern_from_declarations_and_language_items() {
        let artifact_function_id = DefId::new(CrateId(7), LocalDefId(10));
        let mut interface = ArtifactCrateInterface::default();
        interface.insert_function(
            "app::from_self_artifact".to_string(),
            test_function(artifact_function_id, "from_self_artifact"),
        );
        let program = parse_language_item_source("lang sized\n< trait SourceSized\n");
        let mut crate_ctx = CrateContext::new();
        crate_ctx.register_crate(
            crate::crate_system::CrateManifest {
                crate_: crate::crate_system::CrateConfig {
                    name: "app".to_string(),
                    version: "0.1.0".to_string(),
                    no_std: false,
                },
                lib: crate::crate_system::LibConfig {
                    path: "lib.rk".to_string(),
                },
                dependencies: None,
            },
            PathBuf::from("/app"),
            program.module.clone(),
        );
        crate_ctx
            .add_extern_crate(crate::crate_system::ExternCrateRecord::new(
                CrateId(7),
                "app".to_string(),
                crate::crate_system::ExternCrateMetadata::new(
                    interface,
                    crate::collect::resolver::ResolverTables::default(),
                    BTreeMap::new(),
                )
                .with_language_items(LanguageItems {
                    sized: Some(SizedLanguageItems {
                        trait_id: DefId::new(CrateId(7), LocalDefId(11)),
                    }),
                    ..LanguageItems::default()
                }),
                crate::crate_system::ExternCrateBodies::default(),
                crate::crate_system::ExternCrateLink::metadata_only(BTreeMap::new()),
            ))
            .expect("test self artifact record should register");

        let decls = collect(&program, &crate_ctx, false, Some("app"))
            .expect("same-name extern should not participate in normal collection");
        let current_sized_id = decls.item_index.defs_named("SourceSized")[0];

        assert!(!decls.items.functions().contains_key(&artifact_function_id));
        assert!(!decls
            .resolver
            .item_names_by_id
            .contains_key(&artifact_function_id));
        assert_eq!(
            decls.language_items.sized.map(|items| items.trait_id),
            Some(current_sized_id),
        );
    }

    #[test]
    fn collect_language_items_keeps_same_name_extern_without_registered_source_crate() {
        let sized_id = DefId::new(CrateId(7), LocalDefId(11));
        let crate_ctx = language_item_provider_context([(
            "app",
            LanguageItems {
                sized: Some(SizedLanguageItems { trait_id: sized_id }),
                ..LanguageItems::default()
            },
        )]);

        let decls = collect(
            &parse_language_item_source("struct App\n"),
            &crate_ctx,
            false,
            Some("app"),
        )
        .expect("same-name explicit extern should remain a dependency without a source crate");

        assert_eq!(
            decls.language_items.sized.map(|items| items.trait_id),
            Some(sized_id),
        );
    }

    #[test]
    fn collect_language_items_conflicts_with_same_name_extern_without_registered_source_crate() {
        let crate_ctx = language_item_provider_context([(
            "app",
            LanguageItems {
                sized: Some(SizedLanguageItems {
                    trait_id: DefId::new(CrateId(7), LocalDefId(11)),
                }),
                ..LanguageItems::default()
            },
        )]);

        let Err(errors) = collect(
            &parse_language_item_source("lang sized\n< trait SourceSized\n"),
            &crate_ctx,
            false,
            Some("app"),
        ) else {
            panic!("same-name explicit extern should conflict with a current provider");
        };

        assert_eq!(
            errors
                .into_iter()
                .map(|error| error.message)
                .collect::<Vec<_>>(),
            vec!["multiple language-item providers claim protocol 'sized': app, app"],
        );
    }

    #[test]
    fn collect_language_items_rejects_current_and_dependency_sized_providers() {
        let crate_ctx = language_item_provider_context([(
            "dep",
            LanguageItems {
                sized: Some(SizedLanguageItems {
                    trait_id: DefId::new(CrateId(7), LocalDefId(11)),
                }),
                ..LanguageItems::default()
            },
        )]);

        let Err(errors) = collect(
            &parse_language_item_source("lang sized\n< trait Current\n"),
            &crate_ctx,
            false,
            Some("app"),
        ) else {
            panic!("current and dependency sized providers should conflict");
        };

        assert_eq!(
            errors
                .into_iter()
                .map(|error| error.message)
                .collect::<Vec<_>>(),
            vec!["multiple language-item providers claim protocol 'sized': app, dep"],
        );
    }

    #[test]
    fn collect_language_items_rejects_dependency_sized_providers_in_lexical_order() {
        let crate_ctx = language_item_provider_context([
            (
                "zeta",
                LanguageItems {
                    sized: Some(SizedLanguageItems {
                        trait_id: DefId::new(CrateId(7), LocalDefId(11)),
                    }),
                    ..LanguageItems::default()
                },
            ),
            (
                "alpha",
                LanguageItems {
                    sized: Some(SizedLanguageItems {
                        trait_id: DefId::new(CrateId(8), LocalDefId(12)),
                    }),
                    ..LanguageItems::default()
                },
            ),
        ]);

        let Err(errors) = collect(
            &parse_language_item_source("struct App\n"),
            &crate_ctx,
            false,
            Some("app"),
        ) else {
            panic!("duplicate dependency sized providers should conflict");
        };

        assert_eq!(
            errors
                .into_iter()
                .map(|error| error.message)
                .collect::<Vec<_>>(),
            vec!["multiple language-item providers claim protocol 'sized': alpha, zeta"],
        );
    }

    #[test]
    fn collect_language_items_merges_disjoint_dependency_bundles() {
        let sized_id = DefId::new(CrateId(7), LocalDefId(11));
        let drop_trait_id = DefId::new(CrateId(8), LocalDefId(12));
        let drop_method_id = DefId::new(CrateId(8), LocalDefId(13));
        let crate_ctx = language_item_provider_context([
            (
                "sized_provider",
                LanguageItems {
                    sized: Some(SizedLanguageItems { trait_id: sized_id }),
                    ..LanguageItems::default()
                },
            ),
            (
                "drop_provider",
                LanguageItems {
                    drop: Some(DropLanguageItems {
                        trait_id: drop_trait_id,
                        method_id: drop_method_id,
                    }),
                    ..LanguageItems::default()
                },
            ),
        ]);

        let decls = collect(
            &parse_language_item_source("struct App\n"),
            &crate_ctx,
            false,
            Some("app"),
        )
        .expect("disjoint dependency bundles should merge");

        assert_eq!(
            decls.language_items.sized.map(|items| items.trait_id),
            Some(sized_id),
        );
        assert_eq!(
            decls.language_items.drop,
            Some(DropLanguageItems {
                trait_id: drop_trait_id,
                method_id: drop_method_id,
            }),
        );
    }

    #[test]
    fn collect_language_items_reports_duplicate_try_providers_once_per_protocol() {
        let try_items = |crate_id| TryLanguageItems {
            try_trait_id: DefId::new(CrateId(crate_id), LocalDefId(1)),
            output_id: AssocTypeId(0),
            residual_id: AssocTypeId(1),
            branch_method_id: DefId::new(CrateId(crate_id), LocalDefId(2)),
            from_residual_trait_id: DefId::new(CrateId(crate_id), LocalDefId(3)),
            from_residual_method_id: DefId::new(CrateId(crate_id), LocalDefId(4)),
            control_flow_enum_id: DefId::new(CrateId(crate_id), LocalDefId(5)),
            break_variant_id: VariantId(0),
            continue_variant_id: VariantId(1),
        };
        let crate_ctx = language_item_provider_context([
            (
                "zeta",
                LanguageItems {
                    try_protocol: Some(try_items(7)),
                    ..LanguageItems::default()
                },
            ),
            (
                "alpha",
                LanguageItems {
                    try_protocol: Some(try_items(8)),
                    ..LanguageItems::default()
                },
            ),
        ]);

        let Err(errors) = collect(
            &parse_language_item_source("struct App\n"),
            &crate_ctx,
            false,
            Some("app"),
        ) else {
            panic!("duplicate try providers should conflict");
        };

        assert_eq!(
            errors
                .into_iter()
                .map(|error| error.message)
                .collect::<Vec<_>>(),
            vec!["multiple language-item providers claim protocol 'try': alpha, zeta"],
        );
    }

    #[test]
    fn collect_reports_each_missing_try_protocol_component_deterministically() {
        let Err(errors) = collect_language_item_source("lang try\n< trait Carrier\n") else {
            panic!("incomplete try protocol should be rejected");
        };
        let messages = errors
            .into_iter()
            .map(|error| error.message)
            .collect::<Vec<_>>();

        assert_eq!(
            messages,
            vec![
                "language item try.output is missing".to_string(),
                "language item try.residual is missing".to_string(),
                "language item try.branch is missing".to_string(),
                "language item from_residual is missing".to_string(),
                "language item control_flow is missing".to_string(),
            ],
        );
    }

    #[test]
    fn collect_binds_complete_try_protocol_across_inline_and_source_modules() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_collect_language_items_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let root_path = temp_dir.join("main.rk");
        std::fs::write(
            temp_dir.join("flow.rk"),
            "lang control_flow\n< enum Flow\n    lang break\n    Stop\n    lang continue\n    Go\n",
        )
        .unwrap();
        let (mut program, graph) = load_source_program(
            root_path,
            "lang try\n< trait Carrier\n    lang output\n    type Value\n    lang residual\n    type Remainder\n    lang branch\n    split: I64\nmod flow\n",
            Some("test"),
        );
        let inline = crate::parser::parse_string(
            "lang from_residual\n< trait Rebuild\n    lang method\n    rebuild: I64\n",
            &crate::Config::default(),
        )
        .unwrap();
        program
            .module
            .top_levels
            .insert(1, inline_module("conversion", inline.module.top_levels));

        let decls =
            collect_with_source_graph(&program, &graph, &CrateContext::new(), false, Some("test"))
                .expect("complete try protocol should collect");
        let try_items = decls
            .language_items
            .try_protocol
            .expect("try protocol should bind");
        let carrier_id = decls.item_index.defs_named("Carrier")[0];
        let rebuild_id = decls.item_index.defs_named("Rebuild")[0];
        let flow_id = decls.item_index.defs_named("Flow")[0];

        assert_eq!(try_items.try_trait_id, carrier_id);
        assert_eq!(try_items.output_id, crate::ids::AssocTypeId(0));
        assert_eq!(try_items.residual_id, crate::ids::AssocTypeId(1));
        assert_eq!(
            try_items.branch_method_id,
            decls.items.traits().get(&carrier_id).unwrap().signatures["split"].id,
        );
        assert_eq!(try_items.from_residual_trait_id, rebuild_id);
        assert_eq!(
            try_items.from_residual_method_id,
            decls.items.traits().get(&rebuild_id).unwrap().signatures["rebuild"].id,
        );
        assert_eq!(try_items.control_flow_enum_id, flow_id);
        assert_eq!(try_items.break_variant_id, crate::ids::VariantId(0));
        assert_eq!(try_items.continue_variant_id, crate::ids::VariantId(1));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn collect_rejects_duplicate_language_item_root_role() {
        let program =
            parse_language_item_source("lang sized\n< trait First\nlang sized\n< trait Second\n");
        let second_root = marker_span_bounds(&program, 1, None);

        let Err(errors) = collect_language_item_program(&program) else {
            panic!("duplicate root role should be rejected");
        };

        assert_eq!(
            error_details(errors),
            vec![(
                "language item sized is duplicated".to_string(),
                Some(second_root),
            )],
        );
    }

    #[test]
    fn collect_rejects_duplicate_language_item_child_role() {
        let program = parse_language_item_source(
            "lang drop\n< trait Cleaner\n    lang method\n    first: I64\n    lang method\n    second: I64\n",
        );
        let second_method = marker_span_bounds(&program, 0, Some(1));

        let Err(errors) = collect_language_item_program(&program) else {
            panic!("duplicate child role should be rejected");
        };

        assert_eq!(
            error_details(errors),
            vec![(
                "language item drop.method is duplicated".to_string(),
                Some(second_method),
            )],
        );
    }

    #[test]
    fn collect_rejects_language_item_child_role_in_wrong_protocol_context() {
        let program = parse_language_item_source(
            "lang drop\n< trait Cleaner\n    lang residual\n    clean: I64\n",
        );
        let root = marker_span_bounds(&program, 0, None);
        let residual = marker_span_bounds(&program, 0, Some(0));

        let Err(errors) = collect_language_item_program(&program) else {
            panic!("wrong child role context should be rejected");
        };

        assert_eq!(
            error_details(errors),
            vec![
                (
                    "language item drop.method is missing".to_string(),
                    Some(root),
                ),
                (
                    "language item residual is not valid for drop".to_string(),
                    Some(residual),
                ),
            ],
        );
    }

    #[test]
    fn collect_rejects_language_item_child_with_wrong_declaration_kind() {
        let program = parse_language_item_source(
            "lang index\n< trait Lookup\n    lang output\n    output: I64\n    lang method\n    index: I64\n",
        );
        let root = marker_span_bounds(&program, 0, None);
        let output = marker_span_bounds(&program, 0, Some(0));

        let Err(errors) = collect_language_item_program(&program) else {
            panic!("wrong child declaration kind should be rejected");
        };

        assert_eq!(
            error_details(errors),
            vec![
                (
                    "language item index.output must mark an associated type".to_string(),
                    Some(output),
                ),
                (
                    "language item index.output is missing".to_string(),
                    Some(root),
                ),
            ],
        );
    }

    #[test]
    fn collect_rejects_nonexported_language_item_root() {
        let program = parse_language_item_source("lang sized\ntrait Hidden\n");
        let root = marker_span_bounds(&program, 0, None);

        let Err(errors) = collect_language_item_program(&program) else {
            panic!("nonexported root should be rejected");
        };

        assert_eq!(
            error_details(errors),
            vec![(
                "language item sized root must be exported".to_string(),
                Some(root),
            )],
        );
    }

    #[test]
    fn collect_rejects_try_without_from_residual_or_control_flow() {
        let program = parse_language_item_source(
            "lang try\n< trait Carrier\n    lang output\n    type Value\n    lang residual\n    type Remainder\n    lang branch\n    split: I64\n",
        );
        let root = marker_span_bounds(&program, 0, None);

        let Err(errors) = collect_language_item_program(&program) else {
            panic!("partial try family should be rejected");
        };

        assert_eq!(
            error_details(errors),
            vec![
                (
                    "language item from_residual is missing".to_string(),
                    Some(root)
                ),
                (
                    "language item control_flow is missing".to_string(),
                    Some(root)
                ),
            ],
        );
    }

    #[test]
    fn collect_rejects_from_residual_without_try_or_control_flow() {
        let program = parse_language_item_source(
            "lang from_residual\n< trait Rebuild\n    lang method\n    rebuild: I64\n",
        );
        let root = marker_span_bounds(&program, 0, None);

        let Err(errors) = collect_language_item_program(&program) else {
            panic!("orphan from_residual should be rejected");
        };

        assert_eq!(
            error_details(errors),
            vec![
                ("language item try is missing".to_string(), Some(root)),
                (
                    "language item control_flow is missing".to_string(),
                    Some(root)
                ),
            ],
        );
    }

    #[test]
    fn collect_rejects_control_flow_without_try_or_from_residual() {
        let program = parse_language_item_source(
            "lang control_flow\n< enum Flow\n    lang break\n    Stop\n    lang continue\n    Next\n",
        );
        let root = marker_span_bounds(&program, 0, None);

        let Err(errors) = collect_language_item_program(&program) else {
            panic!("orphan control_flow should be rejected");
        };

        assert_eq!(
            error_details(errors),
            vec![
                ("language item try is missing".to_string(), Some(root)),
                (
                    "language item from_residual is missing".to_string(),
                    Some(root)
                ),
            ],
        );
    }

    #[test]
    fn collect_rejects_language_item_marker_with_missing_member_target() {
        let mut program = parse_language_item_source(
            "lang drop\n< trait Cleaner\n    lang method\n    clean: I64\n",
        );
        let root = marker_span_bounds(&program, 0, None);
        let method = marker_span_bounds(&program, 0, Some(0));
        let TopLevel::TraitDecl(trait_decl) = &mut program.module.top_levels[0] else {
            panic!("expected marked trait");
        };
        trait_decl.language_items.members[0].member_name = "missing".to_string();
        let Err(errors) = collect_language_item_program(&program) else {
            panic!("missing language item member target should be rejected");
        };

        assert_eq!(
            error_details(errors),
            vec![
                (
                    "language item drop.method targets missing member missing".to_string(),
                    Some(method),
                ),
                (
                    "language item drop.method is missing".to_string(),
                    Some(root),
                ),
            ],
        );
    }
}

mod artifact_exports;
mod exports;
mod extern_crates;
mod imports;
mod module_loading;
mod type_lowering;
mod type_names;

use crate::ast;
use crate::collect::item_index::ItemIndex;
use crate::collect::resolver::{build_resolver_tables, ResolverTables};
use crate::collect::{DeclarationScope, DeclarationTypeVars};
use crate::crate_artifact::ArtifactExport;
use crate::hir::{HirEnum, HirExtern, HirFunction, HirFunctionSig, HirImpl, HirStruct, HirTrait};
use crate::ids::{DefId, ModuleId, TypeVarId};
use crate::language_items::LanguageItems;
use crate::lexer::Span;
use crate::lower::ResolveError;
use crate::type_lowering::TypeLoweringContext;
pub(crate) use exports::{
    child_export_references, collect_exports, export_references_module, merge_export_references,
};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
pub(crate) use type_names::path_names;

pub(crate) struct CollectContext {
    pub(crate) type_vars: DeclarationTypeVars,
    pub(crate) scope: DeclarationScope,
    pub(crate) structs: HashMap<String, HirStruct>,
    pub(crate) enums: HashMap<String, HirEnum>,
    pub(crate) traits: HashMap<String, HirTrait>,
    pub(crate) impls: Vec<HirImpl>,
    pub(crate) externs: Vec<HirExtern>,
    pub(crate) functions: HashMap<String, HirFunction>,
    pub(crate) function_sigs: HashMap<String, HirFunctionSig>,
    pub(crate) type_aliases: HashMap<String, crate::hir::HirTypeAlias>,
    pub(crate) function_type_vars: HashMap<DefId, HashSet<TypeVarId>>,
    pub(crate) errors: Vec<ResolveError>,
    pub(crate) current_function: Option<String>,
    pub(crate) current_crate_name: Option<String>,
    pub(crate) current_qualified_module_prefix: Option<String>,
    pub(crate) current_module_path: PathBuf,
    pub(crate) loaded_modules: HashSet<PathBuf>,
    pub(crate) loaded_module_paths: Vec<(String, PathBuf)>,
    pub(crate) import_aliases: HashMap<String, String>,
    pub(crate) current_trait: Option<String>,
    pub(crate) current_trait_generics: Vec<String>,
    pub(crate) current_generic_owner: Option<DefId>,
    pub(crate) current_generic_params: Vec<String>,
    pub(crate) current_generic_kinds: Vec<crate::type_services::kind::Kind>,
    pub(crate) infix_precedence: HashMap<String, u8>,
    pub(crate) loaded_prelude_export_ids: HashMap<String, ArtifactExport>,
    pub(crate) module_file_cache: HashMap<PathBuf, ast::Module>,
    pub(crate) dependency_root_export_ids: HashMap<String, HashMap<String, ArtifactExport>>,
    pub(crate) language_items: LanguageItems<DefId>,
    pub(crate) canonical_import_aliases: HashMap<String, DefId>,
    pub(crate) canonical_names_by_id: HashMap<DefId, String>,
    pub(crate) scoped_module_aliases: HashMap<String, HashMap<String, DefId>>,
    pub(crate) export_aliases: HashMap<String, String>,
    /// Every current-crate item by canonical path, known before any header is
    /// lowered, so imports resolve without waiting for their target's types.
    pub(crate) declared_items: HashMap<String, DefId>,
    /// Imports resolved to an item whose header was not lowered yet; bound by
    /// `bind_deferred_imports` once every header is.
    pub(crate) deferred_import_bindings: Vec<(String, DefId)>,
}

pub(crate) struct LocalCollection {
    pub(crate) structs: HashMap<String, HirStruct>,
    pub(crate) enums: HashMap<String, HirEnum>,
    pub(crate) traits: HashMap<String, HirTrait>,
    pub(crate) impls: Vec<HirImpl>,
    pub(crate) externs: Vec<HirExtern>,
    pub(crate) functions: HashMap<String, HirFunction>,
    pub(crate) function_sigs: HashMap<String, HirFunctionSig>,
    pub(crate) type_aliases: HashMap<String, crate::hir::HirTypeAlias>,
    pub(crate) function_type_vars: HashMap<DefId, HashSet<TypeVarId>>,
    pub(crate) infix_precedence: HashMap<String, u8>,
    pub(crate) loaded_module_paths: Vec<(String, PathBuf)>,
    pub(crate) type_vars: DeclarationTypeVars,
    pub(crate) loaded_prelude_export_ids: HashMap<String, ArtifactExport>,
    pub(crate) module_file_cache: HashMap<PathBuf, ast::Module>,
    pub(crate) dependency_root_export_ids: HashMap<String, HashMap<String, ArtifactExport>>,
    pub(crate) language_items: LanguageItems<DefId>,
    pub(crate) resolver: ResolverTables,
    pub(crate) errors: Vec<ResolveError>,
}

impl CollectContext {
    pub(crate) fn new() -> Self {
        Self {
            type_vars: DeclarationTypeVars::new(),
            scope: DeclarationScope::new(),
            structs: HashMap::new(),
            enums: HashMap::new(),
            traits: HashMap::new(),
            impls: Vec::new(),
            externs: Vec::new(),
            functions: HashMap::new(),
            function_sigs: HashMap::new(),
            type_aliases: HashMap::new(),
            errors: Vec::new(),
            function_type_vars: HashMap::new(),
            current_function: None,
            current_crate_name: None,
            current_qualified_module_prefix: None,
            current_module_path: PathBuf::new(),
            loaded_modules: HashSet::new(),
            loaded_module_paths: Vec::new(),
            import_aliases: HashMap::new(),
            current_trait: None,
            current_trait_generics: Vec::new(),
            current_generic_owner: None,
            current_generic_params: Vec::new(),
            current_generic_kinds: Vec::new(),
            infix_precedence: HashMap::new(),
            loaded_prelude_export_ids: HashMap::new(),
            module_file_cache: HashMap::new(),
            dependency_root_export_ids: HashMap::new(),
            language_items: LanguageItems::default(),
            canonical_import_aliases: HashMap::new(),
            canonical_names_by_id: HashMap::new(),
            scoped_module_aliases: HashMap::new(),
            export_aliases: HashMap::new(),
            declared_items: HashMap::new(),
            deferred_import_bindings: Vec::new(),
        }
    }

    pub(crate) fn bootstrap_for_collection(
        _inject_prelude: bool,
        current_crate_name: Option<&str>,
        file_path: Option<&PathBuf>,
    ) -> Self {
        let mut context = Self::new();
        context.current_crate_name = current_crate_name.map(ToString::to_string);

        if let Some(file_path) = file_path {
            context.current_module_path = file_path.clone();
        }

        context.register_current_crate_root();
        context
    }

    pub(crate) fn register_current_crate_root(&mut self) {
        let Some(crate_name) = self.current_crate_name.clone() else {
            return;
        };

        if self.current_module_path.as_os_str().is_empty() {
            return;
        }

        if !self
            .loaded_module_paths
            .iter()
            .any(|(name, path)| name == &crate_name && path == &self.current_module_path)
        {
            self.loaded_module_paths
                .insert(0, (crate_name, self.current_module_path.clone()));
        }
    }

    pub(crate) fn seed_source_graph(&mut self, graph: &crate::source_loader::ModuleGraph) {
        for (qualified_name, path) in graph.loaded_module_paths() {
            if !self
                .loaded_module_paths
                .iter()
                .any(|(name, existing_path)| name == &qualified_name && existing_path == &path)
            {
                self.loaded_module_paths.push((qualified_name, path));
            }
        }

        for (path, module) in graph.module_file_cache() {
            self.module_file_cache.entry(path).or_insert(module);
        }
    }

    pub(crate) fn into_local_collection(
        mut self,
        item_index: &ItemIndex,
        root_module_id: ModuleId,
        current_crate_name: Option<&str>,
    ) -> LocalCollection {
        self.normalize_type_aliases();
        let Self {
            type_vars,
            scope: _,
            structs,
            enums,
            traits,
            impls,
            externs,
            functions,
            function_sigs,
            type_aliases,
            function_type_vars,
            errors,
            current_function: _,
            current_crate_name: _,
            current_qualified_module_prefix: _,
            current_module_path: _,
            loaded_modules: _,
            loaded_module_paths,
            import_aliases: _,
            current_trait: _,
            current_trait_generics: _,
            current_generic_owner: _,
            current_generic_params: _,
            current_generic_kinds: _,
            infix_precedence,
            loaded_prelude_export_ids,
            module_file_cache,
            dependency_root_export_ids,
            language_items,
            canonical_import_aliases,
            canonical_names_by_id,
            scoped_module_aliases,
            export_aliases,
            declared_items: _,
            deferred_import_bindings: _,
        } = self;

        let mut resolver = build_resolver_tables(
            item_index,
            root_module_id,
            current_crate_name,
            &export_aliases,
            canonical_import_aliases,
        );
        for (def_id, name) in canonical_names_by_id {
            resolver.item_names_by_id.entry(def_id).or_insert(name);
        }
        for (module, aliases) in scoped_module_aliases {
            resolver
                .scoped_module_aliases
                .entry(module)
                .or_default()
                .extend(aliases);
        }
        LocalCollection {
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
            errors,
        }
    }

    fn normalize_type_aliases(&mut self) {
        let mut env = crate::type_services::normalize::TypeNormalizationEnv::new();
        self.populate_type_normalization_env(&mut env);
        let aliases = self
            .type_aliases
            .values()
            .map(|alias| (alias.id, alias.ty.clone()))
            .collect::<HashMap<_, _>>();
        let mut normalized = HashMap::new();
        for (id, ty) in aliases {
            match crate::type_services::normalize::TypeNormalizer::new(&env).normalize(&ty) {
                Ok(ty) => {
                    normalized.insert(id, ty);
                }
                Err(error) => self.push_error(error.to_string()),
            }
        }
        for alias in self.type_aliases.values_mut() {
            if let Some(ty) = normalized.get(&alias.id) {
                alias.ty = ty.clone();
            }
        }
    }

    pub(crate) fn record_loaded_prelude_exports(
        &mut self,
        exports: impl IntoIterator<Item = (String, ArtifactExport)>,
    ) {
        self.loaded_prelude_export_ids.extend(exports);
    }

    pub(crate) fn push_error(&mut self, message: String) {
        self.errors.push(ResolveError::non_source(message));
    }

    pub(crate) fn push_error_with_span(&mut self, message: String, span: Span) {
        self.errors.push(ResolveError::with_span(message, span));
    }
}

#[cfg(test)]
mod tests;

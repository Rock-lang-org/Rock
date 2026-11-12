mod artifact_exports;
mod exports;
mod extern_crates;
mod generic_scope;
mod imports;
mod infix_operators;
mod module_loading;
mod module_sources;
mod name_tables;
mod type_lowering;
mod type_names;
mod variant_names;

use crate::ast;
use crate::collect::item_index::ItemIndex;
use crate::collect::resolver::{build_resolver_tables, ResolverTables, VariantAlias};
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
pub(crate) use type_names::{is_known_nominal_type_name, path_names};

pub(crate) use generic_scope::{GenericFrame, GenericScope};
use module_sources::ModuleSources;
use name_tables::{NameTables, ResolverNameTables};

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
    /// Trait and generic parameters in scope while a header is lowered.
    pub(crate) generics: GenericScope,
    /// Source files, the current file and the parsed-module cache.
    pub(crate) modules: ModuleSources,
    pub(crate) infix_precedence: HashMap<String, u8>,
    /// Where each `infix_precedence` entry was declared.
    infix_origins: HashMap<String, infix_operators::InfixOrigin>,
    pub(crate) loaded_prelude_export_ids: HashMap<String, ArtifactExport>,
    pub(crate) dependency_root_export_ids: HashMap<String, HashMap<String, ArtifactExport>>,
    pub(crate) language_items: LanguageItems<DefId>,
    /// Import, re-export and canonical item names.
    pub(crate) names: NameTables,
    /// Enum variants imported under a short name; see `ResolverTables`.
    pub(crate) variant_aliases: HashMap<String, VariantAlias>,
    pending_variant_imports: Vec<PendingVariantImport>,
    explicit_imports: HashMap<String, Option<VariantAlias>>,
}

/// A variant import whose variant name is checked after collection.
struct PendingVariantImport {
    key: String,
    target: VariantAlias,
    written: String,
    span: Option<Span>,
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
    /// Declaring crate of each operator inherited from a dependency.
    pub(crate) infix_owners: HashMap<String, String>,
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
            generics: GenericScope::default(),
            modules: ModuleSources::default(),
            infix_precedence: HashMap::new(),
            infix_origins: HashMap::new(),
            loaded_prelude_export_ids: HashMap::new(),
            dependency_root_export_ids: HashMap::new(),
            language_items: LanguageItems::default(),
            names: NameTables::default(),
            variant_aliases: HashMap::new(),
            pending_variant_imports: Vec::new(),
            explicit_imports: HashMap::new(),
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
            context.modules.set_current_path(file_path.clone());
        }

        context.register_current_crate_root();
        context
    }

    pub(crate) fn register_current_crate_root(&mut self) {
        let Some(crate_name) = self.current_crate_name.clone() else {
            return;
        };

        self.modules.register_root(&crate_name);
    }

    pub(crate) fn seed_source_graph(&mut self, graph: &crate::source_loader::ModuleGraph) {
        for (qualified_name, path) in graph.loaded_module_paths() {
            self.modules.add_path(qualified_name, path);
        }

        for (path, module) in graph.module_file_cache() {
            self.modules.cache_module(path, module);
        }
    }

    pub(crate) fn into_local_collection(
        mut self,
        item_index: &ItemIndex,
        root_module_id: ModuleId,
        current_crate_name: Option<&str>,
    ) -> LocalCollection {
        self.normalize_type_aliases();
        self.check_variant_imports();
        let variant_exports = self.variant_exports();
        let infix_owners = self.dependency_infix_owners();
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
            generics: _,
            modules,
            infix_precedence,
            infix_origins: _,
            loaded_prelude_export_ids,
            dependency_root_export_ids,
            language_items,
            names,
            variant_aliases,
            pending_variant_imports: _,
            explicit_imports: _,
        } = self;
        let (loaded_module_paths, module_file_cache) = modules.into_registry_and_cache();
        let ResolverNameTables {
            canonical_import_aliases,
            canonical_names_by_id,
            scoped_module_aliases,
            export_aliases,
        } = names.into_resolver_tables();

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
        resolver.variant_aliases = variant_aliases;
        resolver.variant_exports = variant_exports;
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
            infix_owners,
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

    /// Imports the variant named by `path` (`Enum::Variant`, any enum prefix
    /// the context knows) under its short name, keyed for `module_prefix`.
    ///
    /// Imports are handled before enum bodies are collected, so only the
    /// enum is resolved here; the variant name is checked by
    /// [`Self::check_variant_imports`] once every declaration is known.
    pub(crate) fn import_enum_variant(
        &mut self,
        path: &[String],
        module_prefix: Option<&str>,
        span: Option<Span>,
        explicit: bool,
    ) -> bool {
        let written = path.join("::");
        let Some(mut target) = module_prefix
            .map(|prefix| format!("{prefix}::{written}"))
            .into_iter()
            .chain(std::iter::once(written.clone()))
            .find_map(|candidate| self.resolve_variant_source(&candidate))
        else {
            return false;
        };
        let Some(variant) = path.last() else {
            return false;
        };
        let key = match module_prefix {
            Some(prefix) if !prefix.is_empty() => format!("{prefix}::{variant}"),
            _ => variant.clone(),
        };
        target.fallback = !explicit;
        // The collector may try both the written and qualified path for one
        // import; validate that source occurrence only once.
        if self.pending_variant_imports.iter().any(|pending| {
            pending.key == key && pending.target == target && pending.span == span
        }) {
            return true;
        }
        if explicit {
            self.record_explicit_import(&key, Some(target.clone()), span.clone());
            self.variant_aliases.insert(key.clone(), target.clone());
        } else {
            self.variant_aliases.entry(key.clone())
                .and_modify(|existing| {
                    if existing.fallback {
                        *existing = target.clone();
                    }
                })
                .or_insert_with(|| target.clone());
        }
        self.pending_variant_imports.push(PendingVariantImport {
            key,
            target,
            written,
            span,
        });
        true
    }

    /// Drops and reports variant imports whose enum has no such variant.
    fn check_variant_imports(&mut self) {
        for pending in std::mem::take(&mut self.pending_variant_imports) {
            let alias = &pending.target;
            let exists = self
                .enums
                .values()
                .find(|enumeration| enumeration.id == alias.enum_id)
                .is_some_and(|enumeration| {
                    enumeration
                        .variants
                        .iter()
                        .any(|variant| variant.name == alias.variant)
                });
            if exists {
                continue;
            }
            if self.variant_aliases.get(&pending.key) == Some(alias) {
                self.variant_aliases.remove(&pending.key);
            }
            let message = format!("unresolved import '{}'", pending.written);
            match pending.span {
                Some(span) => self.push_error_with_span(message, span),
                None => self.push_error(message),
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

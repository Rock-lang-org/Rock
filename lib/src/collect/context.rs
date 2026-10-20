use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::ast;
use crate::collect::item_index::ItemIndex;
use crate::collect::named_items::NamedItems;
use crate::collect::resolver::{build_resolver_tables, ResolverTables};
use crate::collect::{DeclarationScope, DeclarationTypeVars};
use crate::crate_artifact::ArtifactExport;
use crate::crate_system::{CrateContext, ExternCrateRef};
use crate::hir::{
    HirEnum, HirExtern, HirFunction, HirFunctionSig, HirImpl, HirParam, HirStruct, HirTrait,
};
use crate::ids::{DefId, ModuleId, TypeVarId};
use crate::language_items::LanguageItems;
use crate::lexer::Span;
use crate::lexer::lossless::SyntaxContext;
use crate::lower::ResolveError;
use crate::type_lowering::{ResolvedNominalType, TypeLowerer, TypeLoweringContext};
use crate::types::{GenericParamId, Type};

pub(crate) fn seg_name(seg: &ast::IdentOrType) -> Option<String> {
    match seg {
        ast::IdentOrType::Ident(ident) => Some(ident.name.clone()),
        ast::IdentOrType::Type(ast::ParseType::Type(inner)) => Some(inner.name.clone()),
        _ => None,
    }
}

pub(crate) fn path_names(path: &[ast::IdentOrType]) -> Vec<String> {
    path.iter().filter_map(seg_name).collect()
}

fn is_builtin_type_name(name: &str) -> bool {
    matches!(
        name,
        "I8" | "I16"
            | "I32"
            | "I64"
            | "U8"
            | "U16"
            | "U32"
            | "U64"
            | "F32"
            | "F64"
            | "Bool"
            | "Str"
            | "Char"
            | "()"
    )
}

fn collect_generic_names_from_parse_type<F>(
    ty: &ast::ParseType,
    generic_params: &mut Vec<String>,
    is_known_type_name: &F,
) where
    F: Fn(&str) -> bool,
{
    match ty {
        ast::ParseType::Type(inner) => {
            if inner.generics.is_empty()
                && !is_builtin_type_name(&inner.name)
                && !is_known_type_name(&inner.name)
                && !generic_params.contains(&inner.name)
            {
                generic_params.push(inner.name.clone());
            }
            for generic in &inner.generics {
                collect_generic_names_from_parse_type(generic, generic_params, is_known_type_name);
            }
        }
        ast::ParseType::Application(application) => {
            collect_generic_names_from_parse_type(
                &application.constructor,
                generic_params,
                is_known_type_name,
            );
            for arg in &application.args {
                collect_generic_names_from_parse_type(arg, generic_params, is_known_type_name);
            }
        }
        ast::ParseType::Lambda(lambda) => {
            collect_generic_names_from_parse_type(&lambda.body, generic_params, is_known_type_name);
        }
        ast::ParseType::Hole(_) => {}
        ast::ParseType::Associated { base, .. } => {
            for generic in &base.generics {
                collect_generic_names_from_parse_type(generic, generic_params, is_known_type_name);
            }
        }
        ast::ParseType::Slice(inner)
        | ast::ParseType::Reference { pointee: inner, .. }
        | ast::ParseType::Pointer(inner) => {
            collect_generic_names_from_parse_type(inner, generic_params, is_known_type_name);
        }
        ast::ParseType::Array { inner, .. } => {
            collect_generic_names_from_parse_type(inner, generic_params, is_known_type_name);
        }
        ast::ParseType::Function(args) | ast::ParseType::Tuple(args) => {
            for arg in args {
                collect_generic_names_from_parse_type(arg, generic_params, is_known_type_name);
            }
        }
        ast::ParseType::Unit(_) => {}
    }
}

fn is_known_nominal_type_name(context: &CollectContext, name: &str) -> bool {
    if context.structs.contains_key(name) || context.enums.contains_key(name) {
        return true;
    }

    if let Some(qualified) = context.import_aliases.get(name) {
        if context.structs.contains_key(qualified) || context.enums.contains_key(qualified) {
            return true;
        }
    }

    if let Some(id) = context.canonical_import_aliases.get(name) {
        return context
            .structs
            .values()
            .any(|structure| structure.id == *id)
            || context.enums.values().any(|enum_def| enum_def.id == *id);
    }

    false
}

pub(crate) fn collect_exports(module: &ast::Module) -> HashMap<String, Option<String>> {
    let mut exports = HashMap::new();
    for top_level in &module.top_levels {
        match top_level {
            ast::TopLevel::Export(path) => {
                let segments = match path { ast::Path::Ident(path) => &path.path, ast::Path::Type(path) => &path.path };
                if crate::collect::resolver::binding_path(segments).is_some_and(|path| path.iter().any(|(_, context)| *context != SyntaxContext::ROOT)) { continue; }
                let names = match path {
                    ast::Path::Ident(ip) => path_names(&ip.path),
                    ast::Path::Type(tp) => path_names(&tp.path),
                };
                if let Some(short_name) = names.last().cloned() {
                    let source = if names.len() > 1 {
                        Some(names.join("::"))
                    } else {
                        None
                    };
                    exports.insert(short_name, source);
                }
            }
            ast::TopLevel::FunctionDecl(fd) if fd.exported && fd.name.context == SyntaxContext::ROOT => {
                exports.insert(fd.name.name.clone(), None);
            }
            ast::TopLevel::Extern(sig) if sig.exported && sig.name.context == SyntaxContext::ROOT => {
                exports.insert(sig.name.name.clone(), None);
            }
            ast::TopLevel::StructDecl(sd) if sd.exported && sd.name.context == SyntaxContext::ROOT => {
                exports.insert(sd.name.name.clone(), None);
            }
            ast::TopLevel::EnumDecl(ed) if ed.exported && ed.name.context == SyntaxContext::ROOT => {
                exports.insert(ed.name.name.clone(), None);
            }
            ast::TopLevel::TraitDecl(td) if td.exported && td.name.context == SyntaxContext::ROOT => {
                exports.insert(td.name.name.clone(), None);
            }
            ast::TopLevel::Mod(ident, true) if ident.context == SyntaxContext::ROOT => {
                exports.insert(ident.name.clone(), None);
            }
            ast::TopLevel::GlobExport(module_path) => {
                if crate::collect::resolver::binding_path(module_path).is_some_and(|path| path.iter().any(|(_, context)| *context != SyntaxContext::ROOT)) { continue; }
                let sentinel = format!("{}::*", path_names(module_path).join("::"));
                exports.insert(sentinel.clone(), Some(sentinel));
            }
            _ => {}
        }
    }
    exports
}

fn find_inline_module<'a>(module: &'a ast::Module, name: &str) -> Option<&'a ast::Module> {
    module
        .top_levels
        .iter()
        .find_map(|top_level| match top_level {
            ast::TopLevel::Module(module_decl)
                if module_decl.0.name.as_ref().map(|ident| ident.name.as_str()) == Some(name) =>
            {
                Some(&module_decl.0)
            }
            _ => None,
        })
}

pub(crate) fn export_references_module(
    exports: &HashMap<String, Option<String>>,
    module_name: &str,
    qualified_module_name: &str,
) -> bool {
    exports.values().any(|source| {
        source
            .as_deref()
            .map(|source| {
                export_reference_tail(source, module_name, qualified_module_name).is_some()
            })
            .unwrap_or(false)
    })
}

fn export_reference_tail<'a>(
    source: &'a str,
    module_name: &str,
    qualified_module_name: &str,
) -> Option<&'a str> {
    [module_name, qualified_module_name]
        .into_iter()
        .find_map(|prefix| {
            if source == prefix {
                Some("")
            } else {
                source
                    .strip_prefix(prefix)
                    .and_then(|tail| tail.strip_prefix("::"))
            }
        })
}

pub(crate) fn child_export_references(
    exports: &HashMap<String, Option<String>>,
    module_name: &str,
    qualified_module_name: &str,
) -> HashMap<String, Option<String>> {
    let mut references = HashMap::new();
    for (export_name, source) in exports {
        let Some(source) = source.as_deref() else {
            continue;
        };
        let Some(tail) = export_reference_tail(source, module_name, qualified_module_name) else {
            continue;
        };
        if tail.is_empty() || tail == "*" {
            continue;
        }

        let key = if tail.ends_with("::*") {
            tail.to_string()
        } else {
            export_name.clone()
        };
        references.insert(key, Some(tail.to_string()));
    }

    references
}

pub(crate) fn merge_export_references(
    exports: &HashMap<String, Option<String>>,
    inherited_references: HashMap<String, Option<String>>,
) -> HashMap<String, Option<String>> {
    let mut references = exports.clone();
    for (key, value) in inherited_references {
        insert_export_reference(&mut references, key, value);
    }
    references
}

fn insert_export_reference(
    references: &mut HashMap<String, Option<String>>,
    key: String,
    value: Option<String>,
) {
    if references.values().any(|existing| existing == &value) {
        return;
    }

    if !references.contains_key(&key) {
        references.insert(key, value);
        return;
    }

    let fallback = value.as_deref().unwrap_or(&key);
    let mut candidate = fallback.to_string();
    let mut ordinal = 0;
    while references.contains_key(&candidate) {
        ordinal += 1;
        candidate = format!("{}#{}", fallback, ordinal);
    }
    references.insert(candidate, value);
}

pub(crate) struct CollectContext {
    pub(crate) type_vars: DeclarationTypeVars,
    pub(crate) scope: DeclarationScope,
    pub(crate) structs: NamedItems<HirStruct>,
    pub(crate) enums: NamedItems<HirEnum>,
    pub(crate) traits: NamedItems<HirTrait>,
    pub(crate) impls: Vec<HirImpl>,
    pub(crate) externs: Vec<HirExtern>,
    pub(crate) functions: NamedItems<HirFunction>,
    pub(crate) function_sigs: NamedItems<HirFunctionSig>,
    pub(crate) type_aliases: NamedItems<crate::hir::HirTypeAlias>,
    pub(crate) function_type_vars: HashMap<DefId, HashSet<TypeVarId>>,
    pub(crate) errors: Vec<ResolveError>,
    pub(crate) current_function: Option<String>,
    pub(crate) current_crate_name: Option<String>,
    pub(crate) current_qualified_module_prefix: Option<String>,
    pub(crate) current_module_path: PathBuf,
    pub(crate) loaded_modules: HashSet<PathBuf>,
    pub(crate) loaded_module_paths: Vec<(String, PathBuf)>,
    pub(crate) import_aliases: HashMap<String, String>,
    pub(crate) explicit_import_aliases: HashMap<String, String>,
    pub(crate) current_trait: Option<String>,
    pub(crate) current_trait_id: Option<DefId>,
    pub(crate) current_trait_generics: Vec<String>,
    pub(crate) current_generic_owner: Option<DefId>,
    pub(crate) current_generic_params: Vec<String>,
    pub(crate) current_generic_kinds: Vec<crate::type_services::kind::Kind>,
    pub(crate) generic_contexts: HashMap<GenericParamId, SyntaxContext>,
    pub(crate) item_resolution: ResolverTables,
    pub(crate) current_module_id: Option<ModuleId>,
    pub(crate) infix_precedence: HashMap<String, u8>,
    pub(crate) loaded_prelude_export_ids: HashMap<String, ArtifactExport>,
    pub(crate) module_file_cache: HashMap<PathBuf, ast::Module>,
    pub(crate) dependency_root_export_ids: HashMap<String, HashMap<String, ArtifactExport>>,
    pub(crate) language_items: LanguageItems<DefId>,
    pub(crate) canonical_import_aliases: HashMap<String, DefId>,
    pub(crate) canonical_names_by_id: HashMap<DefId, String>,
    pub(crate) scoped_module_aliases: HashMap<String, HashMap<String, DefId>>,
    pub(crate) export_aliases: HashMap<String, String>,
    pub(crate) export_function_aliases: HashMap<String, String>,
}

pub(crate) struct LocalCollection {
    pub(crate) structs: NamedItems<HirStruct>,
    pub(crate) enums: NamedItems<HirEnum>,
    pub(crate) traits: NamedItems<HirTrait>,
    pub(crate) impls: Vec<HirImpl>,
    pub(crate) externs: Vec<HirExtern>,
    pub(crate) functions: NamedItems<HirFunction>,
    pub(crate) function_sigs: NamedItems<HirFunctionSig>,
    pub(crate) type_aliases: NamedItems<crate::hir::HirTypeAlias>,
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

impl TypeLoweringContext for CollectContext {
    fn member_context_matches(&self, owner: DefId, actual: SyntaxContext, declared: SyntaxContext) -> bool {
        self.item_resolution.member_context_matches(owner, actual, declared)
    }
    fn current_trait_definition(&self) -> Option<HirTrait> {
        if let Some(id) = self.current_trait_id { return self.traits.get_id(id).cloned(); }
        self.current_trait.as_ref().and_then(|name| self.resolve_trait_type(name, SyntaxContext::ROOT))
    }
    fn associated_context(&self, owner: DefId, id: crate::ids::AssocTypeId) -> SyntaxContext {
        self.item_resolution.associated_contexts.get(&(owner, id)).copied().unwrap_or_default()
    }
    fn constructor_params(&self, id: DefId) -> Option<Vec<crate::types::GenericParamDecl>> {
        self.structs.values().find(|item| item.id == id).map(|item| item.generic_params.clone())
            .or_else(|| self.enums.values().find(|item| item.id == id).map(|item| item.generic_params.clone()))
            .or_else(|| self.type_aliases.values().find(|item| item.id == id).map(|item| item.generic_params.clone()))
    }
    fn push_type_error(&mut self, message: String, span: Span) {
        self.push_error_with_span(message, span);
    }

    fn current_trait_name(&self) -> Option<String> {
        self.current_trait.clone()
    }

    fn resolve_nominal_type(&self, name: &str, context: SyntaxContext) -> Option<ResolvedNominalType> {
        if let Some(id) = self.resolve_contextual_item(name, context) {
            return self.structs.get_id(id).cloned().map(ResolvedNominalType::Struct)
                .or_else(|| self.enums.get_id(id).cloned().map(ResolvedNominalType::Enum));
        }
        if context != SyntaxContext::ROOT || self.item_resolution.contextual_scope(self.current_module_id) { return None; }
        if let Some(structure) = self.structs.get(name).cloned() {
            return Some(ResolvedNominalType::Struct(structure));
        }

        if let Some(enum_def) = self.enums.get(name).cloned() {
            return Some(ResolvedNominalType::Enum(enum_def));
        }

        if let Some(id) = self.canonical_import_aliases.get(name).copied() {
            if let Some(structure) = self
                .structs
                .values()
                .find(|structure| structure.id == id)
                .cloned()
            {
                return Some(ResolvedNominalType::Struct(structure));
            }

            if let Some(enum_def) = self
                .enums
                .values()
                .find(|enum_def| enum_def.id == id)
                .cloned()
            {
                return Some(ResolvedNominalType::Enum(enum_def));
            }
        }

        if let Some(qualified) = self.import_aliases.get(name) {
            if let Some(structure) = self.structs.get(qualified).cloned() {
                return Some(ResolvedNominalType::Struct(structure));
            }

            if let Some(enum_def) = self.enums.get(qualified).cloned() {
                return Some(ResolvedNominalType::Enum(enum_def));
            }
        }

        None
    }

    fn resolve_trait_type(&self, name: &str, context: SyntaxContext) -> Option<HirTrait> {
        if let Some(id) = self.resolve_contextual_item(name, context) {
            return self.traits.get_id(id).cloned();
        }
        if context != SyntaxContext::ROOT || self.item_resolution.contextual_scope(self.current_module_id) { return None; }
        if let Some(trait_def) = self.traits.get(name).cloned() {
            return Some(trait_def);
        }

        if let Some(id) = self.canonical_import_aliases.get(name).copied() {
            if let Some(trait_def) = self.traits.values().find(|trait_def| trait_def.id == id) {
                return Some(trait_def.clone());
            }
        }

        self.import_aliases
            .get(name)
            .and_then(|qualified| self.traits.get(qualified))
            .cloned()
    }

    fn resolve_type_alias(&self, name: &str, context: SyntaxContext) -> Option<crate::hir::HirTypeAlias> {
        if let Some(id) = self.resolve_contextual_item(name, context) {
            return self.type_aliases.get_id(id).cloned();
        }
        if context != SyntaxContext::ROOT || self.item_resolution.contextual_scope(self.current_module_id) { return None; }
        self.type_aliases
            .get(name)
            .cloned()
            .or_else(|| {
                self.canonical_import_aliases.get(name).and_then(|id| {
                    self.type_aliases
                        .values()
                        .find(|alias| alias.id == *id)
                        .cloned()
                })
            })
            .or_else(|| {
                self.import_aliases
                    .get(name)
                    .and_then(|qualified| self.type_aliases.get(qualified))
                    .cloned()
            })
    }

    fn generic_type_for_name(&mut self, name: &str, context: SyntaxContext, span: Span) -> Type {
        self.contextual_generic_type(name, context, span)
    }

    fn scoped_generic_type(&mut self, name: &str, context: SyntaxContext, _span: Span) -> Option<Type> {
        let owner = self.current_generic_owner?;
        self.current_generic_params.iter().enumerate().find_map(|(index, param)| {
            let id = GenericParamId { owner, index: index as u32 };
            (param == name && (self.generic_context(id) == context
                || (name == "Self" && self.member_context_matches(owner, context, self.generic_context(id)))))
                .then_some(Type::Generic(id))
        })
    }

    fn generic_context(&self, id: GenericParamId) -> SyntaxContext {
        self.generic_contexts.get(&id).copied().unwrap_or(SyntaxContext::ROOT)
    }

    fn populate_type_normalization_env(
        &self,
        env: &mut crate::type_services::normalize::TypeNormalizationEnv,
    ) {
        for structure in self.structs.values() {
            env.register_constructor(
                structure.id,
                crate::types::NominalTypeKind::Struct,
                crate::type_lowering::constructor_kind(&structure.generic_params),
            );
            for param in &structure.generic_params {
                env.register_generic_kind(param.id, param.kind.clone());
            }
        }
        for enumeration in self.enums.values() {
            env.register_constructor(
                enumeration.id,
                crate::types::NominalTypeKind::Enum,
                crate::type_lowering::constructor_kind(&enumeration.generic_params),
            );
            for param in &enumeration.generic_params {
                env.register_generic_kind(param.id, param.kind.clone());
            }
        }
        for trait_def in self.traits.values() {
            for param in &trait_def.generic_params {
                env.register_generic_kind(param.id, param.kind.clone());
            }
            for method in trait_def.methods.values() {
                for param in &method.generic_params {
                    env.register_generic_kind(param.id, param.kind.clone());
                }
            }
            for signature in trait_def.signatures.values() {
                for param in &signature.generic_params {
                    env.register_generic_kind(param.id, param.kind.clone());
                }
            }
            for associated_type in &trait_def.associated_types {
                env.register_projection_kind(
                    crate::types::AssociatedTypeKey {
                        owner: trait_def.id,
                        assoc_type_id: associated_type.id,
                    },
                    associated_type.kind.clone(),
                );
            }
        }
        for function in self.functions.values() {
            for param in &function.generic_params {
                env.register_generic_kind(param.id, param.kind.clone());
            }
        }
        for signature in self.function_sigs.values() {
            for param in &signature.generic_params {
                env.register_generic_kind(param.id, param.kind.clone());
            }
        }
        for imp in &self.impls {
            for param in imp.type_generics.iter().chain(&imp.trait_generics) {
                env.register_generic_kind(param.id, param.kind.clone());
            }
            for method in imp.methods.values() {
                for param in &method.generic_params {
                    env.register_generic_kind(param.id, param.kind.clone());
                }
            }
        }
        crate::type_lowering::register_type_aliases(env, self.type_aliases.values());
        if let Some(owner) = self.current_generic_owner {
            for (index, kind) in self.current_generic_kinds.iter().enumerate() {
                env.register_generic_kind(
                    GenericParamId {
                        owner,
                        index: index as u32,
                    },
                    kind.clone(),
                );
            }
        }
    }
}

impl CollectContext {
    fn resolve_contextual_item(&self, name: &str, context: SyntaxContext) -> Option<DefId> {
        let mut path = name.split("::").map(|name| (name.to_string(), SyntaxContext::ROOT)).collect::<Vec<_>>();
        path.last_mut()?.1 = context;
        self.item_resolution.resolve_binding_path(self.current_module_id, &path)
    }

    pub(crate) fn seed_item_resolution(&mut self, index: &ItemIndex, root: ModuleId) {
        self.item_resolution = build_resolver_tables(
            index, root, self.current_crate_name.as_deref(), &HashMap::new(), &HashMap::new(), &HashMap::new(),
        );
        // Canonical metadata aliases (including an explicitly supplied prelude)
        // are root bindings too. Definition-site lookup must not depend on the
        // lowerer's spelling-only alias fallback or override source declarations.
        for (alias, id) in &self.canonical_import_aliases {
            let path = alias.split("::").map(|name| (name.to_string(), SyntaxContext::ROOT)).collect::<Vec<_>>();
            if !self.item_resolution.binding_paths.contains_key(&path) {
                self.item_resolution.binding_aliases.entry(path).or_insert(*id);
            }
        }
        let hidden = index.items().iter().filter(|item|
            index.item_binding_path(item).iter().any(|(_, context)| *context != SyntaxContext::ROOT)
        ).map(|item| item.def_id).collect::<HashSet<_>>();
        self.structs.hide_from_root(&hidden);
        self.enums.hide_from_root(&hidden);
        self.traits.hide_from_root(&hidden);
        self.functions.hide_from_root(&hidden);
        self.function_sigs.hide_from_root(&hidden);
        self.type_aliases.hide_from_root(&hidden);
    }

    pub(crate) fn register_generic_syntax(&mut self, owner: DefId, syntax: &[ast::GenericParamDecl]) {
        for (index, param) in syntax.iter().enumerate() {
            self.generic_contexts.insert(GenericParamId { owner, index: index as u32 }, param.name.context);
        }
    }

    fn cached_module_for_path(&mut self, file_path: &PathBuf) -> Option<ast::Module> {
        let canonical_path = file_path
            .canonicalize()
            .unwrap_or_else(|_| file_path.clone());
        self.module_file_cache
            .get(&canonical_path)
            .or_else(|| self.module_file_cache.get(file_path))
            .cloned()
            .inspect(|_| {
                self.loaded_modules.insert(canonical_path);
            })
    }

    fn cached_module_for_qualified_name(&mut self, qualified_name: &str) -> Option<ast::Module> {
        let file_path = self.loaded_module_path_for_qualified_name(qualified_name)?;

        self.cached_module_for_path(&file_path)
    }

    fn loaded_module_path_by_name(&self, module_name: &str) -> Option<PathBuf> {
        self.loaded_module_paths
            .iter()
            .find(|(name, _)| name == module_name)
            .map(|(_, path)| path.clone())
    }

    pub(crate) fn loaded_module_path_for_prefix(
        &self,
        module_name: &str,
        prefix: Option<&str>,
    ) -> Option<PathBuf> {
        if let Some(prefix) = prefix {
            let qualified_name = format!("{}::{}", prefix, module_name);
            return self.loaded_module_path_for_qualified_name(&qualified_name);
        }

        self.loaded_module_path_for_qualified_name(module_name)
    }

    fn loaded_module_path_for_qualified_name(&self, qualified_name: &str) -> Option<PathBuf> {
        self.loaded_module_path_for_current_crate_name(qualified_name)
            .or_else(|| self.loaded_module_path_by_name(qualified_name))
    }

    fn loaded_module_path_for_current_crate_name(&self, qualified_name: &str) -> Option<PathBuf> {
        let crate_name = self.current_crate_name.as_ref()?;
        if qualified_name == crate_name || qualified_name.starts_with(&format!("{}::", crate_name))
        {
            return None;
        }

        let first_segment = qualified_name.split("::").next().unwrap_or(qualified_name);
        if self.loaded_module_path_by_name(first_segment).is_some() {
            return None;
        }

        self.loaded_module_path_by_name(&format!("{}::{}", crate_name, qualified_name))
    }

    pub(crate) fn new() -> Self {
        Self {
            type_vars: DeclarationTypeVars::new(),
            scope: DeclarationScope::new(),
            structs: NamedItems::new(),
            enums: NamedItems::new(),
            traits: NamedItems::new(),
            impls: Vec::new(),
            externs: Vec::new(),
            functions: NamedItems::new(),
            function_sigs: NamedItems::new(),
            type_aliases: NamedItems::new(),
            errors: Vec::new(),
            function_type_vars: HashMap::new(),
            current_function: None,
            current_crate_name: None,
            current_qualified_module_prefix: None,
            current_module_path: PathBuf::new(),
            loaded_modules: HashSet::new(),
            loaded_module_paths: Vec::new(),
            import_aliases: HashMap::new(),
            explicit_import_aliases: HashMap::new(),
            current_trait: None,
            current_trait_id: None,
            current_trait_generics: Vec::new(),
            current_generic_owner: None,
            current_generic_params: Vec::new(),
            current_generic_kinds: Vec::new(),
            generic_contexts: HashMap::new(),
            item_resolution: ResolverTables::default(),
            current_module_id: None,
            infix_precedence: HashMap::new(),
            loaded_prelude_export_ids: HashMap::new(),
            module_file_cache: HashMap::new(),
            dependency_root_export_ids: HashMap::new(),
            language_items: LanguageItems::default(),
            canonical_import_aliases: HashMap::new(),
            canonical_names_by_id: HashMap::new(),
            scoped_module_aliases: HashMap::new(),
            export_aliases: HashMap::new(),
            export_function_aliases: HashMap::new(),
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
            explicit_import_aliases,
            current_trait: _,
            current_trait_id: _,
            current_trait_generics: _,
            current_generic_owner: _,
            current_generic_params: _,
            current_generic_kinds: _,
            generic_contexts,
            item_resolution,
            current_module_id: _,
            infix_precedence,
            loaded_prelude_export_ids,
            module_file_cache,
            dependency_root_export_ids,
            language_items,
            canonical_import_aliases,
            canonical_names_by_id,
            scoped_module_aliases,
            export_aliases,
            export_function_aliases,
        } = self;

        let mut explicit_import_aliases = explicit_import_aliases;
        super::canonicalize_import_alias_targets(
            &mut explicit_import_aliases,
            &export_aliases,
            current_crate_name,
        );
        let mut resolver = build_resolver_tables(
            item_index,
            root_module_id,
            current_crate_name,
            &explicit_import_aliases,
            &export_aliases,
            &export_function_aliases,
        );
        resolver.generic_contexts = generic_contexts;
        resolver.associated_contexts = item_resolution.associated_contexts;
        resolver.member_contexts = item_resolution.member_contexts;
        resolver.variant_contexts = item_resolution.variant_contexts;
        resolver.definition_environments = item_resolution.definition_environments;
        resolver.binding_aliases = item_resolution.binding_aliases;
        resolver.binding_exports = item_resolution.binding_exports;
        for (path, id) in &resolver.binding_aliases {
            if path.iter().all(|(_, context)| *context == SyntaxContext::ROOT) {
                resolver.import_aliases.entry(path.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>().join("::")).or_insert(*id);
            }
        }
        for (alias, def_id) in canonical_import_aliases {
            resolver.import_aliases.entry(alias).or_insert(def_id);
        }
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

    fn contextual_generic_type(&mut self, name: &str, context: SyntaxContext, span: Span) -> Type {
        let Some(owner) = self.current_generic_owner else {
            self.push_error_with_span(
                format!(
                    "generic type parameter '{}' appears outside a generic declaration",
                    name
                ),
                span,
            );
            return Type::Error;
        };

        let Some(index) = self
            .current_generic_params
            .iter()
            .enumerate()
            .position(|(index, param)| {
                let declared = self.generic_context(GenericParamId { owner, index: index as u32 });
                param == name && (declared == context || (name == "Self" && self.member_context_matches(owner, context, declared)))
            })
            .or_else(|| {
                let mut chars = name.chars();
                let is_implicit_generic = matches!(chars.next(), Some(ch) if ch.is_ascii_uppercase())
                    && chars.next().is_none();
                if is_implicit_generic {
                    self.generic_contexts.insert(GenericParamId {
                        owner, index: self.current_generic_params.len() as u32,
                    }, context);
                    self.current_generic_params.push(name.to_string());
                    self.current_generic_kinds
                        .push(crate::type_services::kind::Kind::Type);
                    Some(self.current_generic_params.len() - 1)
                } else {
                    None
                }
            }) else {
            self.push_error_with_span(format!("unknown type '{}'", name), span);
            return Type::Error;
        };

        Type::Generic(GenericParamId {
            owner,
            index: index as u32,
        })
    }

    #[allow(dead_code)]
    pub(crate) fn push_generic_context(
        &mut self,
        owner: DefId,
        params: Vec<String>,
    ) -> (Option<DefId>, Vec<String>) {
        let kinds = vec![crate::type_services::kind::Kind::Type; params.len()];
        let (owner, params, _) = self.push_generic_context_with_kinds(owner, params, kinds);
        (owner, params)
    }

    pub(crate) fn push_generic_context_with_kinds(
        &mut self,
        owner: DefId,
        params: Vec<String>,
        kinds: Vec<crate::type_services::kind::Kind>,
    ) -> (
        Option<DefId>,
        Vec<String>,
        Vec<crate::type_services::kind::Kind>,
    ) {
        self.push_generic_context_with_kinds_at(owner, params, kinds, &[])
    }

    pub(crate) fn push_generic_context_with_kinds_at(
        &mut self,
        owner: DefId,
        params: Vec<String>,
        kinds: Vec<crate::type_services::kind::Kind>,
        spans: &[Span],
    ) -> (
        Option<DefId>,
        Vec<String>,
        Vec<crate::type_services::kind::Kind>,
    ) {
        debug_assert_eq!(params.len(), kinds.len());
        let mut seen = HashSet::new();
        for (index, param) in params.iter().enumerate() {
            let context = self.generic_context(GenericParamId { owner, index: index as u32 });
            if !seen.insert((param.clone(), context)) {
                if let Some(span) = spans.get(index).cloned() {
                    self.push_error_with_span(
                        format!("duplicate generic parameter '{}'", param),
                        span,
                    );
                } else {
                    self.push_error(format!("duplicate generic parameter '{}'", param));
                }
            }
        }
        let prev_owner = self.current_generic_owner;
        let prev_params = std::mem::replace(&mut self.current_generic_params, params);
        let prev_kinds = std::mem::replace(&mut self.current_generic_kinds, kinds);
        self.current_generic_owner = Some(owner);
        (prev_owner, prev_params, prev_kinds)
    }

    #[allow(dead_code)]
    pub(crate) fn pop_generic_context(
        &mut self,
        prev_owner: Option<DefId>,
        prev_params: Vec<String>,
    ) {
        self.current_generic_params = prev_params;
        self.current_generic_kinds =
            vec![crate::type_services::kind::Kind::Type; self.current_generic_params.len()];
        self.current_generic_owner = prev_owner;
    }

    pub(crate) fn pop_generic_context_with_kinds(
        &mut self,
        prev_owner: Option<DefId>,
        prev_params: Vec<String>,
        prev_kinds: Vec<crate::type_services::kind::Kind>,
    ) {
        self.current_generic_params = prev_params;
        self.current_generic_kinds = prev_kinds;
        self.current_generic_owner = prev_owner;
    }

    fn record_canonical_alias(&mut self, alias: String, export: &ArtifactExport) {
        let first_segment = export.source.split("::").next().unwrap_or(&export.source);
        if self
            .current_crate_name
            .as_deref()
            .is_some_and(|crate_name| crate_name == first_segment || !export.source.contains("::"))
        {
            return;
        }

        self.canonical_import_aliases
            .entry(alias)
            .or_insert(export.id);
        self.canonical_names_by_id
            .entry(export.id)
            .or_insert_with(|| export.source.clone());
    }

    fn record_canonical_item_name(&mut self, id: DefId, name: &str) {
        if let Some(existing_name) = self.canonical_names_by_id.get(&id) {
            if existing_name != name {
                self.push_error(format!(
                    "conflicting canonical declaration names for DefId {id:?}: '{existing_name}' and '{name}'"
                ));
            }
            return;
        }

        self.canonical_names_by_id.insert(id, name.to_string());
    }

    fn record_scoped_module_alias(
        &mut self,
        module_prefix: Option<&str>,
        alias: String,
        id: DefId,
    ) {
        let Some(module_prefix) = module_prefix.filter(|prefix| !prefix.is_empty()) else {
            return;
        };

        self.scoped_module_aliases
            .entry(module_prefix.to_string())
            .or_default()
            .insert(alias, id);
    }

    fn record_resolved_import_alias(
        &mut self,
        module_prefix: Option<&str>,
        short_name: &str,
        export: &ArtifactExport,
    ) {
        let resolver_alias = module_prefix.map_or_else(
            || short_name.to_string(),
            |prefix| format!("{}::{}", prefix, short_name),
        );
        self.record_canonical_alias(resolver_alias, export);
        self.record_scoped_module_alias(module_prefix, short_name.to_string(), export.id);
    }

    pub(crate) fn export_for_registered_item(
        &self,
        qualified_name: &str,
    ) -> Option<ArtifactExport> {
        if let Some(function) = self.functions.get(qualified_name) {
            return Some(ArtifactExport {
                source: qualified_name.to_string(),
                id: function.id,
            });
        }
        if let Some(strukt) = self.structs.get(qualified_name) {
            return Some(ArtifactExport {
                source: qualified_name.to_string(),
                id: strukt.id,
            });
        }
        if let Some(enum_) = self.enums.get(qualified_name) {
            return Some(ArtifactExport {
                source: qualified_name.to_string(),
                id: enum_.id,
            });
        }
        if let Some(alias) = self.type_aliases.get(qualified_name) {
            return Some(ArtifactExport {
                source: qualified_name.to_string(),
                id: alias.id,
            });
        }
        if let Some(trait_) = self.traits.get(qualified_name) {
            return Some(ArtifactExport {
                source: qualified_name.to_string(),
                id: trait_.id,
            });
        }
        self.externs
            .iter()
            .find(|ext| ext.name == qualified_name)
            .map(|ext| ArtifactExport {
                source: qualified_name.to_string(),
                id: ext.id,
            })
    }

    fn export_for_import_source(&self, qualified_name: &str) -> Option<ArtifactExport> {
        let mut parts = qualified_name.split("::");
        if let (Some(crate_name), Some(export_name), None) =
            (parts.next(), parts.next(), parts.next())
        {
            if let Some(export) = self
                .dependency_root_export_ids
                .get(crate_name)
                .and_then(|exports| exports.get(export_name))
            {
                return Some(export.clone());
            }
        }

        self.export_for_registered_item(qualified_name)
    }

    fn artifact_root_export_for_import(&self, qualified_name: &str) -> Option<ArtifactExport> {
        let mut parts = qualified_name.split("::");
        let (Some(crate_name), Some(export_name), None) =
            (parts.next(), parts.next(), parts.next())
        else {
            return None;
        };

        self.dependency_root_export_ids
            .get(crate_name)
            .and_then(|exports| exports.get(export_name))
            .cloned()
    }

    fn import_artifact_root_export(
        &mut self,
        short_name: String,
        qualified_name: String,
        export: ArtifactExport,
        is_explicit: bool,
        record_canonical_aliases: bool,
        canonical_prefix: Option<&str>,
    ) {
        let canonical_source = export.source.clone();

        if let Some(func) = self.functions.get(&canonical_source) {
            let param_types: Vec<Type> = func.params.iter().map(|param| param.ty.clone()).collect();
            let func_type = Type::function_with_safety(
                param_types,
                func.ret_type.clone(),
                crate::types::FunctionSafety::from_is_unsafe(func.is_unsafe),
            );
            self.scope.define(short_name.clone(), func_type, false);
            self.import_aliases
                .insert(short_name.clone(), canonical_source.clone());
            if is_explicit && record_canonical_aliases {
                self.explicit_import_aliases
                    .insert(short_name.clone(), canonical_source);
                self.record_canonical_alias(short_name.clone(), &export);
            }
            self.record_resolved_import_alias(canonical_prefix, &short_name, &export);
            return;
        }

        if let Some(binding) = self.scope.lookup(&canonical_source) {
            self.scope
                .define(short_name.clone(), binding.ty.clone(), false);
            self.import_aliases
                .insert(short_name.clone(), canonical_source.clone());
            if is_explicit && record_canonical_aliases {
                self.explicit_import_aliases
                    .insert(short_name.clone(), canonical_source);
                self.record_canonical_alias(short_name.clone(), &export);
            }
            self.record_resolved_import_alias(canonical_prefix, &short_name, &export);
            return;
        }

        if self.structs.contains_key(&canonical_source) || self.type_aliases.contains_key(&canonical_source) {
            self.import_aliases
                .insert(short_name.clone(), canonical_source.clone());
            if is_explicit && record_canonical_aliases {
                self.explicit_import_aliases
                    .insert(short_name.clone(), canonical_source);
                self.record_canonical_alias(short_name.clone(), &export);
            }
            self.record_resolved_import_alias(canonical_prefix, &short_name, &export);
            return;
        }

        if self.enums.contains_key(&canonical_source) {
            self.import_aliases
                .insert(short_name.clone(), canonical_source.clone());
            if is_explicit && record_canonical_aliases {
                self.explicit_import_aliases
                    .insert(short_name.clone(), canonical_source);
                self.record_canonical_alias(short_name.clone(), &export);
            }
            self.record_resolved_import_alias(canonical_prefix, &short_name, &export);
            return;
        }

        if self.traits.contains_key(&canonical_source) {
            self.import_aliases
                .insert(short_name.clone(), canonical_source.clone());
            if is_explicit && record_canonical_aliases {
                self.explicit_import_aliases
                    .insert(short_name.clone(), canonical_source);
                self.record_canonical_alias(short_name.clone(), &export);
            }
            self.record_resolved_import_alias(canonical_prefix, &short_name, &export);
            return;
        }

        self.push_error(format!(
            "Failed to import '{}': canonical declaration '{}' not found",
            qualified_name, export.source
        ));
    }

    pub(crate) fn inject_loaded_prelude(&mut self, _ctx: &CrateContext) {
        if !self.loaded_prelude_export_ids.is_empty() {
            let exports: Vec<(String, ArtifactExport)> = self
                .loaded_prelude_export_ids
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();

            for (short_name, export) in exports {
                let qualified_name = export.source.clone();

                if self.inject_prelude_alias(short_name.clone(), &qualified_name) {
                    self.record_canonical_alias(short_name.clone(), &export);
                } else {
                    self.push_error(format!(
                        "Failed to inject std prelude alias '{}': declaration '{}' not found",
                        short_name, qualified_name
                    ));
                }
            }

            return;
        }
    }

    fn inject_prelude_alias(&mut self, short_name: String, qualified_name: &str) -> bool {
        if let Some(func) = self.functions.get(qualified_name) {
            let param_types: Vec<Type> = func.params.iter().map(|p| p.ty.clone()).collect();
            let func_type = Type::function_with_safety(
                param_types,
                func.ret_type.clone(),
                crate::types::FunctionSafety::from_is_unsafe(func.is_unsafe),
            );
            self.scope.define(short_name.clone(), func_type, false);
            self.import_aliases
                .entry(short_name.clone())
                .or_insert_with(|| qualified_name.to_string());
            true
        } else if let Some(binding) = self.scope.lookup(qualified_name) {
            self.scope
                .define(short_name.clone(), binding.ty.clone(), false);
            self.import_aliases
                .entry(short_name)
                .or_insert_with(|| qualified_name.to_string());
            true
        } else if self.structs.contains_key(qualified_name) {
            self.import_aliases
                .entry(short_name)
                .or_insert_with(|| qualified_name.to_string());
            true
        } else if self.enums.contains_key(qualified_name) {
            self.import_aliases
                .entry(short_name)
                .or_insert_with(|| qualified_name.to_string());
            true
        } else if self.traits.contains_key(qualified_name) {
            self.import_aliases
                .entry(short_name)
                .or_insert_with(|| qualified_name.to_string());
            true
        } else {
            false
        }
    }

    pub(crate) fn register_crate_functions(&mut self, ctx: &CrateContext) {
        for dep in ctx.extern_crates() {
            if ctx.is_current_source_self_artifact(self.current_crate_name.as_deref(), dep.name()) {
                continue;
            }
            self.register_extern_crate(dep);
        }
    }

    pub(crate) fn register_extern_crate(&mut self, dep: ExternCrateRef<'_>) {
        let crate_name = dep.name();
        let metadata = dep.metadata();
        let interface = metadata.interface();

        for (id, name) in &interface.canonical_names {
            self.record_canonical_item_name(*id, name);
        }

        // Nested public facades resolve to the same dependency declarations as
        // their canonical paths, including while collecting function signatures.
        for (alias, id) in &metadata.resolver().export_aliases {
            if let Some(source) = metadata.resolver().canonical_name(*id) {
                self.export_aliases.insert(alias.clone(), source.to_string());
            }
        }

        if !dep.root_exports().is_empty() {
            self.dependency_root_export_ids.insert(
                crate_name.to_string(),
                dep.root_exports()
                    .iter()
                    .map(|(name, export)| (name.clone(), export.clone()))
                    .collect(),
            );
        }

        if crate_name == "std" && !dep.prelude_exports().is_empty() {
            self.record_loaded_prelude_exports(
                dep.prelude_exports()
                    .iter()
                    .map(|(name, export)| (name.clone(), export.clone())),
            );
        }

        for (name, func) in interface.function_items() {
            let param_types: Vec<Type> = func.params.iter().map(|param| param.ty.clone()).collect();
            let func_type = Type::function_with_safety(
                param_types,
                func.ret_type.clone(),
                crate::types::FunctionSafety::from_is_unsafe(func.is_unsafe),
            );
            self.scope.define(name.clone(), func_type, false);
            self.record_canonical_item_name(func.id, &name);
            self.functions.insert(name.clone(), func.clone());
        }

        for ext in interface.extern_items() {
            let func_type = Type::function_with_safety(
                ext.params.clone(),
                ext.ret.clone(),
                crate::types::FunctionSafety::from_is_unsafe(ext.is_unsafe),
            );
            self.scope.define(ext.name.clone(), func_type, false);
            self.record_canonical_item_name(ext.id, &ext.name);
            self.externs.push(ext.clone());
        }

        for (name, strukt) in interface.struct_items() {
            self.record_canonical_item_name(strukt.id, &name);
            self.structs.insert(name.clone(), strukt.clone());
        }

        for (name, enum_) in interface.enum_items() {
            self.record_canonical_item_name(enum_.id, &name);
            self.enums.insert(name.clone(), enum_.clone());
        }

        for (name, alias) in interface.type_alias_items() {
            self.record_canonical_item_name(alias.id, &name);
            self.type_aliases.insert(name, alias);
        }

        for (name, trait_) in interface.trait_items() {
            self.record_canonical_item_name(trait_.id, &name);
            self.traits.insert(name.clone(), trait_.clone());
        }

        for imp in interface.impl_items() {
            self.impls.push(imp.clone());
        }

        for (name, precedence) in &interface.infix_precedence {
            self.infix_precedence.insert(name.clone(), *precedence);
        }
    }

    pub(crate) fn current_module_prefix(&self) -> Option<String> {
        if let Some(prefix) = &self.current_qualified_module_prefix {
            return Some(prefix.clone());
        }

        self.loaded_module_paths
            .iter()
            .find(|(_, path)| path == &self.current_module_path)
            .map(|(name, _)| name.clone())
            .or_else(|| self.current_crate_name.clone())
    }

    fn qualify_import_source(&self, source: &str, canonical_prefix: Option<&str>) -> String {
        if let Some(crate_name) = &self.current_crate_name {
            if let Some(stripped) = source.strip_prefix(&format!("{}::", crate_name)) {
                return stripped.to_string();
            }
        }

        let first_segment = source.split("::").next().unwrap_or(source);
        let is_absolute = self
            .loaded_module_paths
            .iter()
            .any(|(name, _)| name == first_segment)
            || canonical_prefix.is_some_and(|name| name == first_segment);

        if is_absolute {
            source.to_string()
        } else if let Some(prefix) = canonical_prefix {
            format!("{}::{}", prefix, source)
        } else {
            source.to_string()
        }
    }

    pub(crate) fn build_self_param_at(
        &mut self,
        self_receiver: ast::SelfReceiverMode,
        span: crate::lexer::Span,
    ) -> HirParam {
        HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: self.receiver_ty_for_self(self_receiver, span),
            mutable: matches!(self_receiver, ast::SelfReceiverMode::Mut),
            is_ref: false,
        }
    }

    pub(crate) fn receiver_ty_for_self(
        &mut self,
        self_receiver: ast::SelfReceiverMode,
        span: crate::lexer::Span,
    ) -> Type {
        let base = if let Some(index) = self
            .current_generic_params
            .iter()
            .position(|param| param == "Self")
        {
            self.current_generic_owner
                .map(|owner| {
                    Type::Generic(GenericParamId {
                        owner,
                        index: index as u32,
                    })
                })
                .unwrap_or_else(|| self.type_vars.fresh_type_var_at(span.clone()))
        } else {
            self.type_vars.fresh_type_var_at(span)
        };

        match self_receiver {
            ast::SelfReceiverMode::Shared => Type::Reference {
                mutable: false,
                inner: Box::new(base),
            },
            ast::SelfReceiverMode::Mut => Type::Reference {
                mutable: true,
                inner: Box::new(base),
            },
            ast::SelfReceiverMode::Move => base,
        }
    }

    pub(crate) fn lower_param_pattern_at(
        &mut self,
        pattern: &ast::Pattern,
        fallback: crate::lexer::Span,
    ) -> (String, Type, bool, bool) {
        match &pattern.kind {
            ast::PatternKind::Reference {
                pattern: inner,
                mutable,
            } => {
                let (name, ty, inner_mut, _) = self.lower_param_pattern_at(inner, fallback.clone());
                (name, ty, inner_mut || *mutable, true)
            }
            ast::PatternKind::Nested(inner) => self.lower_param_pattern_at(inner, fallback.clone()),
            ast::PatternKind::Ident(ident_pat) => {
                let ty = self
                    .type_vars
                    .fresh_type_var_at(ident_pat.name.span.clone());
                (ident_pat.name.name.clone(), ty, ident_pat.mut_, false)
            }
            ast::PatternKind::Wildcard => {
                let ty = self.type_vars.fresh_type_var_at(fallback.clone());
                ("_".to_string(), ty, false, false)
            }
            _ => {
                let ty = self.type_vars.fresh_type_var_at(fallback);
                let name = pattern
                    .binding
                    .as_ref()
                    .map(|binding| binding.name.clone())
                    .unwrap_or_else(|| "_arg".to_string());
                (name, ty, false, false)
            }
        }
    }

    pub(crate) fn lower_parse_type(&mut self, parse_type: &ast::ParseType) -> Type {
        TypeLowerer::lower_parse_type(self, parse_type)
    }

    pub(crate) fn impl_type_info(
        &mut self,
        imp: &ast::Impl,
    ) -> (String, Vec<String>, crate::hir::HirImplReceiverPattern) {
        let receiver = imp
            .for_
            .clone()
            .unwrap_or_else(|| imp.name.clone());
        let mut type_generics = Vec::new();
        collect_generic_names_from_parse_type(&receiver, &mut type_generics, &|name| {
            is_known_nominal_type_name(self, name)
        });
        let pattern = crate::hir::HirImplReceiverPattern::Exact(self.lower_parse_type(&receiver));
        (receiver.type_name(), type_generics, pattern)
    }

    pub(crate) fn handle_contextual_import(&mut self, path: &ast::Path) -> bool {
        let segments = match path { ast::Path::Ident(path) => &path.path, ast::Path::Type(path) => &path.path };
        if !self.has_contextual_import_path(segments) { return false; }
        let Some(names) = crate::collect::resolver::binding_path(segments) else { return false; };
        let prefix = self.current_module_id.and_then(|id| self.item_resolution.module_binding_paths.get(&id))
            .cloned().unwrap_or_default();
        if names.iter().chain(&prefix).all(|(_, context)| *context == SyntaxContext::ROOT) { return false; }
        let Some(alias) = names.last().cloned() else { return true; };
        let Some(last) = segments.last() else { return true; };
        let span = match last { ast::IdentOrType::Ident(name) => name.span.clone(), ast::IdentOrType::Type(ty) => ty.span() };
        let Some(id) = self.item_resolution.resolve_binding_path(self.current_module_id, &names) else {
            if !self.item_resolution.resolves_macro_directive(self.current_module_id, &names) {
                self.push_error_with_span(format!("unknown contextual import '{}'", names.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>().join("::")), span);
            }
            return true;
        };
        let mut alias_path = prefix;
        alias_path.push(alias);
        if self.item_resolution.binding_aliases.get(&alias_path)
            .or_else(|| self.item_resolution.binding_paths.get(&alias_path)).is_some_and(|previous| *previous != id) {
            self.push_error_with_span("conflicting contextual import".into(), span);
        } else {
            self.item_resolution.binding_aliases.insert(alias_path, id);
        }
        true
    }

    pub(crate) fn has_contextual_import_path(&self, segments: &[ast::IdentOrType]) -> bool {
        self.item_resolution.contextual_scope(self.current_module_id)
            || crate::collect::resolver::binding_path(segments).is_some_and(|path| path.iter().any(|(_, context)| *context != SyntaxContext::ROOT))
    }

    pub(crate) fn handled_binding_glob(&self, segments: &[ast::IdentOrType]) -> bool {
        self.has_contextual_import_path(segments) || crate::collect::resolver::binding_path(segments)
            .is_some_and(|path| self.item_resolution.indexed_module_path(self.current_module_id, &path))
    }

    pub(crate) fn handle_import(
        &mut self,
        path: &ast::Path,
        record_canonical_aliases: bool,
        canonical_prefix: Option<&str>,
    ) {
        if self.handle_contextual_import(path) { return; }
        let names = match path {
            ast::Path::Ident(path) => path_names(&path.path),
            ast::Path::Type(path) => path_names(&path.path),
        };

        if names.is_empty() {
            return;
        }

        let qualified_name = names.join("::");
        let short_name = names.last().cloned().unwrap_or_default();
        self.import_qualified_name(
            short_name,
            qualified_name,
            true,
            record_canonical_aliases,
            canonical_prefix,
        );
    }

    pub(crate) fn import_qualified_name(
        &mut self,
        short_name: String,
        qualified_name: String,
        is_explicit: bool,
        record_canonical_aliases: bool,
        canonical_prefix: Option<&str>,
    ) {
        let canonical_qualified_name = canonical_prefix
            .map(|prefix| self.qualify_import_source(&qualified_name, Some(prefix)))
            .or_else(|| {
                record_canonical_aliases
                    .then(|| self.qualify_import_source(&qualified_name, canonical_prefix))
            });

        if let Some(export) = self.artifact_root_export_for_import(&qualified_name) {
            self.import_artifact_root_export(
                short_name,
                qualified_name,
                export,
                is_explicit,
                record_canonical_aliases,
                canonical_prefix,
            );
            return;
        }

        if let Some(export_source) = canonical_qualified_name
            .as_ref()
            .and_then(|name| self.export_aliases.get(name))
            .or_else(|| self.export_aliases.get(&qualified_name))
            .cloned()
        {
            if let Some(export) = self.export_for_registered_item(&export_source) {
                self.import_artifact_root_export(
                    short_name,
                    qualified_name,
                    export,
                    is_explicit,
                    record_canonical_aliases,
                    canonical_prefix,
                );
                return;
            }
        }

        if let Some(func) = self.functions.get(&qualified_name) {
            let param_types: Vec<Type> = func.params.iter().map(|param| param.ty.clone()).collect();
            let func_type = Type::function_with_safety(
                param_types,
                func.ret_type.clone(),
                crate::types::FunctionSafety::from_is_unsafe(func.is_unsafe),
            );
            self.scope.define(short_name.clone(), func_type, false);
            self.import_aliases
                .insert(short_name.clone(), qualified_name.clone());
            if is_explicit && record_canonical_aliases {
                self.explicit_import_aliases
                    .insert(short_name.clone(), qualified_name.clone());
                if let Some(export) = self.export_for_import_source(&qualified_name) {
                    self.record_canonical_alias(short_name.clone(), &export);
                }
            }
            if let Some(export) = self.export_for_import_source(&qualified_name) {
                self.record_resolved_import_alias(canonical_prefix, &short_name, &export);
            }
            return;
        }

        if let Some(binding) = self.scope.lookup(&qualified_name) {
            self.scope
                .define(short_name.clone(), binding.ty.clone(), false);
            self.import_aliases
                .insert(short_name.clone(), qualified_name.clone());
            if is_explicit && record_canonical_aliases {
                self.explicit_import_aliases
                    .insert(short_name.clone(), qualified_name.clone());
                if let Some(export) = self.export_for_import_source(&qualified_name) {
                    self.record_canonical_alias(short_name.clone(), &export);
                }
            }
            if let Some(export) = self.export_for_import_source(&qualified_name) {
                self.record_resolved_import_alias(canonical_prefix, &short_name, &export);
            }
            return;
        }

        if self.structs.contains_key(&qualified_name) {
            self.import_aliases
                .insert(short_name.clone(), qualified_name.clone());
            if is_explicit && record_canonical_aliases {
                self.explicit_import_aliases
                    .insert(short_name.clone(), qualified_name.clone());
                if let Some(export) = self.export_for_import_source(&qualified_name) {
                    self.record_canonical_alias(short_name.clone(), &export);
                }
            }
            if let Some(export) = self.export_for_import_source(&qualified_name) {
                self.record_resolved_import_alias(canonical_prefix, &short_name, &export);
            }
            return;
        }

        if self.enums.contains_key(&qualified_name) {
            self.import_aliases
                .insert(short_name.clone(), qualified_name.clone());
            if is_explicit && record_canonical_aliases {
                self.explicit_import_aliases
                    .insert(short_name.clone(), qualified_name.clone());
                if let Some(export) = self.export_for_import_source(&qualified_name) {
                    self.record_canonical_alias(short_name.clone(), &export);
                }
            }
            if let Some(export) = self.export_for_import_source(&qualified_name) {
                self.record_resolved_import_alias(canonical_prefix, &short_name, &export);
            }
            return;
        }

        if self.type_aliases.contains_key(&qualified_name) {
            self.import_aliases
                .insert(short_name.clone(), qualified_name.clone());
            if is_explicit && record_canonical_aliases {
                self.explicit_import_aliases
                    .insert(short_name.clone(), qualified_name.clone());
                if let Some(export) = self.export_for_import_source(&qualified_name) {
                    self.record_canonical_alias(short_name.clone(), &export);
                }
            }
            if let Some(export) = self.export_for_import_source(&qualified_name) {
                self.record_resolved_import_alias(canonical_prefix, &short_name, &export);
            }
            return;
        }

        if self.traits.contains_key(&qualified_name) {
            self.import_aliases
                .insert(short_name.clone(), qualified_name.clone());
            if is_explicit && record_canonical_aliases {
                self.explicit_import_aliases
                    .insert(short_name.clone(), qualified_name.clone());
                if let Some(export) = self.export_for_import_source(&qualified_name) {
                    self.record_canonical_alias(short_name.clone(), &export);
                }
            }
            if let Some(export) = self.export_for_import_source(&qualified_name) {
                self.record_resolved_import_alias(canonical_prefix, &short_name, &export);
            }
            return;
        }

        if let Some(canonical_qualified_name) = canonical_qualified_name {
            if let Some(func) = self.functions.get(&canonical_qualified_name) {
                let param_types: Vec<Type> =
                    func.params.iter().map(|param| param.ty.clone()).collect();
                let func_type = Type::function_with_safety(
                    param_types,
                    func.ret_type.clone(),
                    crate::types::FunctionSafety::from_is_unsafe(func.is_unsafe),
                );
                self.scope.define(short_name.clone(), func_type, false);
                self.import_aliases
                    .insert(short_name.clone(), qualified_name.clone());
                if is_explicit {
                    self.explicit_import_aliases
                        .insert(short_name.clone(), canonical_qualified_name.clone());
                    if let Some(export) = self.export_for_registered_item(&canonical_qualified_name)
                    {
                        self.record_canonical_alias(short_name.clone(), &export);
                    }
                }
                if let Some(export) = self.export_for_registered_item(&canonical_qualified_name) {
                    self.record_resolved_import_alias(canonical_prefix, &short_name, &export);
                }
                return;
            }

            if let Some(binding) = self.scope.lookup(&canonical_qualified_name) {
                self.scope
                    .define(short_name.clone(), binding.ty.clone(), false);
                self.import_aliases
                    .insert(short_name.clone(), qualified_name.clone());
                if is_explicit {
                    self.explicit_import_aliases
                        .insert(short_name.clone(), canonical_qualified_name.clone());
                    if let Some(export) = self.export_for_registered_item(&canonical_qualified_name)
                    {
                        self.record_canonical_alias(short_name.clone(), &export);
                    }
                }
                if let Some(export) = self.export_for_registered_item(&canonical_qualified_name) {
                    self.record_resolved_import_alias(canonical_prefix, &short_name, &export);
                }
                return;
            }

            if self.structs.contains_key(&canonical_qualified_name) {
                self.import_aliases
                    .insert(short_name.clone(), canonical_qualified_name.clone());
                if is_explicit {
                    self.explicit_import_aliases
                        .insert(short_name.clone(), canonical_qualified_name.clone());
                    if let Some(export) = self.export_for_registered_item(&canonical_qualified_name)
                    {
                        self.record_canonical_alias(short_name.clone(), &export);
                    }
                }
                if let Some(export) = self.export_for_registered_item(&canonical_qualified_name) {
                    self.record_resolved_import_alias(canonical_prefix, &short_name, &export);
                }
                return;
            }

            if self.enums.contains_key(&canonical_qualified_name) {
                self.import_aliases
                    .insert(short_name.clone(), canonical_qualified_name.clone());
                if is_explicit {
                    self.explicit_import_aliases
                        .insert(short_name.clone(), canonical_qualified_name.clone());
                    if let Some(export) = self.export_for_registered_item(&canonical_qualified_name)
                    {
                        self.record_canonical_alias(short_name.clone(), &export);
                    }
                }
                if let Some(export) = self.export_for_registered_item(&canonical_qualified_name) {
                    self.record_resolved_import_alias(canonical_prefix, &short_name, &export);
                }
                return;
            }

            if self.type_aliases.contains_key(&canonical_qualified_name) {
                self.import_aliases
                    .insert(short_name.clone(), canonical_qualified_name.clone());
                if is_explicit {
                    self.explicit_import_aliases
                        .insert(short_name.clone(), canonical_qualified_name.clone());
                    if let Some(export) = self.export_for_registered_item(&canonical_qualified_name)
                    {
                        self.record_canonical_alias(short_name.clone(), &export);
                    }
                }
                if let Some(export) = self.export_for_registered_item(&canonical_qualified_name) {
                    self.record_resolved_import_alias(canonical_prefix, &short_name, &export);
                }
                return;
            }

            if self.traits.contains_key(&canonical_qualified_name) {
                self.import_aliases
                    .insert(short_name.clone(), canonical_qualified_name.clone());
                if is_explicit {
                    self.explicit_import_aliases
                        .insert(short_name.clone(), canonical_qualified_name.clone());
                    if let Some(export) = self.export_for_registered_item(&canonical_qualified_name)
                    {
                        self.record_canonical_alias(short_name.clone(), &export);
                    }
                }
                if let Some(export) = self.export_for_registered_item(&canonical_qualified_name) {
                    self.record_resolved_import_alias(canonical_prefix, &short_name, &export);
                }
            }
        }
    }

    pub(crate) fn handle_glob_import(&mut self, module_path: &[String]) {
        let module_label = format!("{}::*", module_path.join("::"));
        let canonical_prefix = self.current_module_prefix();
        match self.glob_import_targets(module_path) {
            Ok(targets) => {
                for (short_name, qualified_name) in targets {
                    if module_path.len() == 1 {
                        if let Some(export) = self
                            .dependency_root_export_ids
                            .get(&module_path[0])
                            .and_then(|exports| exports.get(&short_name))
                            .cloned()
                        {
                            self.record_canonical_alias(short_name.clone(), &export);
                        }
                    }
                    self.import_qualified_name(
                        short_name,
                        qualified_name,
                        false,
                        false,
                        canonical_prefix.as_deref(),
                    );
                }
            }
            Err(err) => self.push_error(format!("Failed to import {}: {}", module_label, err)),
        }
    }

    pub(crate) fn glob_import_targets(
        &mut self,
        module_path: &[String],
    ) -> Result<Vec<(String, String)>, String> {
        if module_path.len() == 1 {
            if let Some(exports) = self.dependency_root_export_ids.get(&module_path[0]) {
                return Ok(exports
                    .iter()
                    .map(|(name, export)| (name.clone(), export.source.clone()))
                    .collect());
            }
        }

        let (module, resolved_prefix) = self.load_module_by_path(module_path)?;
        let exports =
            self.expand_glob_exports_with_prefix(&resolved_prefix, collect_exports(&module));
        let mut targets = Vec::new();

        for (name, source) in exports {
            if name.ends_with("::*") {
                continue;
            }

            let qualified_name = source.map_or_else(
                || format!("{}::{}", resolved_prefix, name),
                |source| {
                    let first_segment = source.split("::").next().unwrap_or(source.as_str());
                    let is_absolute = self
                        .loaded_module_paths
                        .iter()
                        .any(|(crate_name, _)| crate_name == first_segment);

                    if is_absolute {
                        source
                    } else {
                        format!("{}::{}", resolved_prefix, source)
                    }
                },
            );
            targets.push((name, qualified_name));
        }

        Ok(targets)
    }

    pub(crate) fn load_module_by_path(
        &mut self,
        module_path: &[String],
    ) -> Result<(ast::Module, String), String> {
        if module_path.is_empty() {
            return Err("Empty module path".to_string());
        }

        let is_external_crate = self
            .loaded_module_paths
            .iter()
            .any(|(name, _)| name == &module_path[0]);

        let (mut module, mut resolved_prefix, start_index) = if is_external_crate {
            let crate_name = &module_path[0];
            if module_path.len() == 1 {
                let lib_path = self
                    .loaded_module_paths
                    .iter()
                    .find(|(name, _)| name == crate_name)
                    .map(|(_, path)| path.clone())
                    .ok_or_else(|| format!("Unknown crate {}", crate_name))?;
                let canonical_path = lib_path.canonicalize().unwrap_or_else(|_| lib_path.clone());
                let module = self
                    .module_file_cache
                    .get(&canonical_path)
                    .or_else(|| self.module_file_cache.get(&lib_path))
                    .cloned()
                    .ok_or_else(|| {
                        format!(
                            "Module '{}' was not loaded by the source database; expected {}",
                            crate_name,
                            lib_path.display()
                        )
                    })?;
                (module, crate_name.clone(), 1)
            } else {
                let module_name = &module_path[1];
                let module = self.load_external_crate_module(module_name, crate_name)?;
                (module, format!("{}::{}", crate_name, module_name), 2)
            }
        } else {
            let module_name = &module_path[0];
            let resolved_prefix = self
                .current_module_prefix()
                .map(|prefix| format!("{}::{}", prefix, module_name))
                .unwrap_or_else(|| module_name.clone());
            let module = self.load_module(module_name)?;
            (module, resolved_prefix, 1)
        };

        for segment in &module_path[start_index..] {
            let next_prefix = format!("{}::{}", resolved_prefix, segment);
            if let Some(loaded_module) = self.cached_module_for_qualified_name(&next_prefix) {
                module = loaded_module;
                resolved_prefix = next_prefix;
                continue;
            }

            let inline_module = find_inline_module(&module, segment)
                .ok_or_else(|| format!("Module {} not found in {}", segment, resolved_prefix))?;
            module = inline_module.clone();
            resolved_prefix = next_prefix;
        }

        Ok((module, resolved_prefix))
    }

    pub(crate) fn load_module(&mut self, module_name: &str) -> Result<ast::Module, String> {
        let prefix = self.current_module_prefix();
        self.load_module_with_prefix(module_name, prefix.as_deref())
    }

    pub(crate) fn load_module_with_prefix(
        &mut self,
        module_name: &str,
        prefix: Option<&str>,
    ) -> Result<ast::Module, String> {
        if let Some(file_path) = self.loaded_module_path_for_prefix(module_name, prefix) {
            if let Some(cached) = self.cached_module_for_path(&file_path) {
                return Ok(cached);
            }
        }

        let display_name = prefix
            .map(|prefix| format!("{}::{}", prefix, module_name))
            .unwrap_or_else(|| module_name.to_string());
        Err(format!(
            "Module '{}' was not loaded by the source database",
            display_name
        ))
    }

    pub(crate) fn load_external_crate_module(
        &mut self,
        module_name: &str,
        crate_name: &str,
    ) -> Result<ast::Module, String> {
        let qualified_module_name = format!("{}::{}", crate_name, module_name);
        if let Some(file_path) = self
            .loaded_module_paths
            .iter()
            .find(|(name, _)| name == &qualified_module_name)
            .map(|(_, path)| path.clone())
        {
            if let Some(cached) = self.cached_module_for_path(&file_path) {
                return Ok(cached);
            }
        }

        Err(format!(
            "Module '{}' was not loaded by the source database",
            qualified_module_name
        ))
    }

    pub(crate) fn expand_glob_exports(
        &mut self,
        exports: HashMap<String, Option<String>>,
    ) -> HashMap<String, Option<String>> {
        let mut expanded = HashMap::new();

        for (key, value) in &exports {
            if let Some(module_prefix) = key.strip_suffix("::*") {
                let segments: Vec<String> =
                    module_prefix.split("::").map(ToString::to_string).collect();
                match self.load_module_by_path(&segments) {
                    Ok((sub_module, resolved_prefix)) => {
                        let sub_exports = self.expand_glob_exports_with_prefix(
                            &resolved_prefix,
                            collect_exports(&sub_module),
                        );
                        for (name, source) in sub_exports {
                            if name.ends_with("::*") {
                                continue;
                            }

                            let qualified =
                                source.unwrap_or_else(|| format!("{}::{}", resolved_prefix, name));
                            expanded.insert(name, Some(qualified));
                        }
                    }
                    Err(err) => self.push_error(format!(
                        "Failed to expand glob export {}::*: {}",
                        module_prefix, err
                    )),
                }
            } else {
                expanded.insert(key.clone(), value.clone());
            }
        }

        expanded
    }

    pub(crate) fn expand_glob_exports_with_prefix(
        &mut self,
        prefix: &str,
        exports: HashMap<String, Option<String>>,
    ) -> HashMap<String, Option<String>> {
        let previous_prefix = self
            .current_qualified_module_prefix
            .replace(prefix.to_string());
        let expanded = self.expand_glob_exports(exports);
        self.current_qualified_module_prefix = previous_prefix;
        expanded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ast::{Ident, Module, ParseType, ParseTypeInner};
    use crate::hir::HirAssociatedTypeDecl;
    use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId};
    use crate::lexer::Span;
    use crate::types::GenericParamDecl;

    fn named_type(name: &str) -> ParseType {
        ParseType::Type(ParseTypeInner { context: Default::default(),
            name: name.to_string(),
            generics: vec![],
            span: Span::test(),
        })
    }

    fn def_id(index: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(index))
    }

    #[test]
    fn macro_a_14_definition_scope_sees_canonical_metadata_aliases_without_source_shadow_leaks() {
        for source in ["main = -> 1\n", "lift = -> true\nmain = -> 1\n"] {
            let program = crate::parser::parse_string(source, &crate::Config::default()).unwrap();
            let mut ids = crate::collect::item_index::IndexingIds::new_root();
            let index = crate::collect::item_index::index_root_module_items_with_sources(&mut ids, &program.module, None, &HashMap::new());
            let mut context = CollectContext::new();
            let external = DefId::new(CrateId(7), LocalDefId(1));
            context.canonical_import_aliases.insert("lift".into(), external);
            context.seed_item_resolution(&index, ids.root_module_id());
            let fresh = SyntaxContext::allocated(87);
            let mut environments = crate::macro_expansion::binding_environment::BindingEnvironments::default();
            environments.record_source_scope(fresh, Vec::new(), SyntaxContext::ROOT).unwrap();
            context.item_resolution.definition_environments = environments.resolve(&CrateContext::new()).unwrap();
            let selected = context.item_resolution.resolve_binding_path(None, &[("lift".into(), fresh)]).unwrap();
            if source.starts_with("lift") {
                assert_ne!(selected, external);
            } else {
                assert_eq!(selected, external);
            }
        }
    }

    #[test]
    fn load_external_crate_module_prefers_seeded_module_path() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_collect_external_dir_module_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(temp_dir.join("util")).unwrap();
        let dep_root = temp_dir.join("lib.rk");
        let util_mod = temp_dir.join("util/mod.rk");
        std::fs::write(&dep_root, "mod util\n").unwrap();
        std::fs::write(&util_mod, "answer = -> 42\n").unwrap();

        let module = Module {
            name: Some(Ident { context: Default::default(),
                name: "util".to_string(),
                span: Span::test(),
            }),
            top_levels: vec![],
            is_inline: false,
            filepath: Some(util_mod.clone()),
        };
        let mut context = CollectContext::new();
        context
            .loaded_module_paths
            .push(("dep".to_string(), dep_root.clone()));
        context
            .loaded_module_paths
            .push(("dep::util".to_string(), util_mod.clone()));
        context.module_file_cache.insert(util_mod.clone(), module);

        let loaded = context
            .load_external_crate_module("util", "dep")
            .expect("external module loading should use graph-seeded util/mod.rk path");

        assert_eq!(loaded.filepath.as_ref(), Some(&util_mod));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn load_module_requires_seeded_module_path() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_collect_requires_seeded_module_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let root = temp_dir.join("main.rk");
        let util = temp_dir.join("util.rk");

        let module = Module {
            name: Some(Ident { context: Default::default(),
                name: "util".to_string(),
                span: Span::test(),
            }),
            top_levels: vec![],
            is_inline: false,
            filepath: Some(util.clone()),
        };
        let mut context = CollectContext::new();
        context.current_module_path = root;
        context.module_file_cache.insert(util, module);

        let error = context
            .load_module("util")
            .expect_err("module cache should not bypass graph-seeded module paths");

        assert!(error.contains("was not loaded by the source database"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn load_module_with_prefix_does_not_fallback_to_unqualified_module() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_collect_no_unqualified_fallback_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let top_level_io = temp_dir.join("io.rk");

        let module = Module {
            name: Some(Ident { context: Default::default(),
                name: "io".to_string(),
                span: Span::test(),
            }),
            top_levels: vec![],
            is_inline: false,
            filepath: Some(top_level_io.clone()),
        };
        let mut context = CollectContext::new();
        context
            .loaded_module_paths
            .push(("io".to_string(), top_level_io.clone()));
        context.module_file_cache.insert(top_level_io, module);

        let error = context
            .load_module_with_prefix("io", Some("math"))
            .expect_err("prefixed lookup must not fall back to top-level io");

        assert!(error.contains("math::io"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn load_module_with_prefix_prefers_current_crate_prefixed_graph_path() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_collect_prefers_current_crate_graph_path_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let alias_io = temp_dir.join("alias_io.rk");
        let graph_io = temp_dir.join("graph_io.rk");

        let alias_module = Module {
            name: Some(Ident { context: Default::default(),
                name: "io".to_string(),
                span: Span::test(),
            }),
            top_levels: vec![],
            is_inline: false,
            filepath: Some(alias_io.clone()),
        };
        let graph_module = Module {
            name: Some(Ident { context: Default::default(),
                name: "io".to_string(),
                span: Span::test(),
            }),
            top_levels: vec![],
            is_inline: false,
            filepath: Some(graph_io.clone()),
        };
        let mut context = CollectContext::new();
        context.current_crate_name = Some("test".to_string());
        context
            .loaded_module_paths
            .push(("math::io".to_string(), alias_io.clone()));
        context
            .loaded_module_paths
            .push(("test::math::io".to_string(), graph_io.clone()));
        context.module_file_cache.insert(alias_io, alias_module);
        context
            .module_file_cache
            .insert(graph_io.clone(), graph_module);

        let module = context
            .load_module_with_prefix("io", Some("math"))
            .expect("current-crate-prefixed graph path should resolve");

        assert_eq!(module.filepath.as_ref(), Some(&graph_io));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn load_external_crate_module_requires_seeded_child_path() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_collect_requires_seeded_external_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let dep_root = temp_dir.join("lib.rk");
        let util = temp_dir.join("util.rk");

        let module = Module {
            name: Some(Ident { context: Default::default(),
                name: "util".to_string(),
                span: Span::test(),
            }),
            top_levels: vec![],
            is_inline: false,
            filepath: Some(util.clone()),
        };
        let mut context = CollectContext::new();
        context
            .loaded_module_paths
            .push(("dep".to_string(), dep_root));
        context.module_file_cache.insert(util, module);

        let error = context
            .load_external_crate_module("util", "dep")
            .expect_err("external module cache should not bypass graph-seeded module paths");

        assert!(error.contains("was not loaded by the source database"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn collect_context_lower_parse_type_reports_bare_slice_error() {
        let mut context = CollectContext::new();

        let ty = context.lower_parse_type(&ParseType::Slice(Box::new(named_type("I64"))));

        assert_eq!(ty, Type::Error);
        assert!(context
            .errors
            .iter()
            .any(|error| error.message.contains("bare slice type [T]")));
    }

    #[test]
    fn collect_context_lower_parse_type_creates_implicit_generic() {
        let owner = def_id(70);
        let mut context = CollectContext::new();
        context.current_generic_owner = Some(owner);

        let ty = context.lower_parse_type(&named_type("T"));

        assert_eq!(ty, Type::Generic(GenericParamId { owner, index: 0 }));
        assert_eq!(context.current_generic_params, vec!["T".to_string()]);
        assert!(context.errors.is_empty());
    }

    #[test]
    fn collect_context_unknown_generic_error_keeps_parse_span() {
        let owner = def_id(71);
        let span = Span::new("/virtual/types.rk".into(), 9, 10);
        let mut context = CollectContext::new();
        context.current_generic_owner = Some(owner);
        context.current_generic_params = vec!["U".to_string()];
        context.current_generic_kinds = vec![crate::type_services::kind::Kind::Type];

        let ty = context.lower_parse_type(&ParseType::Type(ParseTypeInner { context: Default::default(),
            name: "missing".to_string(),
            generics: Vec::new(),
            span: span.clone(),
        }));

        assert_eq!(ty, Type::Error);
        assert_eq!(context.errors[0].span(), Some(span));
    }

    #[test]
    fn collect_context_reports_duplicate_generic_names_before_lookup() {
        let owner = def_id(72);
        let mut context = CollectContext::new();

        context.push_generic_context(owner, vec!["T".to_string(), "T".to_string()]);

        assert!(context
            .errors
            .iter()
            .any(|error| { error.message.contains("duplicate generic parameter 'T'") }));
    }

    #[test]
    fn collect_context_nominal_type_does_not_resolve_by_suffix() {
        let struct_id = def_id(75);
        let mut context = CollectContext::new();
        context.structs.insert(
            "dep::Widget".to_string(),
            HirStruct {
                id: struct_id,
                name: "dep::Widget".to_string(),
                generic_params: Vec::new(),
                fields: Vec::new(),
            },
        );

        let ty = context.lower_parse_type(&named_type("Widget"));

        assert_eq!(ty, Type::Error);
        assert!(!context.errors.is_empty());
    }

    #[test]
    fn collect_context_lowers_associated_type_through_canonical_trait_import_id() {
        let trait_id = def_id(80);
        let assoc_type_id = AssocTypeId(3);
        let mut context = CollectContext::new();
        context.traits.insert(
            "dep::Iterable".to_string(),
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "dep::Iterable".to_string(),
                generic_params: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: trait_id,
                        index: 0,
                    },
                    "Self",
                )],
                associated_types: vec![HirAssociatedTypeDecl {
                    id: assoc_type_id,
                    name: "Item".to_string(),
                    kind: crate::type_services::kind::Kind::Type,
                }],
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        );
        context
            .canonical_import_aliases
            .insert("Iterable".to_string(), trait_id);
        context
            .canonical_names_by_id
            .insert(trait_id, "dep::Iterable".to_string());
        context.current_trait = Some("Iterable".to_string());
        context.current_generic_owner = Some(trait_id);
        context.current_generic_params = vec!["Self".to_string()];

        let ty = context.lower_parse_type(&ParseType::Associated {
            base: ParseTypeInner { context: Default::default(),
                name: "Self".to_string(),
                generics: vec![],
                span: Span::test(),
            },
            member: Ident { context: Default::default(),
                name: "Item".to_string(),
                span: Span::test(),
            },
        });

        assert!(matches!(ty, Type::Projection { trait_id: id, .. } if id == trait_id));
        assert!(context.errors.is_empty());
    }
}

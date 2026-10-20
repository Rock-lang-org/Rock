use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::collect::item_index::{ItemIndex, ItemKind, ModuleKind};
use crate::ids::{DefId, ModuleId};
use crate::lexer::lossless::SyntaxContext;
use crate::types::GenericParamId;

pub(crate) trait BindingSegment {
    fn binding_names(&self) -> Option<Vec<(String, SyntaxContext)>>;
}

impl BindingSegment for String {
    fn binding_names(&self) -> Option<Vec<(String, SyntaxContext)>> {
        Some(self.split("::").map(|name| (name.to_string(), SyntaxContext::ROOT)).collect())
    }
}

impl BindingSegment for crate::ast::IdentOrType {
    fn binding_names(&self) -> Option<Vec<(String, SyntaxContext)>> {
        match self {
            crate::ast::IdentOrType::Ident(name) => Some(vec![(name.name.clone(), name.context)]),
            crate::ast::IdentOrType::Type(ty) => ty.binding_names(),
        }
    }
}

impl BindingSegment for crate::ast::ParseType {
    fn binding_names(&self) -> Option<Vec<(String, SyntaxContext)>> {
        match self {
            crate::ast::ParseType::Associated { base, member } => {
                Some(vec![(base.name.clone(), base.context), (member.name.clone(), member.context)])
            }
            crate::ast::ParseType::Type(name) => Some(vec![(name.name.clone(), name.context)]),
            crate::ast::ParseType::Application(application) => application.constructor.binding_names(),
            _ => None,
        }
    }
}

pub(crate) fn binding_path<S: BindingSegment>(segments: &[S]) -> Option<Vec<(String, SyntaxContext)>> {
    Some(segments.iter().map(BindingSegment::binding_names).collect::<Option<Vec<_>>>()?.into_iter().flatten().collect())
}

pub(crate) fn remap_generic_contexts(
    contexts: &mut HashMap<GenericParamId, SyntaxContext>,
    params: &[GenericParamId],
    owner: DefId,
) {
    // Snapshot before publishing: a permutation can reuse its own source IDs.
    let remapped = params.iter().enumerate().map(|(index, source)| {
        let context = contexts.get(source).copied().unwrap_or(SyntaxContext::ROOT);
        (GenericParamId { owner, index: index as u32 }, context)
    }).collect::<Vec<_>>();
    contexts.extend(remapped);
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolverTables {
    // Compilation-local binding authority; never serialize context integers into artifacts.
    #[serde(skip)]
    pub(crate) generic_contexts: HashMap<GenericParamId, SyntaxContext>,
    #[serde(skip)]
    pub(crate) associated_contexts: HashMap<(DefId, crate::ids::AssocTypeId), SyntaxContext>,
    #[serde(skip)]
    pub(crate) member_contexts: HashMap<DefId, SyntaxContext>,
    #[serde(skip)]
    pub(crate) variant_contexts: HashMap<(DefId, crate::ids::VariantId), SyntaxContext>,
    #[serde(skip)]
    pub(crate) binding_paths: HashMap<Vec<(String, SyntaxContext)>, DefId>,
    #[serde(skip)]
    pub(crate) binding_aliases: HashMap<Vec<(String, SyntaxContext)>, DefId>,
    #[serde(skip)]
    pub(crate) binding_exports: HashMap<Vec<(String, SyntaxContext)>, DefId>,
    #[serde(skip)]
    pub(crate) macro_bindings: HashSet<Vec<(String, SyntaxContext)>>,
    #[serde(skip)]
    pub(crate) definition_binding_paths: HashMap<DefId, Vec<(String, SyntaxContext)>>,
    #[serde(skip)]
    pub(crate) signature_paths: HashMap<Vec<(String, SyntaxContext)>, DefId>,
    #[serde(skip)]
    pub(crate) module_binding_paths: HashMap<ModuleId, Vec<(String, SyntaxContext)>>,
    #[serde(skip)]
    pub(crate) definition_environments: HashMap<SyntaxContext, crate::macro_expansion::binding_environment::ResolvedDefinitionEnvironment>,
    pub module_paths: HashMap<String, ModuleId>,
    pub module_names_by_id: HashMap<ModuleId, String>,
    pub item_paths: HashMap<String, DefId>,
    pub item_names_by_id: HashMap<DefId, String>,
    pub import_aliases: HashMap<String, DefId>,
    pub export_aliases: HashMap<String, DefId>,
    pub scoped_module_aliases: HashMap<String, HashMap<String, DefId>>,
    pub module_aliases: HashMap<String, DefId>,
}

impl ResolverTables {
    pub(crate) fn source_lookup_scopes(&self, scope: Option<ModuleId>, context: SyntaxContext) -> Vec<(Vec<(String, SyntaxContext)>, SyntaxContext)> {
        let mut scope = scope.and_then(|id| self.module_binding_paths.get(&id)).cloned().unwrap_or_default();
        let mut context = context;
        let mut scopes = Vec::new();
        for _ in 0..=self.definition_environments.len() {
            scopes.push((scope, context));
            scopes.push((Vec::new(), context));
            let Some(crate::macro_expansion::binding_environment::ResolvedDefinitionEnvironment::SourceScope { path, context: original }) = self.definition_environments.get(&context) else { break; };
            scope = path.clone();
            context = *original;
        }
        scopes
    }

    pub(crate) fn member_context_matches(&self, owner: DefId, actual: SyntaxContext, declared: SyntaxContext) -> bool {
        use crate::macro_expansion::binding_environment::ResolvedDefinitionEnvironment;
        let mut actual = actual;
        for _ in 0..=self.definition_environments.len() {
            if actual == declared { return true; }
            match self.definition_environments.get(&actual) {
                Some(ResolvedDefinitionEnvironment::SourceScope { context, .. }) => actual = *context,
                Some(ResolvedDefinitionEnvironment::PublicDependency(exports)) => {
                    return declared == SyntaxContext::ROOT && exports.values().any(|id| *id == owner);
                }
                None => return false,
            }
        }
        false
    }

    /// Macro-only directives are checked by expansion, not assigned ordinary DefIds.
    pub(crate) fn resolves_macro_directive(&self, scope: Option<ModuleId>, path: &[(String, SyntaxContext)]) -> bool {
        let mut scope = scope.and_then(|id| self.module_binding_paths.get(&id)).cloned().unwrap_or_default();
        let mut path = path.to_vec();
        for _ in 0..=self.definition_environments.len() {
            let mut qualified = scope.clone();
            qualified.extend_from_slice(&path);
            if self.resolve_member_contexts_with(&qualified, |path| self.absolute_macro_binding(path)).is_some()
                || self.resolve_member_contexts_with(&path, |path| self.absolute_macro_binding(path)).is_some() { return true; }
            let Some((_, context)) = path.first() else { return false; };
            let Some(crate::macro_expansion::binding_environment::ResolvedDefinitionEnvironment::SourceScope { path: prefix, context: original }) = self.definition_environments.get(context) else { return false; };
            if path.iter().any(|(_, other)| *other != SyntaxContext::ROOT && other != context) { return false; }
            scope = prefix.clone();
            path = path.iter().map(|(name, current)| (name.clone(), if current == context { *original } else { *current })).collect();
        }
        false
    }

    fn absolute_macro_binding(&self, path: &[(String, SyntaxContext)]) -> Option<()> {
        if self.macro_bindings.contains(path) { return Some(()); }
        for split in (1..path.len()).rev() {
            let Some(id) = self.resolve_absolute_binding_path(&path[..split]) else { continue; };
            let Some(prefix) = self.definition_binding_paths.get(&id) else { continue; };
            let mut target = prefix.clone(); target.extend_from_slice(&path[split..]);
            if self.macro_bindings.contains(&target) { return Some(()); }
        }
        None
    }

    pub(crate) fn contextual_scope(&self, scope: Option<ModuleId>) -> bool {
        scope.and_then(|id| self.module_binding_paths.get(&id))
            .is_some_and(|path| path.iter().any(|(_, context)| *context != SyntaxContext::ROOT))
    }

    pub(crate) fn indexed_module_path(&self, scope: Option<ModuleId>, path: &[(String, SyntaxContext)]) -> bool {
        self.resolve_binding_path(scope, path).and_then(|id| self.definition_binding_paths.get(&id))
            .is_some_and(|path| self.module_binding_paths.values().any(|module| module == path))
    }

    pub(crate) fn resolve_binding_path(
        &self,
        scope: Option<ModuleId>,
        path: &[(String, SyntaxContext)],
    ) -> Option<DefId> {
        let mut scope = scope;
        let mut path = path.to_vec();
        for _ in 0..=self.definition_environments.len() {
            if let Some((_, context)) = path.first() {
                if let Some(environment) = self.definition_environments.get(context) {
                    use crate::macro_expansion::binding_environment::ResolvedDefinitionEnvironment;
                    // A definition path cannot merge unrelated contextual authorities.
                    if path.iter().any(|(_, other)| *other != SyntaxContext::ROOT && other != context) { return None; }
                    match environment {
                        ResolvedDefinitionEnvironment::SourceScope { path: prefix, context: original } => {
                            if let Some(id) = self.resolve_lexical_member_contexts(scope, &path) { return Some(id); }
                            let names = path.iter().map(|(name, current)| (name.clone(), if current == context { *original } else { *current })).collect::<Vec<_>>();
                            let mut qualified = prefix.clone();
                            qualified.extend(names.clone());
                            if let Some(id) = self.resolve_absolute_binding_path(&qualified) { return Some(id); }
                            path = names;
                            scope = None;
                            continue;
                        }
                        ResolvedDefinitionEnvironment::PublicDependency(exports) => {
                            let name = path.iter().skip(1).map(|(name, _)| name.as_str()).collect::<Vec<_>>().join("::");
                            return exports.get(&name).copied();
                        }
                    }
                }
            }
            return self.resolve_lexical_binding_path(scope, &path);
        }
        None
    }

    fn resolve_lexical_binding_path(&self, scope: Option<ModuleId>, path: &[(String, SyntaxContext)]) -> Option<DefId> {
        if let Some(prefix) = scope.and_then(|id| self.module_binding_paths.get(&id)) {
            let mut qualified = prefix.clone();
            qualified.extend_from_slice(path);
            if let Some(id) = self.resolve_absolute_binding_path(&qualified) { return Some(id); }
        }
        self.resolve_absolute_binding_path(path)
    }

    fn resolve_lexical_member_contexts(&self, scope: Option<ModuleId>, path: &[(String, SyntaxContext)]) -> Option<DefId> {
        if let Some(prefix) = scope.and_then(|id| self.module_binding_paths.get(&id)) {
            let mut qualified = prefix.clone(); qualified.extend_from_slice(path);
            if let Some(id) = self.resolve_member_contexts(&qualified) { return Some(id); }
        }
        self.resolve_member_contexts(path)
    }

    fn resolve_member_contexts(&self, path: &[(String, SyntaxContext)]) -> Option<DefId> {
        self.resolve_member_contexts_with(path, |path| self.resolve_absolute_binding_path(path))
    }

    fn resolve_member_contexts_with<T>(&self, path: &[(String, SyntaxContext)], lookup: impl Fn(&[(String, SyntaxContext)]) -> Option<T>) -> Option<T> {
        let mut path = path.to_vec();
        for _ in 0..=self.definition_environments.len().saturating_mul(path.len()) {
            if let Some(value) = lookup(&path) { return Some(value); }
            let split = (1..path.len()).rev().find(|split| self.resolve_absolute_binding_path(&path[..*split])
                .is_some_and(|id| self.module_binding_paths.values().any(|module| self.definition_binding_paths.get(&id) == Some(module))))?;
            let crate::macro_expansion::binding_environment::ResolvedDefinitionEnvironment::SourceScope { context, .. } = self.definition_environments.get(&path[split].1)? else { return None; };
            path[split].1 = *context;
        }
        None
    }

    fn resolve_absolute_binding_path(&self, path: &[(String, SyntaxContext)]) -> Option<DefId> {
        if let Some(id) = self.binding_aliases.get(path).or_else(|| self.binding_paths.get(path)) { return Some(*id); }
        // Module aliases carry the selected module identity, not its printed name.
        for split in (1..path.len()).rev() {
            let Some(id) = self.binding_aliases.get(&path[..split]) else { continue; };
            let Some(prefix) = self.definition_binding_paths.get(id) else { continue; };
            let mut target = prefix.clone();
            target.extend_from_slice(&path[split..]);
            if let Some(id) = self.binding_aliases.get(&target).or_else(|| self.binding_paths.get(&target)) { return Some(*id); }
        }
        None
    }

    pub(crate) fn generic_context(&self, id: GenericParamId) -> SyntaxContext {
        self.generic_contexts.get(&id).copied().unwrap_or(SyntaxContext::ROOT)
    }

    pub fn resolve_item_or_alias(&self, name: &str) -> Option<DefId> {
        self.item_paths
            .get(name)
            .copied()
            .or_else(|| self.import_aliases.get(name).copied())
            .or_else(|| self.export_aliases.get(name).copied())
            .or_else(|| self.module_aliases.get(name).copied())
    }

    pub fn canonical_name(&self, id: DefId) -> Option<&str> {
        self.item_names_by_id.get(&id).map(String::as_str)
    }

    pub fn resolve_module_local_alias(&self, module_path: &str, alias: &str) -> Option<DefId> {
        self.scoped_module_aliases
            .get(module_path)
            .and_then(|aliases| aliases.get(alias))
            .copied()
    }

    pub fn insert_import_alias_with_name(&mut self, alias: String, source: String, id: DefId) {
        self.import_aliases.insert(alias, id);
        self.item_names_by_id.entry(id).or_insert(source);
    }

    pub fn insert_export_alias_with_name(&mut self, alias: String, source: String, id: DefId) {
        self.export_aliases.insert(alias, id);
        self.item_names_by_id.entry(id).or_insert(source);
    }

    pub fn insert_module_alias_with_name(&mut self, alias: String, source: String, id: DefId) {
        self.module_aliases.insert(alias, id);
        self.item_names_by_id.entry(id).or_insert(source);
    }

    pub fn merge_global_inputs(&mut self, other: &ResolverTables) {
        self.generic_contexts.extend(other.generic_contexts.clone());
        self.associated_contexts.extend(other.associated_contexts.clone());
        self.member_contexts.extend(other.member_contexts.clone());
        self.variant_contexts.extend(other.variant_contexts.clone());
        self.binding_paths.extend(other.binding_paths.clone());
        self.module_paths.extend(other.module_paths.clone());
        self.module_names_by_id
            .extend(other.module_names_by_id.clone());
        self.item_paths.extend(
            other
                .item_paths
                .iter()
                .filter(|(path, _)| path.contains("::"))
                .map(|(path, id)| (path.clone(), *id)),
        );
        self.item_names_by_id.extend(other.item_names_by_id.clone());
        self.import_aliases.extend(
            other
                .import_aliases
                .iter()
                .filter(|(alias, _)| alias.contains("::"))
                .map(|(alias, id)| (alias.clone(), *id)),
        );
        self.export_aliases.extend(
            other
                .export_aliases
                .iter()
                .filter(|(alias, _)| alias.contains("::"))
                .map(|(alias, id)| (alias.clone(), *id)),
        );
        for (module, aliases) in &other.scoped_module_aliases {
            self.scoped_module_aliases
                .entry(module.clone())
                .or_default()
                .extend(aliases.clone());
        }
        self.module_aliases.extend(other.module_aliases.clone());
    }
}

pub fn build_resolver_tables(
    item_index: &ItemIndex,
    root_module_id: ModuleId,
    current_crate_name: Option<&str>,
    import_aliases: &HashMap<String, String>,
    export_aliases: &HashMap<String, String>,
    _export_function_aliases: &HashMap<String, String>,
) -> ResolverTables {
    let mut canonical_module_paths = HashMap::new();
    let mut canonical_module_names_by_id = HashMap::new();
    let mut module_segments = HashMap::new();
    let mut module_binding_paths = HashMap::new();

    for module in item_index.modules() {
        module_binding_paths.insert(module.module_id, item_index.module_binding_path(module.module_id));
        let segments = canonical_module_segments(
            item_index,
            root_module_id,
            module.module_id,
            current_crate_name,
        );

        if module.module_id != root_module_id && !segments.is_empty()
            && item_index.module_binding_path(module.module_id).iter().all(|(_, context)| *context == SyntaxContext::ROOT) {
            let canonical_path = segments.join("::");
            canonical_module_paths.insert(canonical_path.clone(), module.module_id);
            if let Some(alias) = crate_qualified_path_alias(current_crate_name, &canonical_path) {
                canonical_module_paths.insert(alias, module.module_id);
            }
            canonical_module_names_by_id.insert(module.module_id, canonical_path);
        }

        module_segments.insert(module.module_id, segments);
    }

    let mut canonical_item_paths = HashMap::new();
    let mut canonical_item_names_by_id = HashMap::new();
    let mut scoped_module_aliases: HashMap<String, HashMap<String, DefId>> = HashMap::new();
    let mut binding_paths = HashMap::new();
    let mut signature_paths = HashMap::new();
    let mut definition_binding_paths = HashMap::new();
    for item in item_index.items() {
        if item.kind == ItemKind::Impl {
            continue;
        }
        let binding_path = item_index.item_binding_path(item);
        definition_binding_paths.insert(item.def_id, binding_path.clone());
        if item.kind == ItemKind::FunctionSignature { signature_paths.insert(binding_path.clone(), item.def_id); }
        binding_paths.insert(binding_path.clone(), item.def_id);
        if let Some(crate_name) = current_crate_name {
            let mut qualified = vec![(crate_name.to_string(), SyntaxContext::ROOT)];
            qualified.extend(binding_path.clone());
            binding_paths.insert(qualified, item.def_id);
        }
        let root_visible = binding_path.iter().all(|(_, context)| *context == SyntaxContext::ROOT);

        let canonical_path = if item.kind == ItemKind::Module {
            let Some(canonical_path) = canonical_module_path_for_shell(
                item_index,
                root_module_id,
                current_crate_name,
                item.module_id,
                &item.name,
            ) else {
                continue;
            };

            canonical_path
        } else {
            let mut path = module_segments
                .get(&item.module_id)
                .cloned()
                .unwrap_or_default();
            path.push(item.name.clone());
            path.join("::")
        };

        if item.kind != ItemKind::Module && root_visible {
            if let Some(module_name) = canonical_module_names_by_id.get(&item.module_id) {
                scoped_module_aliases
                    .entry(module_name.clone())
                    .or_default()
                    .insert(item.name.clone(), item.def_id);
            }
        }

        if root_visible {
            canonical_item_paths.insert(canonical_path.clone(), item.def_id);
            if let Some(alias) = crate_qualified_path_alias(current_crate_name, &canonical_path) {
                canonical_item_paths.insert(alias, item.def_id);
            }
        }
        canonical_item_names_by_id.insert(item.def_id, canonical_path);
    }

    let resolved_import_aliases = import_aliases
        .iter()
        .filter_map(|(alias, target_path)| {
            canonical_item_paths
                .get(target_path)
                .copied()
                .map(|def_id| (alias.clone(), def_id))
        })
        .collect();

    let resolved_export_aliases = export_aliases
        .iter()
        .filter_map(|(alias_path, source_path)| {
            canonical_item_paths
                .get(source_path)
                .copied()
                .map(|def_id| (alias_path.clone(), def_id))
        })
        .collect();

    ResolverTables {
        generic_contexts: HashMap::new(),
        associated_contexts: HashMap::new(),
        member_contexts: HashMap::new(),
        variant_contexts: HashMap::new(),
        binding_paths,
        binding_aliases: HashMap::new(),
        binding_exports: HashMap::new(),
        macro_bindings: HashSet::new(),
        definition_binding_paths,
        signature_paths,
        module_binding_paths,
        definition_environments: HashMap::new(),
        module_paths: canonical_module_paths,
        module_names_by_id: canonical_module_names_by_id,
        item_paths: canonical_item_paths,
        item_names_by_id: canonical_item_names_by_id,
        import_aliases: resolved_import_aliases,
        export_aliases: resolved_export_aliases,
        scoped_module_aliases,
        module_aliases: HashMap::new(),
    }
}

fn crate_qualified_path_alias(
    current_crate_name: Option<&str>,
    canonical_path: &str,
) -> Option<String> {
    let crate_name = current_crate_name?;
    if canonical_path.is_empty()
        || canonical_path == crate_name
        || canonical_path.starts_with(&format!("{}::", crate_name))
    {
        return None;
    }

    Some(format!("{}::{}", crate_name, canonical_path))
}

fn canonical_module_segments(
    item_index: &ItemIndex,
    root_module_id: ModuleId,
    module_id: ModuleId,
    current_crate_name: Option<&str>,
) -> Vec<String> {
    if module_id == root_module_id {
        return Vec::new();
    }

    let mut lineage = Vec::new();
    let mut current = Some(module_id);

    while let Some(current_id) = current {
        let module = item_index
            .get_module(current_id)
            .expect("indexed module IDs should resolve");
        if module.module_id == root_module_id {
            break;
        }

        lineage.push((module.kind, module.name.clone()));
        current = module.parent;
    }

    lineage.reverse();

    let mut segments = Vec::new();
    if matches!(lineage.first(), Some((ModuleKind::SourceBacked, _))) {
        if let Some(crate_name) = current_crate_name {
            segments.push(crate_name.to_string());
        }
    }

    for (_, name) in lineage {
        if let Some(name) = name {
            segments.push(name);
        }
    }

    segments
}

fn canonical_module_path_for_shell(
    item_index: &ItemIndex,
    root_module_id: ModuleId,
    current_crate_name: Option<&str>,
    parent_module_id: ModuleId,
    module_name: &str,
) -> Option<String> {
    item_index
        .modules()
        .iter()
        .find(|module| {
            module.parent == Some(parent_module_id) && module.name.as_deref() == Some(module_name)
        })
        .map(|module| {
            canonical_module_segments(
                item_index,
                root_module_id,
                module.module_id,
                current_crate_name,
            )
            .join("::")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ids::{CrateId, DefId, LocalDefId};

    fn def_id(index: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(index))
    }

    #[test]
    fn resolver_tables_resolve_items_imports_and_exports_by_id() {
        let item_id = def_id(1);
        let import_id = def_id(2);
        let export_id = def_id(3);
        let mut resolver = ResolverTables::default();
        resolver
            .item_paths
            .insert("crate::item".to_string(), item_id);
        resolver
            .import_aliases
            .insert("imported".to_string(), import_id);
        resolver
            .export_aliases
            .insert("crate::alias".to_string(), export_id);

        assert_eq!(resolver.resolve_item_or_alias("crate::item"), Some(item_id));
        assert_eq!(resolver.resolve_item_or_alias("imported"), Some(import_id));
        assert_eq!(
            resolver.resolve_item_or_alias("crate::alias"),
            Some(export_id)
        );
        assert_eq!(resolver.resolve_item_or_alias("missing"), None);
    }

    #[test]
    fn resolver_tables_insert_aliases_with_reverse_names() {
        let import_id = def_id(10);
        let export_id = def_id(11);
        let mut resolver = ResolverTables::default();

        resolver.insert_import_alias_with_name(
            "short".to_string(),
            "dep::long".to_string(),
            import_id,
        );
        resolver.insert_export_alias_with_name(
            "crate::public".to_string(),
            "crate::private".to_string(),
            export_id,
        );

        assert_eq!(resolver.import_aliases.get("short"), Some(&import_id));
        assert_eq!(
            resolver.export_aliases.get("crate::public"),
            Some(&export_id)
        );
        assert_eq!(resolver.canonical_name(import_id), Some("dep::long"));
        assert_eq!(resolver.canonical_name(export_id), Some("crate::private"));
    }

    #[test]
    fn resolver_tables_resolve_distinct_alias_categories_by_id() {
        let import_id = def_id(21);
        let export_id = def_id(22);
        let module_id = def_id(23);
        let mut resolver = ResolverTables::default();

        resolver.insert_import_alias_with_name(
            "imported".to_string(),
            "dep::internal::value".to_string(),
            import_id,
        );
        resolver.insert_export_alias_with_name(
            "demo::public".to_string(),
            "demo::internal::value".to_string(),
            export_id,
        );
        resolver.insert_module_alias_with_name(
            "local".to_string(),
            "demo::module::local".to_string(),
            module_id,
        );

        assert_eq!(resolver.resolve_item_or_alias("imported"), Some(import_id));
        assert_eq!(
            resolver.resolve_item_or_alias("demo::public"),
            Some(export_id)
        );
        assert_eq!(resolver.resolve_item_or_alias("local"), Some(module_id));
        assert_eq!(
            resolver.canonical_name(module_id),
            Some("demo::module::local")
        );
    }

    #[test]
    fn resolver_tables_expose_scoped_module_local_aliases() {
        let first_id = def_id(31);
        let second_id = def_id(32);
        let mut resolver = ResolverTables::default();

        resolver
            .scoped_module_aliases
            .entry("demo::first".to_string())
            .or_default()
            .insert("answer".to_string(), first_id);
        resolver
            .scoped_module_aliases
            .entry("demo::second".to_string())
            .or_default()
            .insert("answer".to_string(), second_id);

        assert_eq!(
            resolver.resolve_module_local_alias("demo::first", "answer"),
            Some(first_id)
        );
        assert_eq!(
            resolver.resolve_module_local_alias("demo::second", "answer"),
            Some(second_id)
        );
    }
}

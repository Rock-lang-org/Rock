use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::collect::item_index::{ItemIndex, ItemKind, ModuleKind};
use crate::ids::{DefId, ModuleId};

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolverTables {
    pub module_paths: HashMap<String, ModuleId>,
    pub module_names_by_id: HashMap<ModuleId, String>,
    pub item_paths: HashMap<String, DefId>,
    pub item_names_by_id: HashMap<DefId, String>,
    pub import_aliases: HashMap<String, DefId>,
    pub export_aliases: HashMap<String, DefId>,
    pub scoped_module_aliases: HashMap<String, HashMap<String, DefId>>,
    pub module_aliases: HashMap<String, DefId>,
    /// Enum variants imported under a short name (`> Light::Off`, or the std
    /// prelude's `Some`), keyed like import aliases: the bare name at the
    /// crate root, `module::name` inside a module.
    #[serde(default)]
    pub variant_aliases: HashMap<String, VariantAlias>,
}

/// An imported enum variant; variants are not items, so they are named by
/// their enum and their variant name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariantAlias {
    pub enum_id: DefId,
    pub variant: String,
}

/// The variant an export names when its id is an enum but its source path
/// continues past the enum to one of its variants (`...::Option::Some`).
pub(crate) fn exported_variant_name(source: &str, enumeration: &crate::hir::HirEnum) -> Option<String> {
    let (owner, variant) = source.rsplit_once("::")?;
    let owner_name = owner.rsplit("::").next().unwrap_or(owner);
    (owner_name == enumeration.name
        && enumeration
            .variants
            .iter()
            .any(|candidate| candidate.name == variant))
    .then(|| variant.to_string())
}

impl ResolverTables {
    /// A variant imported under `name`, preferring an import made inside
    /// `module_path` over one made at the crate root.
    pub fn resolve_variant_alias(&self, module_path: Option<&str>, name: &str) -> Option<&VariantAlias> {
        module_path
            .and_then(|module_path| self.variant_aliases.get(&format!("{module_path}::{name}")))
            .or_else(|| self.variant_aliases.get(name))
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
    export_aliases: &HashMap<String, String>,
    import_aliases: HashMap<String, DefId>,
) -> ResolverTables {
    let mut canonical_module_paths = HashMap::new();
    let mut canonical_module_names_by_id = HashMap::new();
    let mut module_segments = HashMap::new();

    for module in item_index.modules() {
        let segments = canonical_module_segments(
            item_index,
            root_module_id,
            module.module_id,
            current_crate_name,
        );

        if module.module_id != root_module_id && !segments.is_empty() {
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
    for item in item_index.items() {
        if item.kind == ItemKind::Impl {
            continue;
        }

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

        if item.kind != ItemKind::Module {
            if let Some(module_name) = canonical_module_names_by_id.get(&item.module_id) {
                scoped_module_aliases
                    .entry(module_name.clone())
                    .or_default()
                    .insert(item.name.clone(), item.def_id);
            }
        }

        canonical_item_paths.insert(canonical_path.clone(), item.def_id);
        if let Some(alias) = crate_qualified_path_alias(current_crate_name, &canonical_path) {
            canonical_item_paths.insert(alias, item.def_id);
        }
        canonical_item_names_by_id.insert(item.def_id, canonical_path);
    }

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
        module_paths: canonical_module_paths,
        module_names_by_id: canonical_module_names_by_id,
        item_paths: canonical_item_paths,
        item_names_by_id: canonical_item_names_by_id,
        import_aliases,
        export_aliases: resolved_export_aliases,
        scoped_module_aliases,
        module_aliases: HashMap::new(),
        variant_aliases: HashMap::new(),
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
mod tests;

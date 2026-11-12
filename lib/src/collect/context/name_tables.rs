//! The names the collector binds to items: imports, re-exports and the
//! canonical name of each item. Every change to them goes through
//! `NameTables`, so the alias rules (which entry wins, what an explicit import
//! replaces) live in one place instead of in each module that records a name.

use std::collections::{HashMap, HashSet};

use crate::ids::DefId;

#[derive(Default)]
pub(crate) struct NameTables {
    /// Import name to the qualified path it was imported from.
    import_aliases: HashMap<String, String>,
    /// Import name, short or module-qualified, to the imported item.
    canonical_import_aliases: HashMap<String, DefId>,
    canonical_names_by_id: HashMap<DefId, String>,
    /// Module path to the names imported into that module.
    scoped_module_aliases: HashMap<String, HashMap<String, DefId>>,
    /// Re-export path to the qualified path of its source.
    export_aliases: HashMap<String, String>,
    local_export_paths: HashSet<String>,
    /// Every current-crate item by canonical path, known before any header is
    /// lowered, so imports resolve without waiting for their target's types.
    declared_items: HashMap<String, DefId>,
    /// Imports resolved to an item whose header was not lowered yet.
    deferred_import_bindings: Vec<(String, DefId)>,
}

/// The tables the resolver keeps once collection ends.
pub(crate) struct ResolverNameTables {
    pub(crate) canonical_import_aliases: HashMap<String, DefId>,
    pub(crate) canonical_names_by_id: HashMap<DefId, String>,
    pub(crate) scoped_module_aliases: HashMap<String, HashMap<String, DefId>>,
    pub(crate) export_aliases: HashMap<String, String>,
}

impl NameTables {
    pub(crate) fn import_alias(&self, name: &str) -> Option<&str> {
        self.import_aliases.get(name).map(String::as_str)
    }

    /// Binds `name` to `target`, replacing any earlier import of that name.
    pub(crate) fn set_import_alias(&mut self, name: String, target: String) {
        self.import_aliases.insert(name, target);
    }

    /// Binds `name` to `target` unless something already took the name.
    pub(crate) fn default_import_alias(&mut self, name: String, target: String) {
        self.import_aliases.entry(name).or_insert(target);
    }

    pub(crate) fn canonical_alias(&self, name: &str) -> Option<DefId> {
        self.canonical_import_aliases.get(name).copied()
    }

    /// Binds `alias` to `id`. Only an explicit import (`overwrite`) replaces
    /// a name the prelude or a glob import already bound.
    pub(crate) fn record_canonical_alias(&mut self, alias: String, id: DefId, overwrite: bool) {
        if overwrite {
            self.canonical_import_aliases.insert(alias, id);
        } else {
            self.canonical_import_aliases.entry(alias).or_insert(id);
        }
    }

    pub(crate) fn canonical_name(&self, id: DefId) -> Option<&str> {
        self.canonical_names_by_id.get(&id).map(String::as_str)
    }

    pub(crate) fn canonical_names(&self) -> impl Iterator<Item = (DefId, &str)> {
        self.canonical_names_by_id
            .iter()
            .map(|(id, name)| (*id, name.as_str()))
    }

    pub(crate) fn set_canonical_name(&mut self, id: DefId, name: String) {
        self.canonical_names_by_id.insert(id, name);
    }

    pub(crate) fn default_canonical_name(&mut self, id: DefId, name: String) {
        self.canonical_names_by_id.entry(id).or_insert(name);
    }

    pub(crate) fn record_scoped_module_alias(&mut self, module: &str, alias: String, id: DefId) {
        self.scoped_module_aliases
            .entry(module.to_string())
            .or_default()
            .insert(alias, id);
    }

    pub(crate) fn export_alias(&self, path: &str) -> Option<&str> {
        self.export_aliases.get(path).map(String::as_str)
    }

    pub(crate) fn export_sources(&self) -> impl Iterator<Item = (&String, &String)> {
        self.export_aliases.iter().filter(|(name, _)| self.local_export_paths.contains(*name))
    }

    pub(crate) fn set_export_alias(&mut self, path: String, source: String) {
        self.local_export_paths.insert(path.clone());
        self.export_aliases.insert(path, source);
    }

    pub(crate) fn set_dependency_export_alias(&mut self, path: String, source: String) {
        self.export_aliases.insert(path, source);
    }

    /// Records a re-export found before its module is walked; the walk itself
    /// may later refine it with `set_export_alias`.
    pub(crate) fn default_export_alias(&mut self, path: String, source: String) {
        self.local_export_paths.insert(path.clone());
        self.export_aliases.entry(path).or_insert(source);
    }

    pub(crate) fn declared_item(&self, path: &str) -> Option<DefId> {
        self.declared_items.get(path).copied()
    }

    pub(crate) fn declare_item(&mut self, path: String, id: DefId) {
        self.declared_items.insert(path, id);
    }

    pub(crate) fn defer_import_binding(&mut self, name: String, id: DefId) {
        self.deferred_import_bindings.push((name, id));
    }

    pub(crate) fn take_deferred_import_bindings(&mut self) -> Vec<(String, DefId)> {
        std::mem::take(&mut self.deferred_import_bindings)
    }

    pub(crate) fn into_resolver_tables(self) -> ResolverNameTables {
        ResolverNameTables {
            canonical_import_aliases: self.canonical_import_aliases,
            canonical_names_by_id: self.canonical_names_by_id,
            scoped_module_aliases: self.scoped_module_aliases,
            export_aliases: self.export_aliases,
        }
    }
}

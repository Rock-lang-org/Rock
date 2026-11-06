//! The source files the collector reads and where it currently is among
//! them: the file being collected, the qualified prefix of the module being
//! expanded, the `(qualified name, path)` registry and the parsed-file cache.
//! Switching the current file or prefix returns a frame the caller hands back
//! to restore it, so nested module walks cannot forget to unwind.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::ast;

#[derive(Default)]
pub(crate) struct ModuleSources {
    current_path: PathBuf,
    /// Overrides the prefix derived from `current_path` while glob exports of
    /// another module are expanded.
    qualified_prefix: Option<String>,
    /// Canonical paths of the cached modules handed out so far.
    loaded: HashSet<PathBuf>,
    paths: Vec<(String, PathBuf)>,
    file_cache: HashMap<PathBuf, ast::Module>,
}

/// The current file replaced by `ModuleSources::enter`.
pub(crate) struct ModulePathFrame(PathBuf);

/// The qualified prefix replaced by `ModuleSources::enter_qualified_prefix`.
pub(crate) struct QualifiedPrefixFrame(Option<String>);

impl ModuleSources {
    pub(crate) fn current_path(&self) -> &Path {
        &self.current_path
    }

    pub(crate) fn set_current_path(&mut self, path: PathBuf) {
        self.current_path = path;
    }

    /// Makes `path` the file being collected until `leave` gets the frame back.
    pub(crate) fn enter(&mut self, path: PathBuf) -> ModulePathFrame {
        ModulePathFrame(std::mem::replace(&mut self.current_path, path))
    }

    pub(crate) fn leave(&mut self, frame: ModulePathFrame) {
        self.current_path = frame.0;
    }

    pub(crate) fn qualified_prefix(&self) -> Option<&str> {
        self.qualified_prefix.as_deref()
    }

    pub(crate) fn enter_qualified_prefix(&mut self, prefix: String) -> QualifiedPrefixFrame {
        QualifiedPrefixFrame(self.qualified_prefix.replace(prefix))
    }

    pub(crate) fn leave_qualified_prefix(&mut self, frame: QualifiedPrefixFrame) {
        self.qualified_prefix = frame.0;
    }

    /// Registers the crate's root file ahead of every other module, unless
    /// it is already registered or no root file is known.
    pub(crate) fn register_root(&mut self, crate_name: &str) {
        if self.current_path.as_os_str().is_empty() {
            return;
        }

        if !self.has_entry(crate_name, &self.current_path) {
            self.paths
                .insert(0, (crate_name.to_string(), self.current_path.clone()));
        }
    }

    /// Registers `path` under `qualified_name` once.
    pub(crate) fn add_path(&mut self, qualified_name: String, path: PathBuf) {
        if !self.has_entry(&qualified_name, &path) {
            self.paths.push((qualified_name, path));
        }
    }

    fn has_entry(&self, qualified_name: &str, path: &Path) -> bool {
        self.paths
            .iter()
            .any(|(name, existing)| name == qualified_name && existing == path)
    }

    /// Caches `module` for `path` unless a module is already cached there.
    pub(crate) fn cache_module(&mut self, path: PathBuf, module: ast::Module) {
        self.file_cache.entry(path).or_insert(module);
    }

    pub(crate) fn cache_module_with(
        &mut self,
        path: PathBuf,
        module: impl FnOnce() -> ast::Module,
    ) {
        self.file_cache.entry(path).or_insert_with(module);
    }

    pub(crate) fn paths(&self) -> &[(String, PathBuf)] {
        &self.paths
    }

    pub(crate) fn file_cache(&self) -> &HashMap<PathBuf, ast::Module> {
        &self.file_cache
    }

    /// Whether `name` is a registered module or crate name.
    pub(crate) fn is_loaded_name(&self, name: &str) -> bool {
        self.paths.iter().any(|(loaded, _)| loaded == name)
    }

    pub(crate) fn path_for_name(&self, name: &str) -> Option<PathBuf> {
        self.paths
            .iter()
            .find(|(loaded, _)| loaded == name)
            .map(|(_, path)| path.clone())
    }

    /// The registered name of the file being collected.
    pub(crate) fn name_of_current_path(&self) -> Option<String> {
        self.paths
            .iter()
            .find(|(_, path)| path == &self.current_path)
            .map(|(name, _)| name.clone())
    }

    /// The cached module for `path`, looked up by canonical path first.
    pub(crate) fn find_cached(&self, path: &Path) -> Option<&ast::Module> {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.file_cache
            .get(&canonical)
            .or_else(|| self.file_cache.get(path))
    }

    /// Like `find_cached`, and records the module as loaded.
    pub(crate) fn load_cached(&mut self, path: &Path) -> Option<ast::Module> {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let module = self
            .file_cache
            .get(&canonical)
            .or_else(|| self.file_cache.get(path))
            .cloned()?;
        self.loaded.insert(canonical);
        Some(module)
    }

    pub(crate) fn into_registry_and_cache(
        self,
    ) -> (Vec<(String, PathBuf)>, HashMap<PathBuf, ast::Module>) {
        (self.paths, self.file_cache)
    }
}

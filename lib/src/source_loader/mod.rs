use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::ast;
use crate::diagnostic::Diagnostics;
use crate::lexer::Span;
use crate::parser::{self, ParseError};
use crate::Config;

mod diagnostics;
pub use diagnostics::source_load_errors_to_diagnostics;

/// Expands the macros of one loaded file module. The loader applies it before
/// loading the module's own `mod` children, so generated `mod` items load too.
pub type ModuleExpander<'a> = dyn FnMut(ast::Module) -> Result<ast::Module, Diagnostics> + 'a;

#[derive(Debug, Clone)]
pub struct SourceDatabase {
    registered_sources: BTreeMap<PathBuf, RegisteredSource>,
    files: BTreeMap<PathBuf, SourceFile>,
    modules: BTreeMap<PathBuf, ast::Module>,
    /// Canonical paths whose cached module has already been macro-expanded.
    expanded_modules: BTreeSet<PathBuf>,
    load_states: BTreeMap<PathBuf, ModuleLoadState>,
    loaded_files: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceOrigin {
    FileSystem,
    Virtual,
    Artifact { artifact_path: PathBuf },
}

#[derive(Debug, Clone)]
pub struct SourceFile {
    pub original_path: PathBuf,
    pub canonical_path: PathBuf,
    pub display_path: PathBuf,
    pub text: String,
    pub origin: SourceOrigin,
}

#[derive(Debug, Clone)]
struct RegisteredSource {
    display_path: PathBuf,
    text: String,
    origin: SourceOrigin,
}

#[derive(Debug, Clone)]
pub struct ModuleGraph {
    root_path: PathBuf,
    root_module: ast::Module,
    modules: Vec<LoadedModule>,
    modules_by_qualified_name: BTreeMap<String, ModuleId>,
    modules_by_path: BTreeMap<PathBuf, ModuleId>,
    loaded_files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleId(pub usize);

#[derive(Debug, Clone)]
pub struct LoadedModule {
    pub id: ModuleId,
    pub qualified_name: String,
    pub path: PathBuf,
    pub canonical_path: PathBuf,
    pub module: ast::Module,
}

#[derive(Debug, Clone, Default)]
pub struct SourceModuleSet {
    modules: Vec<LoadedModule>,
    modules_by_qualified_name: BTreeMap<String, ModuleId>,
    modules_by_path: BTreeMap<PathBuf, ModuleId>,
}

#[derive(Debug, Clone)]
pub enum SourceLoadError {
    Io {
        path: PathBuf,
        message: String,
    },
    MissingModule {
        module: String,
        searched: Vec<PathBuf>,
        span: Option<Span>,
        parent_source: Option<SourceFile>,
    },
    Parse {
        path: PathBuf,
        source: Option<SourceFile>,
        error: ParseError,
    },
    CircularModule {
        path: PathBuf,
        stack: Vec<PathBuf>,
    },
    Expansion {
        path: PathBuf,
        diagnostics: Diagnostics,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModuleLoadState {
    Loading,
    Loaded,
}

impl ModuleGraph {
    fn new(root_path: PathBuf) -> Self {
        Self {
            root_path: root_path.clone(),
            root_module: ast::Module {
                name: None,
                top_levels: Vec::new(),
                is_inline: true,
                filepath: Some(root_path),
            },
            modules: Vec::new(),
            modules_by_qualified_name: BTreeMap::new(),
            modules_by_path: BTreeMap::new(),
            loaded_files: Vec::new(),
        }
    }

    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    pub fn root_module(&self) -> &ast::Module {
        &self.root_module
    }

    pub fn loaded_files(&self) -> &[PathBuf] {
        &self.loaded_files
    }

    pub fn modules(&self) -> impl Iterator<Item = &LoadedModule> {
        self.modules.iter()
    }

    pub fn module(&self, id: ModuleId) -> Option<&LoadedModule> {
        self.modules.get(id.0)
    }

    pub fn module_by_qualified_name(&self, name: &str) -> Option<&LoadedModule> {
        self.modules_by_qualified_name
            .get(name)
            .and_then(|id| self.module(*id))
    }

    pub fn module_for_path(&self, path: &Path) -> Option<&LoadedModule> {
        self.modules_by_path
            .get(path)
            .and_then(|id| self.module(*id))
    }

    pub fn loaded_module_paths(&self) -> Vec<(String, PathBuf)> {
        self.modules
            .iter()
            .map(|module| (module.qualified_name.clone(), module.path.clone()))
            .collect()
    }

    pub fn module_file_cache(&self) -> std::collections::HashMap<PathBuf, ast::Module> {
        let mut cache = std::collections::HashMap::new();
        for module in &self.modules {
            cache.insert(module.path.clone(), module.module.clone());
            if module.canonical_path != module.path {
                cache
                    .entry(module.canonical_path.clone())
                    .or_insert_with(|| module.module.clone());
            }
        }
        cache
    }

    pub fn source_modules(&self) -> SourceModuleSet {
        SourceModuleSet::from_modules(self.modules().cloned())
    }
}

impl SourceModuleSet {
    pub fn from_modules(modules: impl IntoIterator<Item = LoadedModule>) -> Self {
        let mut set = Self::default();
        for module in modules {
            set.insert(module);
        }
        set
    }

    pub fn insert(&mut self, mut module: LoadedModule) {
        let id = ModuleId(self.modules.len());
        module.id = id;
        self.modules_by_path.insert(module.path.clone(), id);
        if module.canonical_path != module.path {
            self.modules_by_path
                .entry(module.canonical_path.clone())
                .or_insert(id);
        }
        self.modules_by_qualified_name
            .insert(module.qualified_name.clone(), id);
        self.modules.push(module);
    }

    pub fn modules(&self) -> impl Iterator<Item = &LoadedModule> {
        self.modules.iter()
    }

    pub fn module(&self, id: ModuleId) -> Option<&LoadedModule> {
        self.modules.get(id.0)
    }

    pub fn module_by_qualified_name(&self, name: &str) -> Option<&LoadedModule> {
        self.modules_by_qualified_name
            .get(name)
            .and_then(|id| self.module(*id))
    }

    pub fn module_for_path(&self, path: &Path) -> Option<&LoadedModule> {
        self.modules_by_path
            .get(path)
            .and_then(|id| self.module(*id))
    }

    pub fn has_root_name(&self, root_name: &str) -> bool {
        self.modules_by_qualified_name.contains_key(root_name)
    }

    pub fn root_path(&self, root_name: &str) -> Option<PathBuf> {
        self.module_by_qualified_name(root_name)
            .map(|module| module.path.clone())
    }

    pub fn loaded_module_paths(&self) -> Vec<(String, PathBuf)> {
        self.modules()
            .map(|module| (module.qualified_name.clone(), module.path.clone()))
            .collect()
    }
}

impl From<Vec<LoadedModule>> for SourceModuleSet {
    fn from(modules: Vec<LoadedModule>) -> Self {
        Self::from_modules(modules)
    }
}

impl FromIterator<LoadedModule> for SourceModuleSet {
    fn from_iter<T: IntoIterator<Item = LoadedModule>>(iter: T) -> Self {
        Self::from_modules(iter)
    }
}

impl SourceDatabase {
    pub fn new() -> Self {
        Self {
            registered_sources: BTreeMap::new(),
            files: BTreeMap::new(),
            modules: BTreeMap::new(),
            expanded_modules: BTreeSet::new(),
            load_states: BTreeMap::new(),
            loaded_files: Vec::new(),
        }
    }

    pub fn loaded_files(&self) -> &[PathBuf] {
        &self.loaded_files
    }

    pub fn source_files(&self) -> impl Iterator<Item = &SourceFile> {
        self.files.values()
    }

    pub fn add_virtual_source(&mut self, path: PathBuf, text: impl Into<String>) -> &mut Self {
        self.add_registered_source(path, text, SourceOrigin::Virtual)
    }

    pub fn add_artifact_source(
        &mut self,
        path: PathBuf,
        artifact_path: PathBuf,
        text: impl Into<String>,
    ) -> &mut Self {
        self.add_registered_source(path, text, SourceOrigin::Artifact { artifact_path })
    }

    pub fn add_source_provider(&mut self, provider: &crate::SourceProvider) -> &mut Self {
        match provider {
            crate::SourceProvider::Virtual { path, text } => {
                self.add_virtual_source(path.clone(), text.clone())
            }
            crate::SourceProvider::Artifact {
                path,
                artifact_path,
                text,
            } => self.add_artifact_source(path.clone(), artifact_path.clone(), text.clone()),
        }
    }

    pub fn add_source_providers<'a>(
        &mut self,
        providers: impl IntoIterator<Item = &'a crate::SourceProvider>,
    ) -> &mut Self {
        for provider in providers {
            self.add_source_provider(provider);
        }
        self
    }

    pub fn source_file_for_path(&self, path: &Path) -> Option<&SourceFile> {
        self.canonical_key(path)
            .ok()
            .and_then(|key| self.files.get(&key))
            .or_else(|| self.files.get(path))
    }

    pub fn module_for_path(&self, path: &Path) -> Option<&ast::Module> {
        self.canonical_key(path)
            .ok()
            .and_then(|key| self.modules.get(&key))
            .or_else(|| self.modules.get(path))
    }

    pub fn load_entry(
        &mut self,
        path: PathBuf,
        config: &Config,
    ) -> Result<ModuleGraph, Vec<SourceLoadError>> {
        let root_prefix = config.current_crate_name.clone();
        self.load_graph(path, root_prefix, config)
    }

    pub fn load_source_crate(
        &mut self,
        lib_path: PathBuf,
        crate_name: &str,
        config: &Config,
    ) -> Result<ModuleGraph, Vec<SourceLoadError>> {
        self.load_graph(lib_path, Some(crate_name.to_string()), config)
    }

    /// Loads the `mod` children of an already expanded root module, expanding
    /// each child file's macros with `expander` before loading its own children.
    pub fn load_module_declarations(
        &mut self,
        graph: &mut ModuleGraph,
        module: &ast::Module,
        root_prefix: Option<&str>,
        config: &Config,
        expander: &mut ModuleExpander<'_>,
    ) -> Result<(), Vec<SourceLoadError>> {
        graph.root_module = module.clone();
        let mut stack = Vec::new();
        let mut errors = Vec::new();
        self.load_children(
            module,
            root_prefix,
            config,
            &mut stack,
            graph,
            &mut errors,
            Some(expander),
        );

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    fn add_registered_source(
        &mut self,
        path: PathBuf,
        text: impl Into<String>,
        origin: SourceOrigin,
    ) -> &mut Self {
        let source = RegisteredSource {
            display_path: path.clone(),
            text: text.into(),
            origin,
        };
        self.registered_sources.insert(path.clone(), source);
        self.files.remove(&path);
        self.modules.remove(&path);
        self.expanded_modules.remove(&path);
        self.load_states.remove(&path);
        self
    }

    fn load_graph(
        &mut self,
        root_path: PathBuf,
        root_prefix: Option<String>,
        config: &Config,
    ) -> Result<ModuleGraph, Vec<SourceLoadError>> {
        let mut errors = Vec::new();
        let mut graph = ModuleGraph::new(root_path.clone());
        let mut stack = Vec::new();
        let root_module = self.load_module_recursive(
            root_path.clone(),
            root_prefix.clone(),
            config,
            &mut stack,
            &mut graph,
            &mut errors,
            None,
        );

        if let Some(root_module) = root_module {
            graph.root_module = root_module;
        }

        if errors.is_empty() {
            Ok(graph)
        } else {
            Err(errors)
        }
    }

    fn load_module_recursive(
        &mut self,
        path: PathBuf,
        qualified_name: Option<String>,
        config: &Config,
        stack: &mut Vec<PathBuf>,
        graph: &mut ModuleGraph,
        errors: &mut Vec<SourceLoadError>,
        mut expander: Option<&mut ModuleExpander<'_>>,
    ) -> Option<ast::Module> {
        let canonical_path = match self.canonical_key(&path) {
            Ok(path) => path,
            Err(error) => {
                errors.push(error);
                return None;
            }
        };

        if self.load_states.get(&canonical_path) == Some(&ModuleLoadState::Loading) {
            errors.push(SourceLoadError::CircularModule {
                path: path.clone(),
                stack: stack.clone(),
            });
            return self.modules.get(&canonical_path).cloned();
        }

        if let Some(mut module) = self.modules.get(&canonical_path).cloned() {
            module.filepath = Some(path.clone());
            if let Some(expander) = expander.as_deref_mut() {
                if !self.expanded_modules.contains(&canonical_path) {
                    module = match expander(module) {
                        Ok(module) => module,
                        Err(diagnostics) => {
                            errors.push(SourceLoadError::Expansion { path, diagnostics });
                            return None;
                        }
                    };
                    self.modules.insert(canonical_path.clone(), module.clone());
                    self.expanded_modules.insert(canonical_path.clone());
                }
            }
            self.record_graph_loaded_file(graph, path.clone());
            if let Some(qualified_name) = qualified_name.clone() {
                self.record_loaded_module(
                    graph,
                    qualified_name,
                    path.clone(),
                    canonical_path.clone(),
                    module.clone(),
                );
            }
            self.load_states
                .insert(canonical_path.clone(), ModuleLoadState::Loading);
            stack.push(path.clone());
            self.load_children(
                &module,
                qualified_name.as_deref(),
                config,
                stack,
                graph,
                errors,
                expander,
            );
            stack.pop();
            self.load_states
                .insert(canonical_path, ModuleLoadState::Loaded);
            return Some(module);
        }

        self.load_states
            .insert(canonical_path.clone(), ModuleLoadState::Loading);
        stack.push(path.clone());

        let source = match self.read_source_file(path.clone(), canonical_path.clone()) {
            Ok(source) => source,
            Err(error) => {
                errors.push(error);
                stack.pop();
                self.load_states.remove(&canonical_path);
                return None;
            }
        };

        let module = match parser::parse_source(path.clone(), &source.text, config) {
            Ok(module) => module,
            Err(error) => {
                errors.push(SourceLoadError::Parse {
                    path: path.clone(),
                    source: Some(source),
                    error,
                });
                stack.pop();
                self.load_states.remove(&canonical_path);
                return None;
            }
        };
        let module = match expander.as_deref_mut() {
            Some(expander) => match expander(module) {
                Ok(module) => {
                    self.expanded_modules.insert(canonical_path.clone());
                    module
                }
                Err(diagnostics) => {
                    errors.push(SourceLoadError::Expansion {
                        path: path.clone(),
                        diagnostics,
                    });
                    stack.pop();
                    self.load_states.remove(&canonical_path);
                    return None;
                }
            },
            None => module,
        };

        self.modules.insert(canonical_path.clone(), module.clone());
        if !self.loaded_files.contains(&path) {
            self.loaded_files.push(path.clone());
        }
        self.record_graph_loaded_file(graph, path.clone());

        if let Some(qualified_name) = qualified_name.clone() {
            self.record_loaded_module(
                graph,
                qualified_name,
                path.clone(),
                canonical_path.clone(),
                module.clone(),
            );
        }

        self.load_children(
            &module,
            qualified_name.as_deref(),
            config,
            stack,
            graph,
            errors,
            expander,
        );
        stack.pop();
        self.load_states
            .insert(canonical_path, ModuleLoadState::Loaded);

        Some(module)
    }

    fn canonical_key(&self, path: &Path) -> Result<PathBuf, SourceLoadError> {
        if self.registered_sources.contains_key(path) {
            return Ok(path.to_path_buf());
        }

        path.canonicalize().map_err(|error| SourceLoadError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })
    }

    fn read_source_file(
        &mut self,
        original_path: PathBuf,
        canonical_path: PathBuf,
    ) -> Result<SourceFile, SourceLoadError> {
        if let Some(file) = self.files.get(&canonical_path).cloned() {
            return Ok(file);
        }

        let registered_source = self
            .registered_sources
            .get(&canonical_path)
            .or_else(|| self.registered_sources.get(&original_path))
            .cloned();
        if let Some(source) = registered_source {
            let file = SourceFile {
                original_path: original_path.clone(),
                canonical_path: canonical_path.clone(),
                display_path: source.display_path,
                text: source.text,
                origin: source.origin,
            };
            self.files.insert(canonical_path, file.clone());
            return Ok(file);
        }

        let text =
            std::fs::read_to_string(&original_path).map_err(|error| SourceLoadError::Io {
                path: original_path.clone(),
                message: error.to_string(),
            })?;
        let file = SourceFile {
            original_path: original_path.clone(),
            canonical_path: canonical_path.clone(),
            display_path: original_path,
            text,
            origin: SourceOrigin::FileSystem,
        };
        self.files.insert(canonical_path, file.clone());
        Ok(file)
    }

    fn record_graph_loaded_file(&self, graph: &mut ModuleGraph, path: PathBuf) {
        if !graph.loaded_files.contains(&path) {
            graph.loaded_files.push(path);
        }
    }

    fn record_loaded_module(
        &self,
        graph: &mut ModuleGraph,
        qualified_name: String,
        path: PathBuf,
        canonical_path: PathBuf,
        module: ast::Module,
    ) {
        // A module recorded again after macro expansion replaces its parsed
        // entry; keeping both would make later phases visit the stale copy.
        if let Some(&existing) = graph.modules_by_qualified_name.get(&qualified_name) {
            graph.modules[existing.0] = LoadedModule {
                id: existing,
                qualified_name,
                path: path.clone(),
                canonical_path: canonical_path.clone(),
                module,
            };
            graph.modules_by_path.insert(path, existing);
            if canonical_path != graph.modules[existing.0].path {
                graph.modules_by_path.insert(canonical_path, existing);
            }
            return;
        }
        let id = ModuleId(graph.modules.len());
        graph.modules_by_path.insert(path.clone(), id);
        if canonical_path != path {
            graph
                .modules_by_path
                .entry(canonical_path.clone())
                .or_insert(id);
        }
        graph
            .modules_by_qualified_name
            .insert(qualified_name.clone(), id);
        graph.modules.push(LoadedModule {
            id,
            qualified_name,
            path,
            canonical_path,
            module,
        });
    }

    fn load_children(
        &mut self,
        module: &ast::Module,
        prefix: Option<&str>,
        config: &Config,
        stack: &mut Vec<PathBuf>,
        graph: &mut ModuleGraph,
        errors: &mut Vec<SourceLoadError>,
        mut expander: Option<&mut ModuleExpander<'_>>,
    ) {
        for top_level in &module.top_levels {
            match top_level {
                ast::TopLevel::Module(ast::ModuleDecl(inline)) => {
                    let next_prefix = inline.name.as_ref().map(|name| {
                        prefix
                            .map(|prefix| format!("{}::{}", prefix, name.name))
                            .unwrap_or_else(|| name.name.clone())
                    });
                    let mut inline = inline.clone();
                    if inline.filepath.is_none() {
                        inline.filepath = module.filepath.clone();
                    }
                    self.load_children(
                        &inline,
                        next_prefix.as_deref(),
                        config,
                        stack,
                        graph,
                        errors,
                        expander.as_deref_mut(),
                    );
                }
                ast::TopLevel::Mod(ident, _) => {
                    let Some(parent_path) = module.filepath.as_ref() else {
                        continue;
                    };
                    let Some(child_path) = self.resolve_sibling_module(parent_path, ident, errors)
                    else {
                        continue;
                    };
                    let qualified_name = prefix
                        .map(|prefix| format!("{}::{}", prefix, ident.name))
                        .unwrap_or_else(|| ident.name.clone());
                    self.load_module_recursive(
                        child_path,
                        Some(qualified_name),
                        config,
                        stack,
                        graph,
                        errors,
                        expander.as_deref_mut(),
                    );
                }
                _ => {}
            }
        }
    }

    fn resolve_sibling_module(
        &self,
        parent_path: &Path,
        ident: &ast::Ident,
        errors: &mut Vec<SourceLoadError>,
    ) -> Option<PathBuf> {
        let base = parent_path.parent().unwrap_or_else(|| Path::new("."));
        let flat = base.join(format!("{}.rk", ident.name));
        let directory_mod = base.join(&ident.name).join("mod.rk");

        if self.registered_sources.contains_key(&flat) {
            return Some(flat);
        }

        if self.registered_sources.contains_key(&directory_mod) {
            return Some(directory_mod);
        }

        if flat.exists() {
            return Some(flat);
        }

        if directory_mod.exists() {
            return Some(directory_mod);
        }

        errors.push(SourceLoadError::MissingModule {
            module: ident.name.clone(),
            searched: vec![flat, directory_mod],
            span: Some(ident.span.clone()),
            parent_source: self.source_file_for_path(parent_path).cloned(),
        });
        None
    }
}

impl Default for SourceDatabase {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;

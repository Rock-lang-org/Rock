use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::ast;
use crate::lexer::Span;
use crate::parser::{self, ParseError};
use crate::Config;

#[derive(Debug, Clone)]
pub struct SourceDatabase {
    registered_sources: BTreeMap<PathBuf, RegisteredSource>,
    files: BTreeMap<PathBuf, SourceFile>,
    modules: BTreeMap<PathBuf, ast::Module>,
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
    Resource { diagnostic: crate::diagnostic::Diagnostic },
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

    pub(crate) fn root_module_mut(&mut self) -> &mut ast::Module {
        &mut self.root_module
    }

    pub fn loaded_files(&self) -> &[PathBuf] {
        &self.loaded_files
    }

    pub fn modules(&self) -> impl Iterator<Item = &LoadedModule> {
        self.modules.iter()
    }

    pub(crate) fn modules_mut(&mut self) -> impl Iterator<Item = &mut LoadedModule> {
        self.modules.iter_mut()
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

    pub fn load_module_declarations(
        &mut self,
        graph: &mut ModuleGraph,
        module: &ast::Module,
        root_prefix: Option<&str>,
        config: &Config,
    ) -> Result<(), Vec<SourceLoadError>> {
        self.load_module_declarations_with_budget(graph, module, root_prefix, config, &mut |_, _, _| Ok(()))
    }

    /// Charge module visits (`None`) and source bytes (`Some`) before parsing.
    /// A file that grows after preflight charges its additional bytes afterward.
    pub fn load_module_declarations_with_budget(
        &mut self,
        graph: &mut ModuleGraph,
        module: &ast::Module,
        root_prefix: Option<&str>,
        config: &Config,
        budget: &mut dyn FnMut(&Path, Option<&str>, Option<usize>) -> Result<(), crate::diagnostic::Diagnostic>,
    ) -> Result<(), Vec<SourceLoadError>> {
        graph.root_module = module.clone();
        let mut stack = Vec::new();
        let mut errors = Vec::new();
        self.load_children(module, root_prefix, config, &mut stack, graph, &mut errors, budget);

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
            &mut |_, _, _| Ok(()),
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
        budget: &mut dyn FnMut(&Path, Option<&str>, Option<usize>) -> Result<(), crate::diagnostic::Diagnostic>,
    ) -> Option<ast::Module> {
        if let Err(diagnostic) = budget(&path, qualified_name.as_deref(), None) {
            errors.push(SourceLoadError::Resource { diagnostic });
            return None;
        }
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

        // Expansion publishes bodies into the graph, not the raw parse cache.
        // Rediscover children from that authoritative body without inserting a
        // duplicate raw module that would hide its expanded expressions.
        if let Some(loaded) = qualified_name.as_deref()
            .and_then(|name| graph.module_by_qualified_name(name))
            .filter(|loaded| loaded.canonical_path == canonical_path)
        {
            let module = loaded.module.clone();
            self.load_states.insert(canonical_path.clone(), ModuleLoadState::Loading);
            stack.push(path);
            self.load_children(&module, qualified_name.as_deref(), config, stack, graph, errors, budget);
            stack.pop();
            self.load_states.insert(canonical_path, ModuleLoadState::Loaded);
            return Some(module);
        }

        if let Some(mut module) = self.modules.get(&canonical_path).cloned() {
            module.filepath = Some(path.clone());
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
                budget,
            );
            stack.pop();
            self.load_states
                .insert(canonical_path, ModuleLoadState::Loaded);
            return Some(module);
        }

        self.load_states
            .insert(canonical_path.clone(), ModuleLoadState::Loading);
        stack.push(path.clone());

        let size = self.files.get(&canonical_path).map(|source| source.text.len())
            .or_else(|| self.registered_sources.get(&canonical_path).or_else(|| self.registered_sources.get(&path)).map(|source| source.text.len()))
            .or_else(|| std::fs::metadata(&path).ok().and_then(|metadata| usize::try_from(metadata.len()).ok()));
        if let Some(size) = size {
            if let Err(diagnostic) = budget(&path, qualified_name.as_deref(), Some(size)) {
                errors.push(SourceLoadError::Resource { diagnostic });
                stack.pop();
                self.load_states.remove(&canonical_path);
                return None;
            }
        }
        let source = match self.read_source_file(path.clone(), canonical_path.clone()) {
            Ok(source) => source,
            Err(error) => {
                errors.push(error);
                stack.pop();
                self.load_states.remove(&canonical_path);
                return None;
            }
        };

        if let Err(diagnostic) = budget(&path, qualified_name.as_deref(), Some(source.text.len().saturating_sub(size.unwrap_or(0)))) {
            errors.push(SourceLoadError::Resource { diagnostic });
            stack.pop();
            self.load_states.remove(&canonical_path);
            return None;
        }
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
            budget,
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

        let mut text = String::new();
        std::fs::File::open(&original_path)
            .and_then(|file| file.take(parser::SOURCE_LIMITS.max_source_bytes as u64 + 1).read_to_string(&mut text))
            .map_err(|error| SourceLoadError::Io {
                path: original_path.clone(),
                message: error.to_string(),
            })?;
        if text.len() > parser::SOURCE_LIMITS.max_source_bytes {
            return Err(SourceLoadError::Io { path: original_path, message: "source byte limit exceeded".into() });
        }
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
        budget: &mut dyn FnMut(&Path, Option<&str>, Option<usize>) -> Result<(), crate::diagnostic::Diagnostic>,
    ) {
        for top_level in &module.top_levels {
            if errors.iter().any(|error| matches!(error, SourceLoadError::Resource { .. })) { break; }
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
                    if let Some(path) = &inline.filepath {
                        if let Err(diagnostic) = budget(path, next_prefix.as_deref(), None) {
                            errors.push(SourceLoadError::Resource { diagnostic });
                            break;
                        }
                    }
                    self.load_children(
                        &inline,
                        next_prefix.as_deref(),
                        config,
                        stack,
                        graph,
                        errors,
                        budget,
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
                        budget,
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
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::ast::{Ident, Module, ModuleDecl, TopLevel};
    use crate::source_loader::{
        LoadedModule, ModuleGraph, ModuleId, SourceDatabase, SourceLoadError, SourceModuleSet,
        SourceOrigin,
    };
    use crate::Config;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rock_source_loader_{}_{}",
            std::process::id(),
            name
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn no_std_config(entry_file: PathBuf) -> Config {
        Config {
            entry_file,
            no_std: true,
            no_prelude: true,
            ..Config::default()
        }
    }

    #[test]
    fn virtual_sources_load_entry_and_sibling_modules_without_filesystem_paths() {
        let entry = PathBuf::from("/virtual/app/main.rk");
        let util = PathBuf::from("/virtual/app/util.rk");

        let mut db = SourceDatabase::new();
        db.add_virtual_source(entry.clone(), "mod util\nmain = -> util::answer!\n");
        db.add_virtual_source(util.clone(), "answer = -> 1\n< answer\n");

        let graph = db
            .load_entry(entry.clone(), &no_std_config(entry.clone()))
            .expect("virtual entry should load without filesystem access");

        assert_eq!(graph.loaded_files(), &[entry.clone(), util.clone()]);
        assert!(graph.module_by_qualified_name("util").is_some());
        assert!(db.module_for_path(&util).is_some());
    }

    #[test]
    fn module_graph_name_and_path_indexes_resolve_to_same_module_id() {
        let entry = PathBuf::from("/virtual/app/main.rk");
        let util = PathBuf::from("/virtual/app/util.rk");

        let mut db = SourceDatabase::new();
        db.add_virtual_source(entry.clone(), "mod util\nmain = -> util::answer!\n");
        db.add_virtual_source(util.clone(), "answer = -> 1\n< answer\n");

        let graph = db
            .load_entry(entry.clone(), &no_std_config(entry.clone()))
            .expect("virtual entry should load without filesystem access");

        let by_name = graph
            .module_by_qualified_name("util")
            .expect("name lookup should find util");
        let by_path = graph
            .module_for_path(&util)
            .expect("path lookup should find util");

        assert_eq!(by_name.id, by_path.id);
        assert_eq!(
            graph
                .module(by_name.id)
                .map(|module| &module.qualified_name),
            Some(&"util".to_string())
        );
    }

    #[test]
    fn module_graph_distinguishes_same_named_modules_under_different_parents() {
        let dir = temp_dir("same_named_nested_modules");
        let entry = dir.join("main.rk");
        fs::write(&entry, "mod left\nmod right\nmain = -> 0\n").unwrap();
        fs::create_dir_all(dir.join("left")).unwrap();
        fs::create_dir_all(dir.join("right")).unwrap();
        fs::write(dir.join("left").join("mod.rk"), "mod util\n").unwrap();
        fs::write(dir.join("left").join("util.rk"), "answer = -> 1\n").unwrap();
        fs::write(dir.join("right").join("mod.rk"), "mod util\n").unwrap();
        fs::write(dir.join("right").join("util.rk"), "answer = -> 2\n").unwrap();

        let mut db = SourceDatabase::new();
        let graph = db
            .load_entry(entry.clone(), &no_std_config(entry.clone()))
            .expect("nested modules should load");

        let left_util = graph
            .module_by_qualified_name("left::util")
            .expect("left util should be indexed");
        let right_util = graph
            .module_by_qualified_name("right::util")
            .expect("right util should be indexed");
        assert_ne!(left_util.id, right_util.id);
        assert_eq!(
            graph
                .module_for_path(&dir.join("left").join("util.rk"))
                .map(|module| module.id),
            Some(left_util.id)
        );
        assert_eq!(
            graph
                .module_for_path(&dir.join("right").join("util.rk"))
                .map(|module| module.id),
            Some(right_util.id)
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn module_graph_keeps_distinct_logical_modules_for_same_canonical_path() {
        let dir = temp_dir("same_canonical_logical_modules");
        let entry = dir.join("main.rk");
        let left_mod = dir.join("left").join("mod.rk");
        let right_mod = dir.join("right").join("mod.rk");
        fs::write(&entry, "mod left\nmod right\nmain = -> 0\n").unwrap();
        fs::create_dir_all(dir.join("left")).unwrap();
        fs::create_dir_all(dir.join("right")).unwrap();
        fs::write(&left_mod, "mod child\n").unwrap();
        fs::write(dir.join("left").join("child.rk"), "answer = -> 1\n").unwrap();
        fs::write(dir.join("right").join("child.rk"), "answer = -> 2\n").unwrap();
        std::os::unix::fs::symlink(&left_mod, &right_mod).unwrap();

        let mut db = SourceDatabase::new();
        let graph = db
            .load_entry(entry.clone(), &no_std_config(entry.clone()))
            .expect("symlinked modules should load");

        let left_module = graph
            .module_by_qualified_name("left")
            .expect("left module should be indexed");
        let right_module = graph
            .module_by_qualified_name("right")
            .expect("right module should be indexed");
        assert_ne!(left_module.id, right_module.id);
        assert_eq!(left_module.qualified_name, "left");
        assert_eq!(right_module.qualified_name, "right");
        assert_eq!(
            graph.module_for_path(&right_mod).map(|module| module.id),
            Some(right_module.id)
        );
        assert_eq!(
            graph.module_for_path(&left_mod).map(|module| module.id),
            Some(left_module.id)
        );
        assert_eq!(
            graph
                .module_by_qualified_name("right::child")
                .map(|module| module.path.clone()),
            Some(dir.join("right").join("child.rk"))
        );
        assert_eq!(
            graph
                .module_file_cache()
                .get(&left_mod)
                .and_then(|module| module.filepath.clone()),
            Some(left_mod.clone())
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn artifact_backed_source_registration_preserves_origin_metadata() {
        let entry = PathBuf::from("/artifact/app/main.rk");
        let artifact = PathBuf::from("/deps/app.rkca");

        let mut db = SourceDatabase::new();
        db.add_artifact_source(entry.clone(), artifact.clone(), "main = -> 0\n");

        let graph = db
            .load_entry(entry.clone(), &no_std_config(entry.clone()))
            .expect("artifact-backed source should load through explicit registration");
        let source = db
            .source_file_for_path(&entry)
            .expect("registered artifact-backed source should be cached");

        assert_eq!(graph.loaded_files(), &[entry.clone()]);
        assert_eq!(
            source.origin,
            SourceOrigin::Artifact {
                artifact_path: artifact
            }
        );
    }

    #[test]
    fn load_entry_records_root_source_file_and_module() {
        let dir = temp_dir("entry");
        let entry = dir.join("main.rk");
        fs::write(&entry, "main = -> 0\n").unwrap();

        let mut db = SourceDatabase::new();
        let graph = db
            .load_entry(entry.clone(), &no_std_config(entry.clone()))
            .expect("entry should load");

        assert_eq!(graph.root_path(), entry.as_path());
        assert_eq!(graph.root_module().filepath.as_ref(), Some(&entry));
        assert_eq!(graph.loaded_files(), &[entry.clone()]);
        assert!(db.module_for_path(&entry).is_some());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sibling_resolution_prefers_flat_file_before_mod_rs_style_directory() {
        let dir = temp_dir("sibling_preference");
        let entry = dir.join("main.rk");
        fs::write(&entry, "mod util\nmain = -> util::answer!\n").unwrap();
        fs::write(dir.join("util.rk"), "answer = -> 1\n< answer\n").unwrap();
        fs::create_dir_all(dir.join("util")).unwrap();
        fs::write(dir.join("util").join("mod.rk"), "answer = -> 2\n< answer\n").unwrap();

        let mut db = SourceDatabase::new();
        let graph = db
            .load_entry(entry.clone(), &no_std_config(entry.clone()))
            .expect("entry should load");

        let util = graph
            .module_by_qualified_name("util")
            .expect("util module should be loaded");
        assert_eq!(util.path, dir.join("util.rk"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sibling_resolution_supports_directory_mod_file() {
        let dir = temp_dir("directory_mod");
        let entry = dir.join("main.rk");
        fs::write(&entry, "mod util\nmain = -> util::answer!\n").unwrap();
        fs::create_dir_all(dir.join("util")).unwrap();
        fs::write(dir.join("util").join("mod.rk"), "answer = -> 2\n< answer\n").unwrap();

        let mut db = SourceDatabase::new();
        let graph = db
            .load_entry(entry.clone(), &no_std_config(entry.clone()))
            .expect("entry should load");

        assert!(graph
            .module_for_path(&dir.join("util").join("mod.rk"))
            .is_some());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn inline_module_nested_mod_resolves_relative_to_containing_source_file() {
        let dir = temp_dir("inline_nested_mod");
        let entry = dir.join("main.rk");
        fs::write(dir.join("util.rk"), "answer = -> 1\n< answer\n").unwrap();

        let root = Module {
            name: None,
            top_levels: vec![TopLevel::Module(ModuleDecl(Module {
                name: Some(Ident { context: Default::default(),
                    name: "outer".to_string(),
                    span: crate::lexer::Span::test(),
                }),
                top_levels: vec![TopLevel::Mod(
                    Ident { context: Default::default(),
                        name: "util".to_string(),
                        span: crate::lexer::Span::test(),
                    },
                    false,
                )],
                is_inline: true,
                filepath: None,
            }))],
            is_inline: true,
            filepath: Some(entry.clone()),
        };
        let mut graph = ModuleGraph::new(entry.clone());
        graph.root_module = root.clone();
        let mut errors = Vec::new();
        let mut stack = Vec::new();

        let mut db = SourceDatabase::new();
        db.load_children(
            &root,
            None,
            &no_std_config(entry.clone()),
            &mut stack,
            &mut graph,
            &mut errors,
            &mut |_, _, _| Ok(()),
        );

        assert!(errors.is_empty(), "expected no load errors, got {errors:?}");

        let util = graph
            .module_by_qualified_name("outer::util")
            .expect("nested util module should be loaded");
        assert_eq!(util.path, dir.join("util.rk"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cached_module_hit_still_loads_children_on_reused_database() {
        let dir = temp_dir("cached_children");
        let entry = dir.join("main.rk");
        fs::write(&entry, "mod parent\nmain = -> 0\n").unwrap();
        fs::write(dir.join("parent.rk"), "mod child\n").unwrap();

        let mut db = SourceDatabase::new();
        db.load_entry(entry.clone(), &no_std_config(entry.clone()))
            .expect_err("missing child should fail first load");

        fs::write(dir.join("child.rk"), "answer = -> 1\n< answer\n").unwrap();
        let graph = db
            .load_entry(entry.clone(), &no_std_config(entry.clone()))
            .expect("second load should discover child");

        let child = graph
            .module_by_qualified_name("parent::child")
            .expect("child module should be loaded from cached parent traversal");
        assert_eq!(child.path, dir.join("child.rk"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_entry_graph_loaded_files_are_isolated_per_load() {
        let dir = temp_dir("graph_loaded_files");
        let first = dir.join("first.rk");
        let second = dir.join("second.rk");
        fs::write(&first, "main = -> 0\n").unwrap();
        fs::write(&second, "main = -> 1\n").unwrap();

        let mut db = SourceDatabase::new();
        let first_graph = db
            .load_entry(first.clone(), &no_std_config(first.clone()))
            .expect("first entry should load");
        let second_graph = db
            .load_entry(second.clone(), &no_std_config(second.clone()))
            .expect("second entry should load");

        assert_eq!(first_graph.loaded_files(), &[first.clone()]);
        assert_eq!(second_graph.loaded_files(), &[second.clone()]);
        assert_eq!(db.loaded_files(), &[first.clone(), second.clone()]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn source_module_set_lookup_uses_only_preindexed_paths() {
        let dir = temp_dir("source_module_set_no_canonicalize");
        let real_path = dir.join("real.rk");
        let symlink_path = dir.join("link.rk");
        fs::write(&real_path, "main = -> 0\n").unwrap();
        std::os::unix::fs::symlink(&real_path, &symlink_path).unwrap();

        let module = Module {
            name: None,
            top_levels: Vec::new(),
            is_inline: false,
            filepath: Some(real_path.clone()),
        };
        let source_modules = SourceModuleSet::from_modules([LoadedModule {
            id: ModuleId(0),
            qualified_name: "demo".to_string(),
            path: real_path.clone(),
            canonical_path: real_path.canonicalize().unwrap(),
            module,
        }]);

        assert!(source_modules.module_for_path(&real_path).is_some());
        assert!(
            source_modules.module_for_path(&symlink_path).is_none(),
            "lower-facing source module lookup must not canonicalize miss paths"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_entry_file_reports_io_error() {
        let dir = temp_dir("missing_entry");
        let entry = dir.join("main.rk");

        let mut db = SourceDatabase::new();
        let errors = db
            .load_entry(entry.clone(), &no_std_config(entry.clone()))
            .expect_err("missing entry should report an IO error");

        match &errors[..] {
            [SourceLoadError::Io { path, message }] => {
                assert_eq!(path, &entry);
                assert!(!message.is_empty());
            }
            other => panic!("expected one IO error, got {other:?}"),
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_error_preserves_source_path() {
        let dir = temp_dir("parse_error");
        let entry = dir.join("main.rk");
        fs::write(&entry, "main = ->\n").unwrap();

        let mut db = SourceDatabase::new();
        let errors = db
            .load_entry(entry.clone(), &no_std_config(entry.clone()))
            .expect_err("invalid source should report a parse error");

        match &errors[..] {
            [SourceLoadError::Parse {
                path,
                source,
                error: _,
            }] => {
                assert_eq!(path, &entry);
                assert!(source.is_some());
            }
            other => panic!("expected one parse error, got {other:?}"),
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_module_reports_both_searched_paths() {
        let dir = temp_dir("missing_module");
        let entry = dir.join("main.rk");
        fs::write(&entry, "mod missing\nmain = -> 0\n").unwrap();

        let mut db = SourceDatabase::new();
        let errors = db
            .load_entry(entry.clone(), &no_std_config(entry.clone()))
            .expect_err("missing module should fail");

        match &errors[..] {
            [SourceLoadError::MissingModule {
                module,
                searched,
                span,
                parent_source,
            }] => {
                assert_eq!(module, "missing");
                assert_eq!(
                    searched,
                    &vec![dir.join("missing.rk"), dir.join("missing").join("mod.rk")]
                );
                assert!(span.is_some());
                assert_eq!(
                    parent_source.as_ref().map(|source| source.text.as_str()),
                    Some("mod missing\nmain = -> 0\n")
                );
            }
            other => panic!("expected one missing module error, got {other:?}"),
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn circular_module_load_reports_stack() {
        let dir = temp_dir("cycle");
        let entry = dir.join("main.rk");
        fs::write(&entry, "mod a\nmain = -> 0\n").unwrap();
        fs::write(dir.join("a.rk"), "mod b\n").unwrap();
        fs::write(dir.join("b.rk"), "mod a\n").unwrap();

        let mut db = SourceDatabase::new();
        let errors = db
            .load_entry(entry.clone(), &no_std_config(entry.clone()))
            .expect_err("cycle should fail");

        assert!(errors.iter().any(|error| match error {
            SourceLoadError::CircularModule { path, stack } => {
                path.ends_with(Path::new("a.rk"))
                    && stack.iter().any(|entry| entry.ends_with(Path::new("b.rk")))
            }
            _ => false,
        }));

        let _ = fs::remove_dir_all(&dir);
    }
}

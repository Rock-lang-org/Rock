use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::ast;
use crate::collect::item_index::{ItemIndex, ModuleKind};
use crate::ids::ModuleId;
use crate::lower::Lowerer;
use crate::source_loader::SourceModuleSet;

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

#[derive(Debug, Default)]
pub(crate) struct ModuleLoweringContext {
    file_path: PathBuf,
    current_crate_name: Option<String>,
    current_qualified_module_prefix: Option<String>,
    current_module_path: PathBuf,
    loaded_modules: HashSet<PathBuf>,
    loaded_module_ids: HashMap<String, Option<ModuleId>>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct ModuleSourceProvider {
    modules: SourceModuleSet,
}

pub(crate) struct SourceModuleResolver<'a> {
    modules: &'a mut crate::lower::services::LowerModuleService,
}

impl<'a> SourceModuleResolver<'a> {
    pub(crate) fn new(modules: &'a mut crate::lower::services::LowerModuleService) -> Self {
        Self { modules }
    }

    pub(crate) fn load_module_by_path(
        &mut self,
        module_path: &[String],
    ) -> Result<(ast::Module, String), String> {
        if module_path.is_empty() {
            return Err("Empty module path".to_string());
        }

        let is_external_crate = self.modules.has_loaded_root_name(&module_path[0]);

        let (mut module, mut resolved_prefix, start_index) = if is_external_crate {
            let crate_name = &module_path[0];
            if module_path.len() == 1 {
                let lib_path = self
                    .modules
                    .source_root_path(crate_name)
                    .ok_or_else(|| format!("Unknown crate {}", crate_name))?;
                let module = self
                    .modules
                    .source_module_for_path(&lib_path)
                    .ok_or_else(|| {
                        format!(
                            "Crate root '{}' was not loaded by the source database; expected {}",
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
                .modules
                .current_module_prefix()
                .map(|prefix| format!("{}::{}", prefix, module_name))
                .unwrap_or_else(|| module_name.clone());
            let module = self.load_local_module(module_name)?;
            (module, resolved_prefix, 1)
        };

        for segment in &module_path[start_index..] {
            let next_prefix = format!("{}::{}", resolved_prefix, segment);
            if let Some(loaded_module) = self.modules.source_module_for_qualified_name(&next_prefix)
            {
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

    pub(crate) fn load_local_module(&mut self, module_name: &str) -> Result<ast::Module, String> {
        let qualified_module_name = self
            .modules
            .current_module_prefix()
            .map(|prefix| format!("{}::{}", prefix, module_name))
            .unwrap_or_else(|| module_name.to_string());
        let Some(file_path) = self
            .modules
            .source_path_for_module_name(&qualified_module_name)
        else {
            return Err(format!(
                "Module '{}' was not loaded by the source database",
                qualified_module_name
            ));
        };

        if let Some(cached) = self.modules.source_module_for_path(&file_path) {
            self.modules.mark_loaded(&file_path);
            return Ok(cached);
        }

        Err(format!(
            "Module '{}' was not loaded by the source database; expected {}",
            module_name,
            file_path.display()
        ))
    }

    pub(crate) fn load_external_crate_module(
        &mut self,
        module_name: &str,
        crate_name: &str,
    ) -> Result<ast::Module, String> {
        let qualified_module_name = format!("{}::{}", crate_name, module_name);
        let Some(file_path) = self
            .modules
            .source_path_for_module_name(&qualified_module_name)
        else {
            return Err(format!(
                "Module '{}' was not loaded by the source database",
                qualified_module_name
            ));
        };

        if let Some(cached) = self.modules.source_module_for_path(&file_path) {
            self.modules.mark_loaded(&file_path);
            return Ok(cached);
        }

        Err(format!(
            "Module '{}::{}' was not loaded by the source database; expected {}",
            crate_name,
            module_name,
            file_path.display()
        ))
    }
}

impl ModuleLoweringContext {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn configure_current_crate(
        &mut self,
        crate_name: Option<&str>,
        root_file_path: Option<&Path>,
    ) {
        self.current_crate_name = crate_name.map(ToString::to_string);

        if let Some(file_path) = root_file_path {
            self.file_path = file_path.to_path_buf();
            self.current_module_path = file_path.to_path_buf();
        }
    }

    pub(crate) fn associate_current_crate_module_ids(&mut self, item_index: &ItemIndex) {
        self.loaded_module_ids.clear();

        let Some(root_module_id) = item_index
            .modules()
            .iter()
            .find(|module| module.kind == ModuleKind::Root)
            .map(|module| module.module_id)
        else {
            return;
        };
        let mut paths = HashMap::from([(root_module_id, Vec::new())]);

        while paths.len() < item_index.modules().len() {
            let mut inserted = false;
            for module in item_index.modules() {
                let (Some(parent), Some(name)) = (module.parent, module.name.as_ref()) else {
                    continue;
                };
                let Some(parent_path) = paths.get(&parent).cloned() else {
                    continue;
                };
                let mut path = parent_path;
                path.push(name.clone());
                inserted |= paths.insert(module.module_id, path).is_none();
            }
            if !inserted {
                break;
            }
        }

        for (module_id, path) in paths {
            let relative_name = path.join("::");
            if !relative_name.is_empty() {
                self.record_loaded_module_id(relative_name.clone(), module_id);
            }
            if let Some(crate_name) = &self.current_crate_name {
                let qualified_name = if relative_name.is_empty() {
                    crate_name.clone()
                } else {
                    format!("{}::{}", crate_name, relative_name)
                };
                self.record_loaded_module_id(qualified_name, module_id);
            }
        }
    }

    fn record_loaded_module_id(&mut self, qualified_name: String, module_id: ModuleId) {
        match self.loaded_module_ids.get(&qualified_name).copied() {
            None => {
                self.loaded_module_ids
                    .insert(qualified_name, Some(module_id));
            }
            Some(Some(previous_id)) if previous_id != module_id => {
                self.loaded_module_ids.insert(qualified_name, None);
            }
            Some(_) => {}
        }
    }

    fn loaded_module_id(&self, qualified_name: &str) -> Result<ModuleId, &'static str> {
        match self.loaded_module_ids.get(qualified_name) {
            Some(Some(module_id)) => Ok(*module_id),
            Some(None) => Err("ambiguous indexed module identity"),
            None => Err("missing indexed module identity"),
        }
    }

    pub(crate) fn current_crate_name(&self) -> Option<&str> {
        self.current_crate_name.as_deref()
    }

    #[allow(dead_code)]
    pub(crate) fn set_current_crate_name(&mut self, crate_name: Option<String>) {
        self.current_crate_name = crate_name;
    }

    pub(crate) fn root_file_path(&self) -> Option<&Path> {
        (!self.file_path.as_os_str().is_empty()).then_some(self.file_path.as_path())
    }

    #[allow(dead_code)]
    pub(crate) fn current_module_path(&self) -> &Path {
        self.current_module_path.as_path()
    }

    pub(crate) fn set_current_module_path(&mut self, path: PathBuf) {
        self.current_module_path = path;
    }

    pub(crate) fn replace_current_module_path(&mut self, path: PathBuf) -> PathBuf {
        std::mem::replace(&mut self.current_module_path, path)
    }

    #[allow(dead_code)]
    pub(crate) fn with_current_module_path<F, R>(&mut self, path: PathBuf, visit: F) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        let previous_path = self.replace_current_module_path(path);
        let result = visit(self);
        self.current_module_path = previous_path;
        result
    }

    #[allow(dead_code)]
    pub(crate) fn set_current_qualified_module_prefix(&mut self, prefix: Option<String>) {
        self.current_qualified_module_prefix = prefix;
    }

    pub(crate) fn replace_qualified_module_prefix(
        &mut self,
        prefix: Option<&str>,
    ) -> Option<String> {
        match prefix {
            Some(prefix) => self
                .current_qualified_module_prefix
                .replace(prefix.to_string()),
            None => self.current_qualified_module_prefix.take(),
        }
    }

    pub(crate) fn restore_qualified_module_prefix(&mut self, previous_prefix: Option<String>) {
        self.current_qualified_module_prefix = previous_prefix;
    }

    #[allow(dead_code)]
    pub(crate) fn with_qualified_module_prefix<F, R>(&mut self, prefix: Option<&str>, visit: F) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        let previous_prefix = self.replace_qualified_module_prefix(prefix);
        let result = visit(self);
        self.current_qualified_module_prefix = previous_prefix;
        result
    }

    pub(crate) fn current_module_prefix(
        &self,
        source_provider: &ModuleSourceProvider,
    ) -> Option<String> {
        if let Some(prefix) = &self.current_qualified_module_prefix {
            return Some(prefix.clone());
        }

        source_provider
            .modules()
            .find(|module| {
                module.path == self.current_module_path
                    || module.canonical_path == self.current_module_path
            })
            .map(|module| module.qualified_name.clone())
            .or_else(|| self.current_crate_name.clone())
    }

    pub(crate) fn source_path_for_module_name(
        &self,
        source_provider: &ModuleSourceProvider,
        qualified_module_name: &str,
    ) -> Option<PathBuf> {
        self.loaded_path_for_current_crate_module_name(source_provider, qualified_module_name)
            .or_else(|| source_provider.source_path_for_exact_module_name(qualified_module_name))
    }

    fn loaded_path_for_current_crate_module_name(
        &self,
        source_provider: &ModuleSourceProvider,
        qualified_module_name: &str,
    ) -> Option<PathBuf> {
        let crate_name = self.current_crate_name.as_ref()?;
        if qualified_module_name == crate_name
            || qualified_module_name.starts_with(&format!("{}::", crate_name))
        {
            return None;
        }

        let first_segment = qualified_module_name
            .split("::")
            .next()
            .unwrap_or(qualified_module_name);
        if source_provider.has_loaded_root_name(first_segment) {
            return None;
        }

        source_provider.source_path_for_exact_module_name(&format!(
            "{}::{}",
            crate_name, qualified_module_name
        ))
    }

    pub(crate) fn mark_loaded(&mut self, path: &Path) {
        self.loaded_modules.insert(path.to_path_buf());
    }

    pub(crate) fn same_path(left: &Path, right: &Path) -> bool {
        left == right
    }

    pub(crate) fn should_skip_loaded_module_path(&self, file_path: &Path) -> bool {
        self.root_file_path()
            .is_some_and(|root_path| Self::same_path(file_path, root_path))
    }

    pub(crate) fn for_each_loaded_module<F>(lowerer: &mut Lowerer, mut visit: F)
    where
        F: FnMut(&mut Lowerer, ModuleId, &str, &ast::Module),
    {
        let source_modules = lowerer.modules.source_provider().clone();

        for loaded_module in source_modules.modules() {
            let module_name = &loaded_module.qualified_name;
            let file_path = &loaded_module.path;
            if lowerer.modules.should_skip_loaded_module_path(file_path) {
                continue;
            }

            let module_id = match lowerer.modules.loaded_module_id(module_name) {
                Ok(module_id) => module_id,
                Err(reason) => {
                    lowerer.diagnostics.push_toolchain(format!(
                        "{} for loaded module '{}' while lowering bodies",
                        reason, module_name
                    ));
                    continue;
                }
            };

            let old_path = lowerer
                .modules
                .replace_current_module_path(file_path.clone());
            visit(lowerer, module_id, module_name, &loaded_module.module);
            lowerer.modules.set_current_module_path(old_path);
        }
    }

    pub(crate) fn with_qualified_module_context<F>(
        lowerer: &mut Lowerer,
        module: &ast::Module,
        module_prefix: Option<&str>,
        visit: F,
    ) where
        F: FnOnce(&mut Lowerer),
    {
        let previous_prefix = lowerer
            .modules
            .replace_qualified_module_prefix(module_prefix);

        match module_prefix {
            Some(prefix) => {
                Self::with_module_local_aliases(lowerer, module, prefix, true, true, visit)
            }
            None => {
                if !Self::has_root_glob_imports(module) {
                    visit(lowerer);
                    lowerer
                        .modules
                        .restore_qualified_module_prefix(previous_prefix);
                    return;
                }
                let prefix = lowerer.modules.current_module_prefix().unwrap_or_default();
                Self::with_module_local_aliases(lowerer, module, &prefix, false, true, visit)
            }
        }

        lowerer
            .modules
            .restore_qualified_module_prefix(previous_prefix);
    }

    fn has_root_glob_imports(module: &ast::Module) -> bool {
        module
            .top_levels
            .iter()
            .any(|top_level| matches!(top_level, ast::TopLevel::GlobImport(_)))
    }

    pub(crate) fn with_module_local_aliases<F>(
        lowerer: &mut Lowerer,
        module: &ast::Module,
        module_prefix: &str,
        include_local_items: bool,
        record_resolver_aliases: bool,
        visit: F,
    ) where
        F: FnOnce(&mut Lowerer),
    {
        lowerer.push_scope();
        lowerer.inject_module_local_aliases(
            module,
            module_prefix,
            include_local_items,
            record_resolver_aliases,
        );

        visit(lowerer);
        lowerer.pop_scope();
    }
}

impl ModuleSourceProvider {
    pub(crate) fn new(source_modules: impl Into<SourceModuleSet>) -> Self {
        Self {
            modules: source_modules.into(),
        }
    }

    pub(crate) fn empty() -> Self {
        Self::default()
    }

    pub(crate) fn modules(&self) -> impl Iterator<Item = &crate::source_loader::LoadedModule> {
        self.modules.modules()
    }

    #[allow(dead_code)]
    pub(crate) fn source_module_paths(&self) -> Vec<(String, PathBuf)> {
        self.modules
            .modules()
            .map(|module| (module.qualified_name.clone(), module.path.clone()))
            .collect()
    }

    pub(crate) fn loaded_module_paths(&self) -> Vec<(String, PathBuf)> {
        self.modules.loaded_module_paths()
    }

    pub(crate) fn source_module_for_qualified_name(
        &self,
        modules: &ModuleLoweringContext,
        qualified_name: &str,
    ) -> Option<ast::Module> {
        modules
            .source_path_for_module_name(self, qualified_name)
            .and_then(|path| self.source_module_for_path(&path))
    }

    pub(crate) fn source_module_for_path(&self, path: &Path) -> Option<ast::Module> {
        self.modules
            .module_for_path(path)
            .map(|module| module.module.clone())
    }

    pub(crate) fn source_path_for_exact_module_name(
        &self,
        qualified_module_name: &str,
    ) -> Option<PathBuf> {
        self.modules
            .module_by_qualified_name(qualified_module_name)
            .map(|module| module.path.clone())
    }

    pub(crate) fn has_loaded_root_name(&self, root_name: &str) -> bool {
        self.modules.has_root_name(root_name)
    }

    pub(crate) fn source_root_path(&self, root_name: &str) -> Option<PathBuf> {
        self.modules.root_path(root_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ast::{Ident, IdentOrType, IdentifierPath, Module, Path, TopLevel};
    use crate::collect::item_index::{index_root_module_items, IndexingIds};
    use crate::ids::{CrateId, DefId, LocalDefId, ModuleId};
    use crate::lexer::Span;
    use crate::lower::services::LowerModuleService;
    use crate::lower::Lowerer;
    use crate::source_loader::LoadedModule;
    use crate::types::Type;

    fn empty_module(path: &std::path::Path) -> Module {
        Module {
            name: Some(Ident { context: Default::default(),
                name: "io".to_string(),
                span: Span::test(),
            }),
            top_levels: Vec::new(),
            is_inline: false,
            filepath: Some(path.to_path_buf()),
        }
    }

    fn import_path(names: &[&str]) -> TopLevel {
        TopLevel::Import(Path::Ident(IdentifierPath {
            path: names
                .iter()
                .map(|name| {
                    IdentOrType::Ident(Ident { context: Default::default(),
                        name: name.to_string(),
                        span: Span::test(),
                    })
                })
                .collect(),
        }))
    }

    fn loaded_module(qualified_name: &str, path: PathBuf) -> LoadedModule {
        LoadedModule {
            id: crate::source_loader::ModuleId(0),
            qualified_name: qualified_name.to_string(),
            path: path.clone(),
            canonical_path: path.clone(),
            module: empty_module(&path),
        }
    }

    #[test]
    fn source_module_resolver_loads_modules_from_loaded_module_entries() {
        let root_path = PathBuf::from("/virtual/demo/main.rk");
        let local_path = PathBuf::from("/virtual/demo/foo.rk");
        let dep_root_path = PathBuf::from("/virtual/dep/lib.rk");
        let dep_util_path = PathBuf::from("/virtual/dep/util.rk");

        let mut modules = LowerModuleService::from_source_modules(vec![
            loaded_module("demo", root_path.clone()),
            loaded_module("demo::foo", local_path.clone()),
            loaded_module("dep", dep_root_path),
            loaded_module("dep::util", dep_util_path.clone()),
        ]);
        modules.configure_current_crate(Some("demo"), Some(&root_path));

        let (local, local_prefix) = SourceModuleResolver::new(&mut modules)
            .load_module_by_path(&["foo".to_string()])
            .expect("loaded local module entry should resolve");
        assert_eq!(local.filepath.as_ref(), Some(&local_path));
        assert_eq!(local_prefix, "demo::foo");

        let (external, external_prefix) = SourceModuleResolver::new(&mut modules)
            .load_module_by_path(&["dep".to_string(), "util".to_string()])
            .expect("loaded external module entry should resolve");
        assert_eq!(external.filepath.as_ref(), Some(&dep_util_path));
        assert_eq!(external_prefix, "dep::util");
    }

    #[test]
    fn source_module_resolver_loads_cached_local_and_external_modules_from_context() {
        let root_path = PathBuf::from("/virtual/demo/main.rk");
        let local_path = PathBuf::from("/virtual/demo/foo.rk");
        let dep_root_path = PathBuf::from("/virtual/dep/lib.rk");
        let dep_util_path = PathBuf::from("/virtual/dep/util.rk");

        let mut modules = LowerModuleService::from_source_modules(vec![
            loaded_module("demo", root_path.clone()),
            loaded_module("demo::foo", local_path.clone()),
            loaded_module("dep", dep_root_path),
            loaded_module("dep::util", dep_util_path.clone()),
        ]);
        modules.configure_current_crate(Some("demo"), Some(&root_path));

        let (local, local_prefix) = SourceModuleResolver::new(&mut modules)
            .load_module_by_path(&["foo".to_string()])
            .expect("cached local module should resolve");
        assert_eq!(local.filepath.as_ref(), Some(&local_path));
        assert_eq!(local_prefix, "demo::foo");

        let (external, external_prefix) = SourceModuleResolver::new(&mut modules)
            .load_module_by_path(&["dep".to_string(), "util".to_string()])
            .expect("cached external module should resolve from seeded paths");
        assert_eq!(external.filepath.as_ref(), Some(&dep_util_path));
        assert_eq!(external_prefix, "dep::util");
    }

    #[test]
    fn lowerer_exposes_module_backing_state_through_module_service() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_module_context_service_state_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let root_path = temp_dir.join("main.rk");
        let util_path = temp_dir.join("util.rk");

        let mut lowerer = Lowerer::new_for_test();
        lowerer.modules = LowerModuleService::from_source_modules(vec![loaded_module(
            "demo::util",
            util_path.clone(),
        )]);
        lowerer
            .modules
            .configure_current_crate(Some("demo"), Some(&root_path));

        assert_eq!(lowerer.modules.current_crate_name(), Some("demo"));
        assert_eq!(lowerer.modules.root_file_path(), Some(root_path.as_path()));
        assert_eq!(lowerer.modules.current_module_path(), root_path.as_path());
        assert_eq!(
            lowerer.modules.current_module_prefix(),
            Some("demo".to_string())
        );
        assert_eq!(
            lowerer.modules.source_path_for_module_name("util"),
            Some(util_path.clone())
        );
        assert!(lowerer
            .modules
            .source_module_for_qualified_name("util")
            .is_some());

        lowerer
            .modules
            .with_current_module_path(util_path.clone(), |modules| {
                assert_eq!(modules.current_module_path(), util_path.as_path());
            });

        assert_eq!(lowerer.modules.current_module_path(), root_path.as_path());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn module_context_prefers_current_crate_prefixed_graph_path() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_module_context_prefers_graph_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let alias_path = temp_dir.join("alias.rk");
        let graph_path = temp_dir.join("graph.rk");

        let mut lowerer = Lowerer::new_for_test();
        lowerer.modules = LowerModuleService::from_source_modules(vec![
            loaded_module("math::io", alias_path.clone()),
            loaded_module("test::math::io", graph_path.clone()),
        ]);
        lowerer
            .modules
            .set_current_crate_name(Some("test".to_string()));

        let path = lowerer
            .modules
            .source_path_for_module_name("math::io")
            .expect("graph path should resolve");

        assert_eq!(path, graph_path);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn module_context_does_not_use_cache_without_loaded_path() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_module_context_requires_graph_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let cached_path = temp_dir.join("util.rk");

        let mut lowerer = Lowerer::new_for_test();
        lowerer.modules = LowerModuleService::from_source_modules(vec![loaded_module(
            "other::util",
            cached_path.clone(),
        )]);

        assert!(lowerer
            .modules
            .source_path_for_module_name("util")
            .is_none());
        assert!(lowerer
            .modules
            .source_module_for_qualified_name("util")
            .is_none());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn module_context_skips_only_root_file_path() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_module_context_skips_root_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let root_path = temp_dir.join("main.rk");
        let module_path = temp_dir.join("util.rk");

        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .modules
            .configure_current_crate(Some("demo"), Some(&root_path));

        assert!(lowerer.modules.should_skip_loaded_module_path(&root_path));
        assert!(!lowerer.modules.should_skip_loaded_module_path(&module_path));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn module_context_iterates_loaded_modules_from_cache_and_skips_root() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_module_context_iter_loaded_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let root_path = temp_dir.join("main.rk");
        let child_path = temp_dir.join("child.rk");
        let child = empty_module(&child_path);

        let root = Module {
            name: None,
            top_levels: vec![TopLevel::Mod(
                Ident { context: Default::default(),
                    name: "child".to_string(),
                    span: Span::test(),
                },
                false,
            )],
            is_inline: false,
            filepath: Some(root_path.clone()),
        };
        let mut ids = IndexingIds::new_root();
        let mut lowerer = Lowerer::new_for_test();
        lowerer.item_index = index_root_module_items(&mut ids, &root);
        lowerer.modules = LowerModuleService::from_source_modules(vec![
            loaded_module("demo", root_path.clone()),
            LoadedModule {
                module: child,
                ..loaded_module("demo::child", child_path.clone())
            },
        ]);
        lowerer
            .modules
            .configure_current_crate(Some("demo"), Some(&root_path));
        lowerer
            .modules
            .associate_current_crate_module_ids(&lowerer.item_index);

        let mut seen = Vec::new();
        ModuleLoweringContext::for_each_loaded_module(
            &mut lowerer,
            |_, module_id, module_name, module| {
                seen.push((module_id, module_name.to_string(), module.filepath.clone()));
            },
        );

        assert_eq!(
            seen,
            vec![(ModuleId(1), "demo::child".to_string(), Some(child_path),)]
        );
        assert!(
            lowerer.errors().is_empty(),
            "unexpected errors: {:?}",
            lowerer.errors()
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn module_context_owns_loaded_root_name_lookup() {
        let temp_dir = std::env::temp_dir().join(format!(
            "rock_module_context_loaded_root_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let root_path = temp_dir.join("lib.rk");

        let mut lowerer = Lowerer::new_for_test();
        lowerer.modules =
            LowerModuleService::from_source_modules(vec![loaded_module("dep", root_path.clone())]);

        assert!(lowerer.modules.has_loaded_root_name("dep"));
        assert_eq!(lowerer.modules.source_root_path("dep"), Some(root_path));
        assert!(!lowerer.modules.has_loaded_root_name("missing"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn module_context_scopes_module_local_aliases_and_prefix() {
        let parsed =
            crate::parser::parse_string("answer: I64\nanswer = -> 1\n", &crate::Config::default())
                .unwrap();
        let module = parsed.module;

        let mut lowerer = Lowerer::new_for_test();
        let answer_id = DefId::new(CrateId(0), LocalDefId(1));
        lowerer
            .resolver
            .item_paths
            .insert("demo::helper::answer".to_string(), answer_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(answer_id, "demo::helper::answer".to_string());
        lowerer
            .resolver
            .scoped_module_aliases
            .entry("demo::helper".to_string())
            .or_default()
            .insert("answer".to_string(), answer_id);
        lowerer.scope.define(
            "demo::helper::answer".to_string(),
            Type::function(Vec::new(), Type::I64),
            false,
        );
        lowerer
            .modules
            .set_current_qualified_module_prefix(Some("old".to_string()));

        ModuleLoweringContext::with_qualified_module_context(
            &mut lowerer,
            &module,
            Some("demo::helper"),
            |lowerer| {
                assert_eq!(
                    lowerer.modules.current_module_prefix(),
                    Some("demo::helper".to_string())
                );
                assert_eq!(
                    crate::lower::resolution::LowerResolutionContext::new(lowerer)
                        .resolve_module_alias_or_item_id("answer"),
                    Some(answer_id)
                );
                assert!(!lowerer.resolver.module_aliases.contains_key("answer"));
                assert!(lowerer.scope.lookup("answer").is_some());
            },
        );

        assert_eq!(
            lowerer.modules.current_module_prefix(),
            Some("old".to_string())
        );
        assert!(!lowerer.resolver.module_aliases.contains_key("answer"));
        assert!(lowerer.scope.lookup("answer").is_none());
    }

    #[test]
    fn module_context_duplicate_module_aliases_use_scoped_resolver_output() {
        let module = Module {
            name: Some(Ident { context: Default::default(),
                name: "helper".to_string(),
                span: Span::test(),
            }),
            top_levels: vec![
                import_path(&["demo", "helper", "answer"]),
                import_path(&["demo", "helper", "answer"]),
            ],
            is_inline: false,
            filepath: None,
        };

        let previous_id = DefId::new(CrateId(0), LocalDefId(1));
        let answer_id = DefId::new(CrateId(0), LocalDefId(2));
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .resolver
            .module_aliases
            .insert("answer".to_string(), previous_id);
        lowerer
            .resolver
            .item_paths
            .insert("demo::helper::answer".to_string(), answer_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(answer_id, "demo::helper::answer".to_string());
        lowerer
            .resolver
            .scoped_module_aliases
            .entry("demo::helper".to_string())
            .or_default()
            .insert("answer".to_string(), answer_id);
        lowerer.scope.define(
            "demo::helper::answer".to_string(),
            Type::function(Vec::new(), Type::I64),
            false,
        );
        lowerer
            .modules
            .set_current_qualified_module_prefix(Some("demo::helper".to_string()));

        ModuleLoweringContext::with_module_local_aliases(
            &mut lowerer,
            &module,
            "demo::helper",
            true,
            true,
            |lowerer| {
                assert_eq!(
                    crate::lower::resolution::LowerResolutionContext::new(lowerer)
                        .resolve_module_alias_or_item_id("answer"),
                    Some(answer_id)
                );
                assert_eq!(
                    lowerer.resolver.module_aliases.get("answer").copied(),
                    Some(previous_id)
                );
            },
        );

        assert_eq!(
            lowerer.resolver.module_aliases.get("answer").copied(),
            Some(previous_id)
        );
    }
}

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
mod tests;

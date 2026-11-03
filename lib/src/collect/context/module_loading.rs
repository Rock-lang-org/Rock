use super::exports::find_inline_module;
use super::CollectContext;
use crate::ast;
use std::path::PathBuf;

impl CollectContext {
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
}

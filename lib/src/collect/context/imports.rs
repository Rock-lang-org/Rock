use super::exports::collect_exports;
use super::type_names::path_names;
use super::CollectContext;
use crate::ast;
use crate::crate_artifact::ArtifactExport;
use crate::types::Type;
use std::collections::HashMap;

/// What an imported name resolved to.
pub(super) enum ImportTarget {
    /// A function or binding: it also needs a scope entry with this type.
    Value(Type),
    /// A struct, enum, type alias or trait.
    TypeItem,
}

impl CollectContext {
    pub(crate) fn handle_import(
        &mut self,
        path: &ast::Path,
        record_canonical_aliases: bool,
        canonical_prefix: Option<&str>,
    ) {
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

        if let Some(target) = self.lookup_import_target(&qualified_name) {
            let export = self.export_for_import_source(&qualified_name);
            let explicit_target =
                (is_explicit && record_canonical_aliases).then_some(qualified_name.as_str());
            self.bind_import(
                &short_name,
                target,
                &qualified_name,
                explicit_target,
                export,
                canonical_prefix,
            );
            return;
        }

        if let Some(canonical_qualified_name) = canonical_qualified_name {
            if let Some(target) = self.lookup_import_target(&canonical_qualified_name) {
                let export = self.export_for_registered_item(&canonical_qualified_name);
                let explicit_target = is_explicit.then_some(canonical_qualified_name.as_str());
                self.bind_import(
                    &short_name,
                    target,
                    &canonical_qualified_name,
                    explicit_target,
                    export,
                    canonical_prefix,
                );
            }
        }
    }

    /// Finds what `name` refers to among the registered declarations.
    pub(super) fn lookup_import_target(&self, name: &str) -> Option<ImportTarget> {
        if let Some(func) = self.functions.get(name) {
            let param_types: Vec<Type> = func.params.iter().map(|param| param.ty.clone()).collect();
            return Some(ImportTarget::Value(Type::function_with_safety(
                param_types,
                func.ret_type.clone(),
                crate::types::FunctionSafety::from_is_unsafe(func.is_unsafe),
            )));
        }

        if let Some(binding) = self.scope.lookup(name) {
            return Some(ImportTarget::Value(binding.ty.clone()));
        }

        let is_type_item = self.structs.contains_key(name)
            || self.enums.contains_key(name)
            || self.type_aliases.contains_key(name)
            || self.traits.contains_key(name);
        is_type_item.then_some(ImportTarget::TypeItem)
    }

    /// Records every alias table entry for `short_name` once its target is known.
    /// `explicit_target` is `Some` when the import must also land in the explicit alias table.
    pub(super) fn bind_import(
        &mut self,
        short_name: &str,
        target: ImportTarget,
        alias_target: &str,
        explicit_target: Option<&str>,
        export: Option<ArtifactExport>,
        canonical_prefix: Option<&str>,
    ) {
        if let ImportTarget::Value(ty) = target {
            self.scope.define(short_name.to_string(), ty, false);
        }
        self.import_aliases
            .insert(short_name.to_string(), alias_target.to_string());
        if let Some(explicit_target) = explicit_target {
            self.explicit_import_aliases
                .insert(short_name.to_string(), explicit_target.to_string());
            if let Some(export) = &export {
                self.record_canonical_alias(short_name.to_string(), export);
            }
        }
        if let Some(export) = &export {
            self.record_resolved_import_alias(canonical_prefix, short_name, export);
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

use super::exports::collect_exports;
use super::type_names::path_names;
use super::CollectContext;
use crate::ast;
use crate::crate_artifact::ArtifactExport;
use crate::types::Type;
use std::collections::HashMap;

/// What an imported name resolved to.
pub(crate) enum ImportTarget {
    /// A function or binding: it also needs a scope entry with this type.
    Value(Type),
    /// A struct, enum, type alias or trait.
    TypeItem,
}

/// Where an import resolved, before anything is bound.
pub(crate) enum ImportResolution {
    /// A dependency or re-exported item, bound through its artifact export.
    ArtifactExport(ArtifactExport),
    /// A declaration registered under the path as written.
    Declared(ImportTarget),
    /// A declaration registered under the canonical form of the path.
    CanonicalDeclared(String, ImportTarget),
}

impl CollectContext {
    /// Imports `path`. Returns `false` when nothing matches it yet.
    pub(crate) fn handle_import(
        &mut self,
        path: &ast::Path,
        record_canonical_aliases: bool,
        canonical_prefix: Option<&str>,
    ) -> bool {
        let names = path_names(path.segments());

        if names.is_empty() {
            return false;
        }

        let qualified_name = names.join("::");
        let short_name = names.last().cloned().unwrap_or_default();
        self.import_qualified_name(
            short_name,
            qualified_name,
            true,
            record_canonical_aliases,
            canonical_prefix,
        )
    }

    /// Binds `short_name` to the item at `qualified_name`. Returns `false` when no
    /// declaration matches it yet.
    pub(crate) fn import_qualified_name(
        &mut self,
        short_name: String,
        qualified_name: String,
        is_explicit: bool,
        record_canonical_aliases: bool,
        canonical_prefix: Option<&str>,
    ) -> bool {
        let Some(resolution) =
            self.resolve_import(&qualified_name, record_canonical_aliases, canonical_prefix)
        else {
            return false;
        };

        match resolution {
            ImportResolution::ArtifactExport(export) => self.import_artifact_root_export(
                short_name,
                qualified_name,
                export,
                is_explicit,
                record_canonical_aliases,
                canonical_prefix,
            ),
            ImportResolution::Declared(target) => {
                let export = self.export_for_import_source(&qualified_name);
                self.bind_import(
                    &short_name,
                    target,
                    &qualified_name,
                    is_explicit && record_canonical_aliases,
                    export,
                    canonical_prefix,
                );
            }
            ImportResolution::CanonicalDeclared(canonical_qualified_name, target) => {
                let export = self.export_for_registered_item(&canonical_qualified_name);
                self.bind_import(
                    &short_name,
                    target,
                    &canonical_qualified_name,
                    is_explicit,
                    export,
                    canonical_prefix,
                );
            }
        }
        true
    }

    /// Finds what an import of `qualified_name` refers to, without binding anything.
    pub(crate) fn resolve_import(
        &self,
        qualified_name: &str,
        record_canonical_aliases: bool,
        canonical_prefix: Option<&str>,
    ) -> Option<ImportResolution> {
        let canonical_qualified_name = canonical_prefix
            .map(|prefix| self.qualify_import_source(qualified_name, Some(prefix)))
            .or_else(|| {
                record_canonical_aliases
                    .then(|| self.qualify_import_source(qualified_name, canonical_prefix))
            });

        if let Some(export) = self.artifact_root_export_for_import(qualified_name) {
            return Some(ImportResolution::ArtifactExport(export));
        }

        if let Some(export) = canonical_qualified_name
            .as_ref()
            .and_then(|name| self.export_aliases.get(name))
            .or_else(|| self.export_aliases.get(qualified_name))
            .and_then(|export_source| self.export_for_registered_item(export_source))
        {
            return Some(ImportResolution::ArtifactExport(export));
        }

        if let Some(target) = self.lookup_import_target(qualified_name) {
            return Some(ImportResolution::Declared(target));
        }

        let canonical_qualified_name = canonical_qualified_name?;
        let target = self.lookup_import_target(&canonical_qualified_name)?;
        Some(ImportResolution::CanonicalDeclared(
            canonical_qualified_name,
            target,
        ))
    }

    /// Finds what `name` refers to among the registered declarations.
    pub(super) fn lookup_import_target(&self, name: &str) -> Option<ImportTarget> {
        if let Some(func) = self.functions.get(name) {
            return Some(ImportTarget::Value(func.fn_type()));
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
    /// `explicit` imports replace any alias the prelude or a glob import already recorded.
    pub(super) fn bind_import(
        &mut self,
        short_name: &str,
        target: ImportTarget,
        alias_target: &str,
        explicit: bool,
        export: Option<ArtifactExport>,
        canonical_prefix: Option<&str>,
    ) {
        if let ImportTarget::Value(ty) = target {
            self.scope.define(short_name.to_string(), ty, false);
        }
        self.import_aliases
            .insert(short_name.to_string(), alias_target.to_string());
        if let Some(export) = &export {
            if explicit {
                self.record_explicit_canonical_alias(short_name.to_string(), export);
            }
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

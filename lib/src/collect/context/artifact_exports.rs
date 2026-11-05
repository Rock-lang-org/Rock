use super::CollectContext;
use crate::crate_artifact::ArtifactExport;
use crate::hir::HirTrait;
use crate::ids::DefId;

impl CollectContext {
    pub(crate) fn trait_by_name(&self, name: &str) -> Option<&HirTrait> {
        if let Some(trait_def) = self.traits.get(name) {
            return Some(trait_def);
        }

        let mut by_source_name = self
            .traits
            .values()
            .filter(|trait_def| trait_def.name == name);
        if let Some(candidate) = by_source_name.next() {
            if by_source_name.all(|other| other.id == candidate.id) {
                return Some(candidate);
            }
        }

        if let Some(id) = self.names.canonical_alias(name) {
            if let Some(trait_def) = self.traits.values().find(|trait_def| trait_def.id == id) {
                return Some(trait_def);
            }
        }

        self.names
            .import_alias(name)
            .and_then(|qualified| self.traits.get(qualified))
    }

    pub(super) fn record_canonical_alias(&mut self, alias: String, export: &ArtifactExport) {
        self.record_canonical_alias_inner(alias, export, false);
    }

    /// Like `record_canonical_alias`, but replaces an existing entry: an explicit
    /// import must shadow a name the prelude or a glob import brought in.
    pub(super) fn record_explicit_canonical_alias(
        &mut self,
        alias: String,
        export: &ArtifactExport,
    ) {
        self.record_canonical_alias_inner(alias, export, true);
    }

    fn record_canonical_alias_inner(
        &mut self,
        alias: String,
        export: &ArtifactExport,
        overwrite: bool,
    ) {
        // Always record the alias by DefId, local items included, so the resolver
        // never depends on re-resolving a path string later.
        self.names.record_canonical_alias(alias, export.id, overwrite);

        // Local items register their own canonical name when declared; the import
        // path may not be canonical, so only foreign items take their name from here.
        let first_segment = export.source.split("::").next().unwrap_or(&export.source);
        if self
            .current_crate_name
            .as_deref()
            .is_some_and(|crate_name| crate_name == first_segment || !export.source.contains("::"))
        {
            return;
        }

        self.names
            .default_canonical_name(export.id, export.source.clone());
    }

    pub(super) fn record_canonical_item_name(&mut self, id: DefId, name: &str) {
        if let Some(existing_name) = self.names.canonical_name(id) {
            if existing_name != name {
                let message = format!(
                    "conflicting canonical declaration names for DefId {id:?}: '{existing_name}' and '{name}'"
                );
                self.push_error(message);
            }
            return;
        }

        self.names.set_canonical_name(id, name.to_string());
    }

    fn record_scoped_module_alias(
        &mut self,
        module_prefix: Option<&str>,
        alias: String,
        id: DefId,
    ) {
        let Some(module_prefix) = module_prefix.filter(|prefix| !prefix.is_empty()) else {
            return;
        };

        self.names
            .record_scoped_module_alias(module_prefix, alias, id);
    }

    pub(super) fn record_resolved_import_alias(
        &mut self,
        module_prefix: Option<&str>,
        short_name: &str,
        export: &ArtifactExport,
    ) {
        let resolver_alias = module_prefix.map_or_else(
            || short_name.to_string(),
            |prefix| format!("{}::{}", prefix, short_name),
        );
        self.record_canonical_alias(resolver_alias, export);
        self.record_scoped_module_alias(module_prefix, short_name.to_string(), export.id);
    }

    pub(crate) fn export_for_registered_item(
        &self,
        qualified_name: &str,
    ) -> Option<ArtifactExport> {
        if let Some(function) = self.functions.get(qualified_name) {
            return Some(ArtifactExport {
                source: qualified_name.to_string(),
                id: function.id,
            });
        }
        if let Some(strukt) = self.structs.get(qualified_name) {
            return Some(ArtifactExport {
                source: qualified_name.to_string(),
                id: strukt.id,
            });
        }
        if let Some(enum_) = self.enums.get(qualified_name) {
            return Some(ArtifactExport {
                source: qualified_name.to_string(),
                id: enum_.id,
            });
        }
        if let Some(alias) = self.type_aliases.get(qualified_name) {
            return Some(ArtifactExport {
                source: qualified_name.to_string(),
                id: alias.id,
            });
        }
        if let Some(trait_) = self.traits.get(qualified_name) {
            return Some(ArtifactExport {
                source: qualified_name.to_string(),
                id: trait_.id,
            });
        }
        if let Some(sig) = self.function_sigs.get(qualified_name) {
            return Some(ArtifactExport {
                source: qualified_name.to_string(),
                id: sig.id,
            });
        }
        if let Some(ext) = self.externs.iter().find(|ext| ext.name == qualified_name) {
            return Some(ArtifactExport {
                source: qualified_name.to_string(),
                id: ext.id,
            });
        }
        // Declared but not lowered yet: its identity is already known.
        self.names
            .declared_item(qualified_name)
            .map(|id| ArtifactExport {
                source: qualified_name.to_string(),
                id,
            })
    }

    pub(super) fn export_for_import_source(&self, qualified_name: &str) -> Option<ArtifactExport> {
        let mut parts = qualified_name.split("::");
        if let (Some(crate_name), Some(export_name), None) =
            (parts.next(), parts.next(), parts.next())
        {
            if let Some(export) = self
                .dependency_root_export_ids
                .get(crate_name)
                .and_then(|exports| exports.get(export_name))
            {
                return Some(export.clone());
            }
        }

        self.export_for_registered_item(qualified_name)
    }

    pub(super) fn artifact_root_export_for_import(
        &self,
        qualified_name: &str,
    ) -> Option<ArtifactExport> {
        let mut parts = qualified_name.split("::");
        let (Some(crate_name), Some(export_name), None) =
            (parts.next(), parts.next(), parts.next())
        else {
            return None;
        };

        self.dependency_root_export_ids
            .get(crate_name)
            .and_then(|exports| exports.get(export_name))
            .cloned()
    }

    pub(super) fn import_artifact_root_export(
        &mut self,
        short_name: String,
        qualified_name: String,
        export: ArtifactExport,
        is_explicit: bool,
        record_canonical_aliases: bool,
        canonical_prefix: Option<&str>,
    ) {
        let canonical_source = export.source.clone();

        if let Some(target) = self.lookup_import_target(&canonical_source) {
            self.bind_import(
                &short_name,
                target,
                &canonical_source,
                is_explicit && record_canonical_aliases,
                Some(export),
                canonical_prefix,
            );
            return;
        }

        self.push_error(format!(
            "Failed to import '{}': canonical declaration '{}' not found",
            qualified_name, export.source
        ));
    }
}

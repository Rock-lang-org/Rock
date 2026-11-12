use super::path_qualification::{
    local_path_segment_is_absolute, qualify_export_source, qualify_local_import_target,
    qualify_local_module_path, qualify_local_path, qualify_local_path_with_prefix,
};
use super::LocalCollector;
use crate::ast;
use crate::collect::context::{collect_exports, path_names};
use crate::ids::DefId;
use crate::lexer::Span;
use std::collections::HashMap;

impl LocalCollector {
    /// Registers the re-export aliases of every source-backed module before any
    /// import is handled, so an import may name an item through a re-export of
    /// a module walked later. Glob re-exports are expanded by the main walk.
    pub(super) fn predeclare_export_aliases(
        &mut self,
        module: &ast::Module,
        prefix: Option<&str>,
        source_backed: bool,
    ) {
        if let (Some(prefix), true) = (prefix, source_backed) {
            for (export_name, source) in collect_exports(module) {
                // `< Off` may re-export an import whose module has not been
                // walked yet. Resolve its written source during predeclaration.
                let source = source.or_else(|| {
                    module.top_levels.iter().find_map(|top_level| {
                        let ast::TopLevel::Import(path) = top_level else {
                            return None;
                        };
                        let names = path_names(path.segments());
                        (names.last() == Some(&export_name)).then(|| names.join("::"))
                    })
                });
                let Some(source) = source else {
                    continue;
                };
                if export_name.ends_with("::*") || source.ends_with("::*") {
                    continue;
                }
                let qualified_source = qualify_export_source(&self.context, prefix, &source);
                self.context
                    .names
                    .default_export_alias(format!("{}::{}", prefix, export_name), qualified_source);
            }
        }

        for top_level in &module.top_levels {
            match top_level {
                ast::TopLevel::Module(module_decl) => {
                    let inner = &module_decl.0;
                    let Some(name) = &inner.name else {
                        self.predeclare_export_aliases(inner, prefix, source_backed);
                        continue;
                    };
                    let child_prefix = prefix
                        .map(|prefix| format!("{}::{}", prefix, name.name))
                        .unwrap_or_else(|| name.name.clone());
                    self.predeclare_export_aliases(inner, Some(&child_prefix), source_backed);
                }
                ast::TopLevel::Mod(ident, _) => {
                    let loading_prefix = prefix
                        .map(str::to_owned)
                        .or_else(|| self.context.current_module_prefix());
                    // Load failures are reported by the main walk.
                    let Ok(loaded) = self
                        .context
                        .load_module_with_prefix(&ident.name, loading_prefix.as_deref())
                    else {
                        continue;
                    };
                    let Some(path) = self
                        .context
                        .loaded_module_path_for_prefix(&ident.name, loading_prefix.as_deref())
                    else {
                        continue;
                    };
                    let child_prefix = loading_prefix
                        .as_deref()
                        .map(|prefix| format!("{}::{}", prefix, ident.name))
                        .unwrap_or_else(|| ident.name.clone());
                    let outer = self.context.modules.enter(path);
                    self.predeclare_export_aliases(&loaded, Some(&child_prefix), true);
                    self.context.modules.leave(outer);
                }
                _ => {}
            }
        }
    }

    pub(super) fn register_export_aliases(
        &mut self,
        prefix: &str,
        exports: &HashMap<String, Option<String>>,
    ) {
        for (export_name, source) in exports {
            let qualified_export = if prefix.is_empty() {
                export_name.clone()
            } else {
                format!("{}::{}", prefix, export_name)
            };
            let qualified_source = match source {
                Some(source) if prefix.is_empty() => source.clone(),
                Some(source) => qualify_export_source(&self.context, prefix, source),
                None => qualified_export.clone(),
            };

            self.context
                .names
                .set_export_alias(qualified_export.clone(), qualified_source.clone());

            if let Some(func) = self.context.functions.get(&qualified_source) {
                let func_type = func.fn_type();
                self.context
                    .scope
                    .define(qualified_export.clone(), func_type, false);
            } else if let Some(binding) = self.context.scope.lookup(&qualified_source) {
                self.context
                    .scope
                    .define(qualified_export.clone(), binding.ty.clone(), false);
            }
        }
    }

    pub(super) fn handle_import(&mut self, path: &ast::Path) {
        let canonical_prefix = self.context.current_module_prefix();
        let mut resolved =
            self.context
                .handle_import(path, true, canonical_prefix.as_deref());
        let names = path_names(path.segments());
        if let Some(short_name) = names.last().cloned() {
            resolved |= self.import_type_alias_local(
                canonical_prefix.as_deref().unwrap_or_default(),
                short_name,
                names.join("::"),
            );
        }

        if let Some(qualified_path) = qualify_local_path(&self.context, path) {
            resolved |= self.context.handle_import(
                &qualified_path,
                true,
                canonical_prefix.as_deref(),
            );
            let names = path_names(qualified_path.segments());
            if let Some(short_name) = names.last().cloned() {
                resolved |= self.import_type_alias_local(
                    canonical_prefix.as_deref().unwrap_or_default(),
                    short_name,
                    names.join("::"),
                );
            }
        }

        if !resolved {
            self.report_unresolved_import(path);
        }
    }

    pub(super) fn handle_import_in_context(&mut self, path: &ast::Path, prefix: &str) {
        let original_names = path_names(path.segments());
        let mut resolved = false;
        if let (Some(first_segment), Some(short_name)) =
            (original_names.first(), original_names.last().cloned())
        {
            if local_path_segment_is_absolute(&self.context, first_segment) {
                resolved |= self.context.handle_import(path, false, Some(prefix));
                let qualified_name = original_names.join("::");
                resolved |= self.record_module_local_artifact_root_alias(
                    prefix,
                    &short_name,
                    &qualified_name,
                );
                resolved |= self.import_type_alias_local(prefix, short_name, qualified_name);
            }
        }

        let resolved_path = qualify_local_path_with_prefix(&self.context, path, prefix)
            .unwrap_or_else(|| path.clone());
        resolved |= self
            .context
            .handle_import(&resolved_path, false, Some(prefix));
        let names = path_names(resolved_path.segments());

        if let Some(short_name) = names.last().cloned() {
            let qualified_name = names.join("::");
            resolved |=
                self.record_module_local_artifact_root_alias(prefix, &short_name, &qualified_name);
            resolved |= self.import_type_alias_local(prefix, short_name, qualified_name);
        }

        if !resolved {
            self.report_unresolved_import(path);
        }
    }

    /// Every item is registered before imports are handled, so an import that
    /// matches nothing now never will.
    fn report_unresolved_import(&mut self, path: &ast::Path) {
        let segments = path.segments();
        let message = format!("unresolved import '{}'", path_names(segments).join("::"));
        match path_span(segments) {
            Some(span) => self.context.push_error_with_span(message, span),
            None => self.context.push_error(message),
        }
    }

    pub(super) fn handle_glob_import(&mut self, module_path: &[String]) {
        self.context.handle_glob_import(module_path);

        if let Some(qualified_module_path) = qualify_local_module_path(&self.context, module_path) {
            self.context.handle_glob_import(&qualified_module_path);
        }
    }

    pub(super) fn handle_glob_import_in_context(&mut self, module_path: &[String], prefix: &str) {
        let module_label = format!("{}::*", module_path.join("::"));
        match self.context.glob_import_targets(module_path) {
            Ok(targets) => {
                for (short_name, qualified_name) in targets {
                    let resolved_name =
                        qualify_local_import_target(&self.context, prefix, &qualified_name);
                    self.context.import_qualified_name(
                        short_name,
                        resolved_name,
                        false,
                        false,
                        Some(prefix),
                    );
                }
            }
            Err(err) => self
                .context
                .push_error(format!("Failed to import {}: {}", module_label, err)),
        }
    }

    pub(super) fn import_type_alias_local(
        &mut self,
        prefix: &str,
        short_name: String,
        qualified_name: String,
    ) -> bool {
        if let Some(strukt) = self.context.structs.get(&qualified_name).cloned() {
            let item_id = Some(strukt.id);
            self.context.structs.insert(short_name.clone(), strukt);
            let resolver_alias = format!("{}::{}", prefix, short_name);
            self.record_import_type_alias(
                short_name,
                qualified_name,
                item_id,
                Some(resolver_alias),
            );
            return true;
        }

        if let Some(enum_) = self.context.enums.get(&qualified_name).cloned() {
            let item_id = Some(enum_.id);
            self.context.enums.insert(short_name.clone(), enum_);
            let resolver_alias = format!("{}::{}", prefix, short_name);
            self.record_import_type_alias(
                short_name,
                qualified_name,
                item_id,
                Some(resolver_alias),
            );
            return true;
        }

        if let Some(alias) = self.context.type_aliases.get(&qualified_name).cloned() {
            let item_id = Some(alias.id);
            self.context.type_aliases.insert(short_name.clone(), alias);
            let resolver_alias =
                (!prefix.is_empty()).then(|| format!("{}::{}", prefix, short_name));
            self.record_import_type_alias(short_name, qualified_name, item_id, resolver_alias);
            return true;
        }

        if let Some(trait_) = self.context.traits.get(&qualified_name).cloned() {
            let item_id = Some(trait_.id);
            self.context.traits.insert(short_name.clone(), trait_);
            let resolver_alias = format!("{}::{}", prefix, short_name);
            self.record_import_type_alias(
                short_name,
                qualified_name,
                item_id,
                Some(resolver_alias),
            );
            return true;
        }

        false
    }

    pub(super) fn record_module_local_artifact_root_alias(
        &mut self,
        prefix: &str,
        short_name: &str,
        qualified_name: &str,
    ) -> bool {
        let mut parts = qualified_name.split("::");
        let (Some(crate_name), Some(export_name), None) =
            (parts.next(), parts.next(), parts.next())
        else {
            return false;
        };

        let Some(export) = self
            .context
            .dependency_root_export_ids
            .get(crate_name)
            .and_then(|exports| exports.get(export_name))
            .cloned()
        else {
            return false;
        };

        let resolver_alias = format!("{}::{}", prefix, short_name);
        let names = &mut self.context.names;
        names.record_canonical_alias(resolver_alias, export.id, true);
        names.set_canonical_name(export.id, export.source);
        true
    }

    pub(super) fn record_import_type_alias(
        &mut self,
        short_name: String,
        qualified_name: String,
        item_id: Option<DefId>,
        local_resolver_alias: Option<String>,
    ) {
        let resolver_alias = local_resolver_alias.unwrap_or_else(|| short_name.clone());
        if let Some(id) = item_id {
            let names = &mut self.context.names;
            names.record_canonical_alias(resolver_alias, id, true);
            names.set_canonical_name(id, qualified_name);
        }
    }
}

/// Span covering every segment of an import path.
fn path_span(segments: &[ast::IdentOrType]) -> Option<Span> {
    let segment_span = |segment: &ast::IdentOrType| match segment {
        ast::IdentOrType::Ident(ident) => ident.span.clone(),
        ast::IdentOrType::Type(ty) => ty.span(),
    };
    let first = segment_span(segments.first()?);
    let last = segment_span(segments.last()?);
    Some(Span::new(first.file_path, first.start, last.end))
}

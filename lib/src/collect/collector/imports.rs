use super::path_qualification::{
    local_path_segment_is_absolute, qualify_export_source, qualify_local_import_target,
    qualify_local_module_path, qualify_local_path, qualify_local_path_with_prefix,
};
use super::LocalCollector;
use crate::ast;
use crate::collect::context::path_names;
use crate::collect::context::CollectContext;
use crate::ids::DefId;
use crate::lexer::Span;
use crate::types::Type;
use std::collections::HashMap;

impl LocalCollector {
    pub(super) fn register_export_aliases(
        &mut self,
        prefix: &str,
        exports: &HashMap<String, Option<String>>,
    ) {
        for (export_name, source) in exports {
            let Some(source) = source else {
                continue;
            };

            let qualified_export = format!("{}::{}", prefix, export_name);
            let qualified_source = qualify_export_source(&self.context, prefix, source);

            self.context
                .export_aliases
                .insert(qualified_export.clone(), qualified_source.clone());

            if let Some(func) = self.context.functions.get(&qualified_source) {
                let param_types: Vec<Type> = func.params.iter().map(|p| p.ty.clone()).collect();
                let func_type = Type::function_with_safety(
                    param_types,
                    func.ret_type.clone(),
                    crate::types::FunctionSafety::from_is_unsafe(func.is_unsafe),
                );
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
        let mut pending = PendingImport::new(path);
        let mut resolved =
            pending.try_import(&mut self.context, path, true, canonical_prefix.as_deref());
        let names = match path {
            ast::Path::Ident(path) => path_names(&path.path),
            ast::Path::Type(path) => path_names(&path.path),
        };
        if let Some(short_name) = names.last().cloned() {
            resolved |= self.import_type_alias_local(
                canonical_prefix.as_deref().unwrap_or_default(),
                short_name,
                names.join("::"),
            );
        }

        if let Some(qualified_path) = qualify_local_path(&self.context, path) {
            resolved |= pending.try_import(
                &mut self.context,
                &qualified_path,
                true,
                canonical_prefix.as_deref(),
            );
            let names = match &qualified_path {
                ast::Path::Ident(path) => path_names(&path.path),
                ast::Path::Type(path) => path_names(&path.path),
            };
            if let Some(short_name) = names.last().cloned() {
                resolved |= self.import_type_alias_local(
                    canonical_prefix.as_deref().unwrap_or_default(),
                    short_name,
                    names.join("::"),
                );
            }
        }

        if !resolved {
            self.pending_imports.push(pending);
        }
    }

    pub(super) fn handle_import_in_context(&mut self, path: &ast::Path, prefix: &str) {
        let original_names = match path {
            ast::Path::Ident(ip) => path_names(&ip.path),
            ast::Path::Type(tp) => path_names(&tp.path),
        };
        let mut pending = PendingImport::new(path);
        let mut resolved = false;
        if let (Some(first_segment), Some(short_name)) =
            (original_names.first(), original_names.last().cloned())
        {
            if local_path_segment_is_absolute(&self.context, first_segment) {
                resolved |= pending.try_import(&mut self.context, path, false, Some(prefix));
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
        resolved |= pending.try_import(&mut self.context, &resolved_path, false, Some(prefix));
        let names = match &resolved_path {
            ast::Path::Ident(ip) => path_names(&ip.path),
            ast::Path::Type(tp) => path_names(&tp.path),
        };

        if let Some(short_name) = names.last().cloned() {
            let qualified_name = names.join("::");
            resolved |=
                self.record_module_local_artifact_root_alias(prefix, &short_name, &qualified_name);
            resolved |= self.import_type_alias_local(prefix, short_name, qualified_name);
        }

        if !resolved {
            self.pending_imports.push(pending);
        }
    }

    /// Reports imports that matched nothing when they were collected and still match
    /// nothing now that every declaration is registered.
    pub(super) fn report_unresolved_imports(&mut self) {
        for pending in std::mem::take(&mut self.pending_imports) {
            if pending.resolves_now(&self.context) {
                continue;
            }
            let message = format!("unresolved import '{}'", pending.label);
            match pending.span {
                Some(span) => self.context.push_error_with_span(message, span),
                None => self.context.push_error(message),
            }
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
        self.context
            .canonical_import_aliases
            .insert(resolver_alias, export.id);
        self.context
            .canonical_names_by_id
            .insert(export.id, export.source);
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
            self.context
                .canonical_import_aliases
                .insert(resolver_alias, id);
            self.context
                .canonical_names_by_id
                .insert(id, qualified_name);
        }
    }
}

/// An import that matched nothing when it was collected, kept so it can be
/// checked again once every declaration is registered.
pub(super) struct PendingImport {
    label: String,
    span: Option<Span>,
    /// Each `(qualified_name, record_canonical_aliases, canonical_prefix)` tried.
    attempts: Vec<(String, bool, Option<String>)>,
}

impl PendingImport {
    fn new(path: &ast::Path) -> Self {
        let segments = match path {
            ast::Path::Ident(path) => &path.path,
            ast::Path::Type(path) => &path.path,
        };
        Self {
            label: path_names(segments).join("::"),
            span: path_span(segments),
            attempts: Vec::new(),
        }
    }

    fn try_import(
        &mut self,
        context: &mut CollectContext,
        path: &ast::Path,
        record_canonical_aliases: bool,
        canonical_prefix: Option<&str>,
    ) -> bool {
        let names = match path {
            ast::Path::Ident(path) => path_names(&path.path),
            ast::Path::Type(path) => path_names(&path.path),
        };
        self.attempts.push((
            names.join("::"),
            record_canonical_aliases,
            canonical_prefix.map(str::to_string),
        ));
        context.handle_import(path, record_canonical_aliases, canonical_prefix)
    }

    fn resolves_now(&self, context: &CollectContext) -> bool {
        self.attempts.iter().any(
            |(qualified_name, record_canonical_aliases, canonical_prefix)| {
                context
                    .resolve_import(
                        qualified_name,
                        *record_canonical_aliases,
                        canonical_prefix.as_deref(),
                    )
                    .is_some()
                    || context
                        .externs
                        .iter()
                        .any(|ext| &ext.name == qualified_name)
            },
        )
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

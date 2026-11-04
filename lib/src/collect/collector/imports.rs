use super::path_qualification::{
    local_path_segment_is_absolute, qualify_export_source, qualify_local_import_target,
    qualify_local_module_path, qualify_local_path, qualify_local_path_with_prefix,
};
use super::LocalCollector;
use crate::ast;
use crate::collect::context::path_names;
use crate::ids::DefId;
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
        self.context
            .handle_import(path, true, canonical_prefix.as_deref());
        let names = match path {
            ast::Path::Ident(path) => path_names(&path.path),
            ast::Path::Type(path) => path_names(&path.path),
        };
        if let Some(short_name) = names.last().cloned() {
            self.import_type_alias_local(
                canonical_prefix.as_deref().unwrap_or_default(),
                short_name,
                names.join("::"),
            );
        }

        if let Some(qualified_path) = qualify_local_path(&self.context, path) {
            self.context
                .handle_import(&qualified_path, true, canonical_prefix.as_deref());
            let names = match &qualified_path {
                ast::Path::Ident(path) => path_names(&path.path),
                ast::Path::Type(path) => path_names(&path.path),
            };
            if let Some(short_name) = names.last().cloned() {
                self.import_type_alias_local(
                    canonical_prefix.as_deref().unwrap_or_default(),
                    short_name,
                    names.join("::"),
                );
            }
        }
    }

    pub(super) fn handle_import_in_context(&mut self, path: &ast::Path, prefix: &str) {
        let original_names = match path {
            ast::Path::Ident(ip) => path_names(&ip.path),
            ast::Path::Type(tp) => path_names(&tp.path),
        };
        if let (Some(first_segment), Some(short_name)) =
            (original_names.first(), original_names.last().cloned())
        {
            if local_path_segment_is_absolute(&self.context, first_segment) {
                self.context.handle_import(path, false, Some(prefix));
                let qualified_name = original_names.join("::");
                self.record_module_local_artifact_root_alias(prefix, &short_name, &qualified_name);
                self.import_type_alias_local(prefix, short_name, qualified_name);
            }
        }

        let resolved_path = qualify_local_path_with_prefix(&self.context, path, prefix)
            .unwrap_or_else(|| path.clone());
        self.context
            .handle_import(&resolved_path, false, Some(prefix));
        let names = match &resolved_path {
            ast::Path::Ident(ip) => path_names(&ip.path),
            ast::Path::Type(tp) => path_names(&tp.path),
        };

        if let Some(short_name) = names.last().cloned() {
            let qualified_name = names.join("::");
            self.record_module_local_artifact_root_alias(prefix, &short_name, &qualified_name);
            self.import_type_alias_local(prefix, short_name, qualified_name);
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
    ) {
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
            return;
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
            return;
        }

        if let Some(alias) = self.context.type_aliases.get(&qualified_name).cloned() {
            let item_id = Some(alias.id);
            self.context.type_aliases.insert(short_name.clone(), alias);
            let resolver_alias =
                (!prefix.is_empty()).then(|| format!("{}::{}", prefix, short_name));
            self.record_import_type_alias(short_name, qualified_name, item_id, resolver_alias);
            return;
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
        }
    }

    pub(super) fn record_module_local_artifact_root_alias(
        &mut self,
        prefix: &str,
        short_name: &str,
        qualified_name: &str,
    ) {
        let mut parts = qualified_name.split("::");
        let (Some(crate_name), Some(export_name), None) =
            (parts.next(), parts.next(), parts.next())
        else {
            return;
        };

        let Some(export) = self
            .context
            .dependency_root_export_ids
            .get(crate_name)
            .and_then(|exports| exports.get(export_name))
            .cloned()
        else {
            return;
        };

        let resolver_alias = format!("{}::{}", prefix, short_name);
        self.context
            .canonical_import_aliases
            .insert(resolver_alias, export.id);
        self.context
            .canonical_names_by_id
            .insert(export.id, export.source);
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

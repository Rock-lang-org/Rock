use super::CollectContext;
use crate::crate_artifact::ArtifactExport;
use crate::crate_system::{CrateContext, ExternCrateRef};

impl CollectContext {
    pub(crate) fn inject_loaded_prelude(&mut self, _ctx: &CrateContext) {
        if !self.loaded_prelude_export_ids.is_empty() {
            let exports: Vec<(String, ArtifactExport)> = self
                .loaded_prelude_export_ids
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();

            for (short_name, export) in exports {
                let qualified_name = export.source.clone();

                if self.inject_prelude_alias(short_name.clone(), &qualified_name) {
                    self.record_canonical_alias(short_name.clone(), &export);
                } else {
                    self.push_error(format!(
                        "Failed to inject std prelude alias '{}': declaration '{}' not found",
                        short_name, qualified_name
                    ));
                }
            }

            return;
        }
    }

    fn inject_prelude_alias(&mut self, short_name: String, qualified_name: &str) -> bool {
        if let Some(func) = self.functions.get(qualified_name) {
            let func_type = func.fn_type();
            self.scope.define(short_name.clone(), func_type, false);
            self.import_aliases
                .entry(short_name.clone())
                .or_insert_with(|| qualified_name.to_string());
            true
        } else if let Some(binding) = self.scope.lookup(qualified_name) {
            self.scope
                .define(short_name.clone(), binding.ty.clone(), false);
            self.import_aliases
                .entry(short_name)
                .or_insert_with(|| qualified_name.to_string());
            true
        } else if self.structs.contains_key(qualified_name) {
            self.import_aliases
                .entry(short_name)
                .or_insert_with(|| qualified_name.to_string());
            true
        } else if self.enums.contains_key(qualified_name) {
            self.import_aliases
                .entry(short_name)
                .or_insert_with(|| qualified_name.to_string());
            true
        } else if self.traits.contains_key(qualified_name) {
            self.import_aliases
                .entry(short_name)
                .or_insert_with(|| qualified_name.to_string());
            true
        } else {
            false
        }
    }

    pub(crate) fn register_crate_functions(&mut self, ctx: &CrateContext) {
        for dep in ctx.extern_crates() {
            if ctx.is_current_source_self_artifact(self.current_crate_name.as_deref(), dep.name()) {
                continue;
            }
            self.register_extern_crate(dep);
        }
    }

    pub(crate) fn register_extern_crate(&mut self, dep: ExternCrateRef<'_>) {
        let crate_name = dep.name();
        let metadata = dep.metadata();
        let interface = metadata.interface();

        for (id, name) in &interface.canonical_names {
            self.record_canonical_item_name(*id, name);
        }

        // Nested public facades resolve to the same dependency declarations as
        // their canonical paths, including while collecting function signatures.
        for (alias, id) in &metadata.resolver().export_aliases {
            if let Some(source) = metadata.resolver().canonical_name(*id) {
                self.export_aliases
                    .insert(alias.clone(), source.to_string());
            }
        }

        if !dep.root_exports().is_empty() {
            self.dependency_root_export_ids.insert(
                crate_name.to_string(),
                dep.root_exports()
                    .iter()
                    .map(|(name, export)| (name.clone(), export.clone()))
                    .collect(),
            );
        }

        if crate_name == "std" && !dep.prelude_exports().is_empty() {
            self.record_loaded_prelude_exports(
                dep.prelude_exports()
                    .iter()
                    .map(|(name, export)| (name.clone(), export.clone())),
            );
        }

        for (name, func) in interface.function_items() {
            let func_type = func.fn_type();
            self.scope.define(name.clone(), func_type, false);
            self.record_canonical_item_name(func.id, &name);
            self.functions.insert(name.clone(), func.clone());
        }

        for ext in interface.extern_items() {
            let func_type = ext.fn_type();
            self.scope.define(ext.name.clone(), func_type, false);
            self.record_canonical_item_name(ext.id, &ext.name);
            self.externs.push(ext.clone());
        }

        for (name, strukt) in interface.struct_items() {
            self.record_canonical_item_name(strukt.id, &name);
            self.structs.insert(name.clone(), strukt.clone());
        }

        for (name, enum_) in interface.enum_items() {
            self.record_canonical_item_name(enum_.id, &name);
            self.enums.insert(name.clone(), enum_.clone());
        }

        for (name, alias) in interface.type_alias_items() {
            self.record_canonical_item_name(alias.id, &name);
            self.type_aliases.insert(name, alias);
        }

        for (name, trait_) in interface.trait_items() {
            self.record_canonical_item_name(trait_.id, &name);
            self.traits.insert(name.clone(), trait_.clone());
        }

        for imp in interface.impl_items() {
            self.impls.push(imp.clone());
        }

        for (name, precedence) in &interface.infix_precedence {
            self.infix_precedence.insert(name.clone(), *precedence);
        }
    }

    pub(crate) fn current_module_prefix(&self) -> Option<String> {
        if let Some(prefix) = &self.current_qualified_module_prefix {
            return Some(prefix.clone());
        }

        self.loaded_module_paths
            .iter()
            .find(|(_, path)| path == &self.current_module_path)
            .map(|(name, _)| name.clone())
            .or_else(|| self.current_crate_name.clone())
    }

    pub(super) fn qualify_import_source(
        &self,
        source: &str,
        canonical_prefix: Option<&str>,
    ) -> String {
        if let Some(crate_name) = &self.current_crate_name {
            if let Some(stripped) = source.strip_prefix(&format!("{}::", crate_name)) {
                return stripped.to_string();
            }
        }

        let first_segment = source.split("::").next().unwrap_or(source);
        let is_absolute = self
            .loaded_module_paths
            .iter()
            .any(|(name, _)| name == first_segment)
            || canonical_prefix.is_some_and(|name| name == first_segment);

        if is_absolute {
            source.to_string()
        } else if let Some(prefix) = canonical_prefix {
            format!("{}::{}", prefix, source)
        } else {
            source.to_string()
        }
    }
}

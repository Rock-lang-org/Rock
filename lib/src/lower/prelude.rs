use std::collections::{BTreeMap, HashMap};

use crate::collect::resolver::ResolverTables;
use crate::crate_artifact::ArtifactExport;
use crate::lower::items::LowerItems;
use crate::lower::scope::Scope;
use crate::types::Type;

#[derive(Debug, Clone)]
pub(crate) struct PreludeImports {
    enabled: bool,
    loaded_exports: HashMap<String, ArtifactExport>,
}

impl PreludeImports {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            loaded_exports: HashMap::new(),
        }
    }

    pub(crate) fn with_exports(
        enabled: bool,
        loaded_exports: HashMap<String, ArtifactExport>,
    ) -> Self {
        Self {
            enabled,
            loaded_exports,
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn capture_loaded_prelude_exports(
        &mut self,
        crate_name: &str,
        exports: &BTreeMap<String, ArtifactExport>,
    ) {
        if crate_name != "std" || exports.is_empty() {
            return;
        }

        self.loaded_exports.extend(
            exports
                .iter()
                .map(|(name, export)| (name.clone(), export.clone())),
        );
    }

    pub(crate) fn inject_loaded_prelude(
        &self,
        scope: &mut Scope,
        items: &mut LowerItems,
        resolver: &mut ResolverTables,
    ) -> Vec<String> {
        let exports = self.export_entries();
        let mut errors = Vec::new();

        for (short_name, export) in exports {
            if Self::short_name_is_user_owned(resolver, &short_name) {
                continue;
            }

            resolver.insert_import_alias_with_name(
                short_name.clone(),
                export.source.clone(),
                export.id,
            );

            if let Some(func) = items.function(export.id) {
                let func_type = func.fn_type();
                scope.define_alias(short_name.clone(), func_type, false);
            } else if let Some(extern_def) = items.extern_def(export.id) {
                let func_type = Type::function_with_safety(
                    extern_def.params.clone(),
                    extern_def.ret.clone(),
                    crate::types::FunctionSafety::from_is_unsafe(extern_def.is_unsafe),
                );
                scope.define_alias(short_name.clone(), func_type, false);
            } else if let Some(ty) = scope
                .lookup(&export.source)
                .map(|binding| binding.ty.clone())
            {
                scope.define_alias(short_name.clone(), ty, false);
            } else if items.structure(export.id).is_some()
                || items.enumeration(export.id).is_some()
                || items.trait_def(export.id).is_some()
            {
                continue;
            } else {
                errors.push(format!(
                    "Missing loaded prelude declaration for {}",
                    export.source
                ));
            }
        }

        errors
    }

    fn short_name_is_user_owned(resolver: &ResolverTables, short_name: &str) -> bool {
        resolver.item_paths.contains_key(short_name)
            || resolver.import_aliases.contains_key(short_name)
    }

    fn export_entries(&self) -> Vec<(String, ArtifactExport)> {
        let mut exports: Vec<_> = self
            .loaded_exports
            .iter()
            .map(|(name, export)| (name.clone(), export.clone()))
            .collect();
        exports.sort_by(|(left, _), (right, _)| left.cmp(right));
        exports
    }
}

#[cfg(test)]
mod tests;

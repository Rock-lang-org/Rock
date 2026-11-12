use std::collections::{HashMap, HashSet};

use crate::collect::resolver::VariantAlias;
use crate::lexer::Span;

use super::CollectContext;

impl CollectContext {
    /// Follow ordinary re-exports before splitting off a variant's owner.
    /// Bodies are completed later, so this resolves identity, not validity.
    pub(super) fn resolve_variant_source(&self, source: &str) -> Option<VariantAlias> {
        self.resolve_variant_source_inner(source, &mut HashSet::new())
    }

    fn resolve_variant_source_inner(
        &self,
        source: &str,
        visited: &mut HashSet<String>,
    ) -> Option<VariantAlias> {
        if !visited.insert(source.to_string()) {
            return None;
        }
        if let Some(crate_name) = &self.current_crate_name {
            if !source.starts_with(&format!("{crate_name}::")) {
                let qualified = format!("{crate_name}::{source}");
                if let Some(alias) = self.resolve_variant_source_inner(&qualified, visited) {
                    return Some(alias);
                }
            }
        }
        if let Some(alias) = self.variant_aliases.get(source) {
            if !alias.fallback {
                let mut alias = alias.clone();
                alias.fallback = false;
                return Some(alias);
            }
        }
        if self.export_for_registered_item(source).is_some() {
            return None;
        }
        if let Some(target) = self.names.export_alias(source) {
            if let Some(alias) = self.resolve_variant_source_inner(target, visited) {
                return Some(alias);
            }
        }
        if let Some(alias) = self.variant_aliases.get(source) {
            let mut alias = alias.clone();
            alias.fallback = false;
            return Some(alias);
        }
        if let Some(export) = self.artifact_root_export_for_import(source) {
            return self.resolve_variant_source_inner(&export.source, visited);
        }
        let (owner, variant) = source.rsplit_once("::")?;
        let enum_id = self.enums.get(owner).map(|enumeration| enumeration.id)
            .or_else(|| {
                let target = self.names.export_alias(owner)
                    .or_else(|| self.names.import_alias(owner))?;
                self.enums.get(target).map(|enumeration| enumeration.id)
            })?;
        Some(VariantAlias {
            enum_id,
            variant: variant.to_string(),
            fallback: false,
        })
    }

    pub(super) fn variant_exports(&mut self) -> HashMap<String, VariantAlias> {
        let exports = self.names.export_sources()
            .filter_map(|(name, source)| {
                let alias = self.resolve_variant_source(source)?;
                Some((name.clone(), alias))
            })
            .collect::<HashMap<_, _>>();
        for (name, alias) in &exports {
            let key = if !name.contains("::") {
                self.current_crate_name.as_ref().map(|prefix| format!("{prefix}::{name}"))
                    .unwrap_or_else(|| name.clone())
            } else {
                name.clone()
            };
            self.record_explicit_import(&key, Some(alias.clone()), None);
            let valid = self.enums.values().find(|enumeration| enumeration.id == alias.enum_id)
                .is_some_and(|enumeration| {
                    enumeration.variants.iter().any(|variant| variant.name == alias.variant)
                });
            if !valid {
                self.push_error(format!("unresolved variant export '{name}'"));
            }
        }
        exports
    }

    /// A prelude entry is a fallback, not an explicit declaration. Only
    /// collisions involving a variant are handled here; item imports keep
    /// their existing resolution rules.
    pub(super) fn record_explicit_import(
        &mut self,
        key: &str,
        target: Option<VariantAlias>,
        span: Option<Span>,
    ) {
        let conflicts_with_item = target.is_some() && self.names.declared_item(key).is_some();
        let conflicts_with_import = self.explicit_imports.get(key).is_some_and(|previous| {
            (target.is_some() || previous.is_some()) && previous != &target
        });
        if conflicts_with_item || conflicts_with_import {
            let message = format!("conflicting explicit declarations or imports for '{key}'");
            match span {
                Some(span) => self.push_error_with_span(message, span),
                None => self.push_error(message),
            }
        }
        self.explicit_imports.insert(key.to_string(), target);
    }
}

//! Compilation-local import/export surfaces retain every segment's binding context.

use std::collections::HashSet;

use crate::ast::{self, TopLevel};
use crate::crate_system::CrateContext;
use crate::ids::{DefId, ModuleId};
use crate::lexer::{lossless::SyntaxContext, Span};

use super::item_index::{ItemIndex, SourceModuleMap};
use super::resolver::{binding_path, ResolverTables};
use crate::lower::error::ResolveError;

struct Directive {
    scope: ModuleId,
    path: Vec<(String, SyntaxContext)>,
    glob: bool,
    export: bool,
    span: Span,
}

impl ResolverTables {
    pub(crate) fn seed_binding_surfaces(
        &mut self,
        module: &ast::Module,
        root: ModuleId,
        index: &ItemIndex,
        sources: &SourceModuleMap,
        crates: &CrateContext,
    ) -> Vec<ResolveError> {
        let mut directives = Vec::new();
        self.collect_binding_surfaces(module, root, index, sources, &mut Vec::new(), &mut directives);
        let mut glob_aliases = HashSet::new();
        // Imports and re-exports may precede their providers; only identity-bearing
        // surfaces participate, never a spelling-only reconstruction of a path.
        for _ in 0..=directives.len() {
            let mut changed = false;
            for directive in directives.iter().filter(|directive| !directive.glob).chain(directives.iter().filter(|directive| directive.glob)) {
                let prefix = self.module_binding_paths.get(&directive.scope).cloned().unwrap_or_default();
                let targets = self.directive_targets(directive, crates);
                for (name, id) in targets {
                    let mut alias = prefix.clone();
                    alias.push(name);
                    if directive.glob && !directive.export && self.binding_paths.contains_key(&alias) { continue; }
                    let surface = if directive.export { &mut self.binding_exports } else { &mut self.binding_aliases };
                    let overrides_glob = !directive.glob && glob_aliases.remove(&(directive.export, alias.clone()));
                    if !surface.contains_key(&alias) || overrides_glob {
                        surface.insert(alias.clone(), id);
                        if directive.glob { glob_aliases.insert((directive.export, alias.clone())); }
                        changed = true;
                    }
                    if directive.export && !self.binding_aliases.contains_key(&alias) {
                        self.binding_aliases.insert(alias, id);
                        changed = true;
                    }
                }
            }
            if !changed { break; }
        }
        let mut errors = Vec::new();
        for directive in &directives {
            if !(self.contextual_scope(Some(directive.scope)) || directive.path.iter().any(|(_, context)| *context != SyntaxContext::ROOT)) { continue; }
            let targets = self.directive_targets(directive, crates);
            if targets.is_empty() && (!directive.glob || !self.indexed_module_path(Some(directive.scope), &directive.path)) {
                errors.push(ResolveError::with_span("unresolved contextual import or export".into(), directive.span.clone()));
            }
            if directive.glob { continue; }
            for (name, id) in targets {
                let mut alias = self.module_binding_paths.get(&directive.scope).cloned().unwrap_or_default();
                alias.push(name);
                let surface = if directive.export { &self.binding_exports } else { &self.binding_aliases };
                if surface.get(&alias).is_some_and(|previous| *previous != id)
                    || self.binding_paths.get(&alias).is_some_and(|previous| *previous != id) {
                    errors.push(ResolveError::with_span("conflicting contextual import or export".into(), directive.span.clone()));
                }
            }
        }
        errors
    }

    fn directive_targets(&self, directive: &Directive, crates: &CrateContext) -> Vec<((String, SyntaxContext), DefId)> {
        let root_path = directive.path.iter().all(|(_, context)| *context == SyntaxContext::ROOT);
        if root_path {
            if let Some((first, rest)) = directive.path.split_first() {
                if let Some(provider) = crates.extern_crates().find(|provider| provider.name() == first.0) {
                    if directive.glob && rest.is_empty() {
                        return provider.root_exports().iter().map(|(name, export)| ((name.clone(), SyntaxContext::ROOT), export.id)).collect();
                    }
                    if !directive.glob {
                        let name = rest.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>().join("::");
                        if let Some(export) = provider.root_exports().get(&name) {
                            return directive.path.last().cloned().map(|name| vec![(name, export.id)]).unwrap_or_default();
                        }
                    }
                }
            }
        }
        let Some(id) = self.resolve_binding_path(Some(directive.scope), &directive.path) else { return Vec::new(); };
        if !directive.glob {
            return directive.path.last().cloned().map(|name| vec![(name, id)]).unwrap_or_default();
        }
        let Some(prefix) = self.definition_binding_paths.get(&id) else { return Vec::new(); };
        self.binding_exports.iter().filter_map(|(path, id)| {
            (path.len() == prefix.len() + 1 && path.starts_with(prefix)).then(|| (path.last().cloned().unwrap(), *id))
        }).collect()
    }

    fn collect_binding_surfaces(
        &mut self,
        module: &ast::Module,
        scope: ModuleId,
        index: &ItemIndex,
        sources: &SourceModuleMap,
        source_path: &mut Vec<String>,
        directives: &mut Vec<Directive>,
    ) {
        for (position, item) in module.top_levels.iter().enumerate() {
            let path = match item {
                TopLevel::Import(path) | TopLevel::Export(path) => Some(match path { ast::Path::Ident(path) => &path.path, ast::Path::Type(path) => &path.path }),
                TopLevel::GlobImport(path) | TopLevel::GlobExport(path) => Some(path),
                _ => None,
            };
            if let Some(path) = path {
                if let (Some(names), Some(first)) = (binding_path(path), path.first()) {
                    directives.push(Directive {
                        scope, path: names,
                        glob: matches!(item, TopLevel::GlobImport(_) | TopLevel::GlobExport(_)),
                        export: matches!(item, TopLevel::Export(_) | TopLevel::GlobExport(_)),
                        span: match first { ast::IdentOrType::Ident(name) => name.span.clone(), ast::IdentOrType::Type(ty) => ty.span() },
                    });
                }
            }
            let exported = match item {
                TopLevel::FunctionDecl(item) => item.exported,
                TopLevel::Extern(item) => item.exported,
                TopLevel::StructDecl(item) => item.exported,
                TopLevel::EnumDecl(item) => item.exported,
                TopLevel::TraitDecl(item) => item.exported,
                TopLevel::Mod(_, exported) => *exported,
                _ => false,
            };
            if exported {
                if let Some(record) = index.item_at_source(scope, position) {
                    self.binding_exports.insert(index.item_binding_path(record), record.def_id);
                }
            }
            let (name, inline) = match item {
                TopLevel::Module(child) => (child.0.name.as_ref(), Some(&child.0)),
                TopLevel::Mod(name, _) => (Some(name), None),
                _ => continue,
            };
            let Some(name) = name else { continue; };
            let Some(child) = index.child_module_for_name(scope, name) else { continue; };
            source_path.push(name.name.clone());
            if let Some(module) = inline.or_else(|| sources.get(source_path)) {
                self.collect_binding_surfaces(module, child, index, sources, source_path, directives);
            }
            source_path.pop();
        }
    }
}

//! Macro-only discovery over parsed modules and direct dependency descriptors.
//! No ordinary headers, type inference, filesystem reads, or worker execution.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::ast::{MacroDecl, Module, NodeId, Path, TopLevel};
use crate::diagnostic::Diagnostics;
use crate::lexer::Span;

use super::proc_macro::{ProcMacroArtifact, ProcMacroExport};
use super::{proc_macro_error, MacroExpansionContext};

#[derive(Debug, Clone)]
pub struct ProcMacroRegistryEntry {
    pub artifact: ProcMacroArtifact,
    pub export: ProcMacroExport,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MacroIdentity {
    Source { crate_identity: String, definition: NodeId },
    Procedural { crate_identity: String, export_identity: String },
}

#[derive(Debug, Clone)]
pub enum MacroDefinition {
    Declarative(MacroDecl),
    Procedural(ProcMacroRegistryEntry),
}

#[derive(Debug, Clone)]
pub struct MacroEntry {
    pub identity: MacroIdentity,
    pub definition: MacroDefinition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ModuleTarget {
    Source(Vec<String>),
    Dependency { name: String, prefix: Vec<String> },
}

#[derive(Debug, Clone, Default)]
struct Scope {
    declarations: BTreeMap<String, MacroIdentity>,
    imports: Vec<(Vec<String>, Span)>,
    bindings: BTreeMap<String, MacroIdentity>,
    module_aliases: BTreeMap<String, ModuleTarget>,
    exports: HashMap<String, Option<String>>,
    globs: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Default)]
pub struct MacroRegistry {
    entries: HashMap<MacroIdentity, MacroEntry>,
    scopes: BTreeMap<Vec<String>, Scope>,
    dependencies: BTreeMap<String, BTreeMap<Vec<String>, MacroIdentity>>,
    current_crate: Option<String>,
}

impl MacroRegistry {
    pub fn build(module: &Module, context: &MacroExpansionContext<'_>) -> Result<Self, Diagnostics> {
        Self::build_with_modules(module, &context.source_modules, context)
    }

    pub(super) fn build_with_modules(module: &Module, sources: &crate::source_loader::SourceModuleSet, context: &MacroExpansionContext<'_>) -> Result<Self, Diagnostics> {
        Self::build_scopes(module, sources, context, true)
    }

    pub(super) fn build_for_expansion(module: &Module, sources: &crate::source_loader::SourceModuleSet, context: &MacroExpansionContext<'_>) -> Result<Self, Diagnostics> {
        Self::build_scopes(module, sources, context, false)
    }

    fn build_scopes(module: &Module, sources: &crate::source_loader::SourceModuleSet, context: &MacroExpansionContext<'_>, strict: bool) -> Result<Self, Diagnostics> {
        let mut registry = Self { current_crate: context.config.current_crate_name.clone(), ..Self::default() };
        registry.index_module(module, Vec::new())?;
        // These are source-module identities from the loader, not pathname-based
        // guesses. The root AST remains authoritative for inline declarations.
        for loaded in sources.modules() {
            let mut path: Vec<_> = loaded.qualified_name.split("::").map(String::from).collect();
            if path.first().map(String::as_str) == registry.current_crate.as_deref() { path.remove(0); }
            if !path.is_empty() {
                registry.index_module(&loaded.module, path)?;
            }
        }
        for (name, _) in &context.config.extern_artifacts {
            registry.dependencies.entry(name.clone()).or_default();
        }
        let mut descriptors: Vec<_> = context.proc_macro_artifacts.iter().collect();
        descriptors.sort_by(|a, b| a.name.cmp(&b.name).then(a.artifact.crate_identity.cmp(&b.artifact.crate_identity)));
        let mut providers = BTreeMap::new();
        for dependency in descriptors {
            let artifact = &dependency.artifact;
            if dependency.name.is_empty() || artifact.crate_identity.is_empty() {
                return Err(proc_macro_error("macro dependency has an empty identity", module_span(module)));
            }
            if let Some(provider) = providers.insert(dependency.name.clone(), artifact.crate_identity.clone()) {
                if provider != artifact.crate_identity {
                    return Err(proc_macro_error(format!("conflicting macro dependency '{}'", dependency.name), module_span(module)));
                }
            }
            let surface = registry.dependencies.entry(dependency.name.clone()).or_default();
            let mut declared = BTreeSet::new();
            for export in &artifact.exports {
                let path: Vec<_> = export.name.split("::").map(String::from).collect();
                if path.iter().any(String::is_empty) || export.identity.is_empty() {
                    return Err(proc_macro_error("macro export has an invalid name or identity", module_span(module)));
                }
                if !declared.insert(path.clone()) {
                    return Err(proc_macro_error(format!("conflicting macro export '{}'", export.name), module_span(module)));
                }
                let identity = MacroIdentity::Procedural {
                    crate_identity: artifact.crate_identity.clone(), export_identity: export.identity.clone(),
                };
                if let Some(previous) = surface.insert(path, identity.clone()) {
                    if previous != identity { return Err(proc_macro_error(format!("conflicting macro export '{}'", export.name), module_span(module))); }
                }
                if let Some(previous) = registry.entries.get(&identity) {
                    let MacroDefinition::Procedural(previous) = &previous.definition else {
                        return Err(proc_macro_error("conflicting macro identity", module_span(module)));
                    };
                    if previous.artifact != *artifact {
                        return Err(proc_macro_error(format!("conflicting macro provider '{}'", artifact.crate_identity), module_span(module)));
                    }
                } else {
                    registry.entries.insert(identity.clone(), MacroEntry { identity, definition: MacroDefinition::Procedural(
                        ProcMacroRegistryEntry { artifact: artifact.clone(), export: export.clone() },
                    ) });
                }
            }
        }
        registry.bind_imports(strict)?;
        Ok(registry)
    }

    fn index_module(&mut self, module: &Module, path: Vec<String>) -> Result<(), Diagnostics> {
        if self.scopes.contains_key(&path) {
            return Err(proc_macro_error(format!("conflicting source macro module '{}'", path.join("::")), module_span(module)));
        }
        let mut scope = Scope { exports: crate::collect::collect_exports(module), ..Scope::default() };
        for item in &module.top_levels {
            match item {
                TopLevel::MacroDecl(declaration) => {
                    let identity = MacroIdentity::Source {
                        crate_identity: self.current_crate.clone().unwrap_or_else(|| "<current>".into()), definition: declaration.id,
                    };
                    if scope.declarations.insert(declaration.name.name.clone(), identity.clone()).is_some() {
                        return Err(proc_macro_error(format!("conflicting macro declaration '{}'", declaration.name.name), declaration.name.span.clone()));
                    }
                    self.entries.insert(identity.clone(), MacroEntry { identity, definition: MacroDefinition::Declarative(declaration.clone()) });
                }
                TopLevel::Import(import) => {
                    let segments = match import { Path::Ident(path) => &path.path, Path::Type(path) => &path.path };
                    let names = crate::collect::path_names(segments);
                    if names.len() == segments.len() {
                        let span = segments.first().map(|segment| match segment {
                            crate::ast::IdentOrType::Ident(ident) => ident.span.clone(),
                            crate::ast::IdentOrType::Type(ty) => ty.span(),
                        }).unwrap_or_else(|| module_span(module));
                        scope.imports.push((names, span));
                    }
                }
                TopLevel::GlobImport(path) => scope.globs.push(path.clone()),
                _ => {}
            }
        }
        scope.bindings = scope.declarations.clone();
        self.scopes.insert(path.clone(), scope);
        for item in &module.top_levels {
            if let TopLevel::Module(child) = item {
                if let Some(name) = &child.0.name {
                    let mut child_path = path.clone(); child_path.push(name.name.clone());
                    self.index_module(&child.0, child_path)?;
                }
            }
        }
        Ok(())
    }

    fn bind_imports(&mut self, strict: bool) -> Result<(), Diagnostics> {
        let imports: Vec<_> = self.scopes.iter().flat_map(|(scope, data)| data.imports.iter()
            .map(move |(path, span)| (scope.clone(), path.clone(), span.clone()))).collect();
        // Module aliases are ordinary explicit module imports. They never
        // populate this scope with every macro exported by the target module.
        for _ in 0..imports.len() {
            let mut changed = false;
            for (scope, path, span) in &imports {
                if let Some(target) = self.module_target(scope, path) {
                    if let Err(diagnostics) = self.check_module_access(scope, &target, span) {
                        if strict { return Err(diagnostics); }
                        continue;
                    }
                    let Some(alias) = path.last() else { continue; };
                    let data = self.scopes.get_mut(scope).ok_or_else(|| proc_macro_error("unknown macro scope", span.clone()))?;
                    match data.module_aliases.get(alias) {
                        Some(previous) if previous != &target => return Err(proc_macro_error(format!("conflicting module import '{alias}'"), span.clone())),
                        Some(_) => {}
                        None => { data.module_aliases.insert(alias.clone(), target); changed = true; }
                    }
                }
            }
            if !changed { break; }
        }
        for (scope, path, span) in imports {
            let resolved = match self.resolve_identity(&scope, &path, &span, false) {
                Ok(resolved) => resolved,
                Err(diagnostics) if strict => return Err(diagnostics),
                Err(_) => continue,
            };
            if let Some(identity) = resolved {
                let Some(alias) = path.last() else { continue; };
                let data = self.scopes.get_mut(&scope).ok_or_else(|| proc_macro_error("unknown macro scope", span.clone()))?;
                if let Some(previous) = data.bindings.insert(alias.clone(), identity.clone()) {
                    if previous != identity { return Err(proc_macro_error(format!("conflicting macro import '{alias}'"), span)); }
                }
            }
        }
        Ok(())
    }

    pub fn resolve(&self, scope: &[String], path: &str, span: &Span) -> Result<Option<&MacroEntry>, Diagnostics> {
        let names: Vec<_> = path.split("::").map(String::from).collect();
        if let Some(identity) = self.resolve_identity(scope, &names, span, true)? { return Ok(self.entries.get(&identity)); }
        if let Some(cycle) = self.import_cycle(scope, &names) {
            return Err(proc_macro_error(format!("macro import cycle: {}", cycle.join(" -> ")), span.clone()));
        }
        if names.len() == 1 {
            if let Some(data) = self.scopes.get(scope) {
                for glob in &data.globs {
                    let mut candidate = glob.clone(); candidate.extend(names.iter().cloned());
                    if self.resolve_identity(scope, &candidate, span, false)?.is_some() {
                        return Err(proc_macro_error("macro glob imports are not implemented; use an explicit import", span.clone()));
                    }
                }
            }
        }
        Ok(None)
    }

    fn import_cycle(&self, scope: &[String], path: &[String]) -> Option<Vec<String>> {
        let data = self.scopes.get(scope)?;
        let mut current = path.first()?.clone();
        let mut chain = Vec::new();
        for _ in 0..=data.imports.len() {
            if data.bindings.contains_key(&current) || data.module_aliases.contains_key(&current) { return None; }
            if let Some(start) = chain.iter().position(|name| name == &current) {
                chain.push(current);
                return Some(chain[start..].to_vec());
            }
            chain.push(current.clone());
            let (import, _) = data.imports.iter().find(|(import, _)| import.len() > 1 && import.last() == Some(&current))?;
            current = import[0].clone();
        }
        None
    }

    fn resolve_identity(&self, scope: &[String], path: &[String], span: &Span, macro_request: bool) -> Result<Option<MacroIdentity>, Diagnostics> {
        let Some(name) = path.last() else { return Ok(None); };
        if path.len() == 1 {
            let Some(data) = self.scopes.get(scope) else { return Ok(None); };
            if let Some(identity) = data.bindings.get(name) { return Ok(Some(identity.clone())); }
            // Keep an unresolved ordinary import inert until a macro use asks
            // for it. This diagnoses deferred facades without binding through
            // them or rejecting value-only imports during macro discovery.
            if macro_request {
                for (import, _) in &data.imports {
                    if import.len() > 1 && import.last() == Some(name) {
                        self.resolve_identity(scope, import, span, true)?;
                    }
                }
            }
            return Ok(None);
        }
        let Some(target) = self.module_target(scope, &path[..path.len() - 1]) else { return Ok(None); };
        self.check_module_access(scope, &target, span)?;
        match target {
            ModuleTarget::Dependency { name: dependency, mut prefix } => {
                prefix.push(name.clone());
                let identity = self.dependencies.get(&dependency).and_then(|surface| surface.get(&prefix)).cloned();
                if identity.is_none() && macro_request {
                    return Err(proc_macro_error(format!("dependency '{dependency}' has no exported procedural macro '{}'; external declarative macro publication is not implemented", prefix.join("::")), span.clone()));
                }
                Ok(identity)
            }
            ModuleTarget::Source(module) => {
                let Some(data) = self.scopes.get(&module) else { return Ok(None); };
                if module != scope {
                    if data.exports.get(name).is_some_and(Option::is_some) {
                        return if macro_request {
                            Err(proc_macro_error("macro re-export/facade resolution is not implemented", span.clone()))
                        } else { Ok(None) };
                    }
                    if data.bindings.contains_key(name) && !data.exports.contains_key(name) {
                        return Err(proc_macro_error(format!("private macro '{}::{}'", module.join("::"), name), span.clone()));
                    }
                    if data.exports.contains_key(name) && !data.declarations.contains_key(name) && data.bindings.contains_key(name) {
                        return if macro_request {
                            Err(proc_macro_error("macro re-export/facade resolution is not implemented", span.clone()))
                        } else { Ok(None) };
                    }
                }
                Ok(data.declarations.get(name).cloned())
            }
        }
    }

    fn module_target(&self, scope: &[String], path: &[String]) -> Option<ModuleTarget> {
        if path.is_empty() { return Some(ModuleTarget::Source(scope.to_vec())); }
        if let Some(alias) = self.scopes.get(scope).and_then(|data| data.module_aliases.get(&path[0])) {
            let mut target = alias.clone();
            match &mut target {
                ModuleTarget::Source(module) => module.extend_from_slice(&path[1..]),
                ModuleTarget::Dependency { prefix, .. } => prefix.extend_from_slice(&path[1..]),
            }
            return self.target_exists(target);
        }
        if self.dependencies.contains_key(&path[0]) {
            return self.target_exists(ModuleTarget::Dependency { name: path[0].clone(), prefix: path[1..].to_vec() });
        }
        if self.current_crate.as_ref() == Some(&path[0]) {
            return self.target_exists(ModuleTarget::Source(path[1..].to_vec()));
        }
        let mut local = scope.to_vec(); local.extend_from_slice(path);
        self.target_exists(ModuleTarget::Source(local))
            .or_else(|| self.target_exists(ModuleTarget::Source(path.to_vec())))
    }

    fn target_exists(&self, target: ModuleTarget) -> Option<ModuleTarget> {
        let exists = match &target {
            ModuleTarget::Source(path) => self.scopes.contains_key(path),
            ModuleTarget::Dependency { name, prefix } => self.dependencies.get(name)
                .is_some_and(|surface| prefix.is_empty() || surface.keys().any(|path| path.len() > prefix.len() && path.starts_with(prefix))),
        };
        exists.then_some(target)
    }

    fn check_module_access(&self, requester: &[String], target: &ModuleTarget, span: &Span) -> Result<(), Diagnostics> {
        let ModuleTarget::Source(path) = target else { return Ok(()); };
        for index in 0..path.len() {
            let parent = &path[..index];
            if requester.starts_with(parent) { continue; }
            if !self.scopes.get(parent).is_some_and(|scope| scope.exports.contains_key(&path[index])) {
                return Err(proc_macro_error(format!("private source module '{}'", path[..=index].join("::")), span.clone()));
            }
        }
        Ok(())
    }
}

fn module_span(module: &Module) -> Span {
    module.name.as_ref().map(|name| name.span.clone())
        .unwrap_or_else(|| Span::new(module.filepath.clone().unwrap_or_else(|| "<macro-scope>".into()), 0, 0))
}

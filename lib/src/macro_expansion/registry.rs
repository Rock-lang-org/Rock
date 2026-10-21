//! Macro-only discovery over parsed modules and direct dependency descriptors.
//! No ordinary headers, type inference, filesystem reads, or worker execution.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::ast::{MacroDecl, Module, NodeId, Path, ProcMacroDecl, TopLevel};
use crate::diagnostic::Diagnostics;
use crate::lexer::Span;
use crate::lexer::lossless::SyntaxContext;

use super::proc_macro::{ProcMacroArtifact, ProcMacroExport};
use super::{proc_macro_error, MacroExpansionContext};

pub(super) type BindingName = (String, SyntaxContext);
pub(super) type BindingPath = Vec<BindingName>;
fn display(path: &[BindingName]) -> String { path.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>().join("::") }
fn root_path(path: &[String]) -> BindingPath { path.iter().map(|name| (name.clone(), SyntaxContext::ROOT)).collect() }

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
    SourceProcedural(ProcMacroDecl),
}

/// Source identities retained for the subsequent checked host-wrapper build.
#[derive(Debug, Clone)]
pub struct SourceProcMacroExport {
    pub definition: NodeId,
    pub module: Vec<(String, SyntaxContext)>,
    pub export: ProcMacroExport,
}

#[derive(Debug, Clone)]
pub struct MacroEntry {
    pub identity: MacroIdentity,
    pub definition: MacroDefinition,
}

impl MacroEntry {
    pub(super) fn proc_kind(&self) -> Option<&super::proc_macro::ProcMacroKind> {
        match &self.definition {
            MacroDefinition::Declarative(_) => None,
            MacroDefinition::Procedural(entry) => Some(&entry.export.kind),
            MacroDefinition::SourceProcedural(declaration) => Some(&declaration.kind),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ModuleTarget {
    Source(BindingPath),
    Dependency { name: String, prefix: Vec<String> },
}

#[derive(Debug, Clone, Default)]
struct Scope {
    declarations: BTreeMap<BindingName, MacroIdentity>,
    imports: Vec<(BindingPath, Span)>,
    bindings: BTreeMap<BindingName, MacroIdentity>,
    module_aliases: BTreeMap<BindingName, ModuleTarget>,
    exports: HashMap<BindingName, Option<BindingPath>>,
    globs: Vec<BindingPath>,
}

#[derive(Debug, Clone, Default)]
pub struct MacroRegistry {
    entries: HashMap<MacroIdentity, MacroEntry>,
    scopes: BTreeMap<BindingPath, Scope>,
    file_scopes: BTreeMap<Vec<String>, BindingPath>,
    dependencies: BTreeMap<String, BTreeMap<Vec<String>, MacroIdentity>>,
    current_crate: Option<String>,
}

impl MacroRegistry {
    pub fn build(module: &Module, context: &MacroExpansionContext<'_>) -> Result<Self, Diagnostics> {
        Self::build_with_modules(module, &context.source_modules, context)
    }

    pub(crate) fn build_with_modules(module: &Module, sources: &crate::source_loader::SourceModuleSet, context: &MacroExpansionContext<'_>) -> Result<Self, Diagnostics> {
        Self::build_scopes(module, sources, context, true)
    }

    pub(super) fn build_for_expansion(module: &Module, sources: &crate::source_loader::SourceModuleSet, context: &MacroExpansionContext<'_>) -> Result<Self, Diagnostics> {
        Self::build_scopes(module, sources, context, false)
    }

    fn build_scopes(module: &Module, sources: &crate::source_loader::SourceModuleSet, context: &MacroExpansionContext<'_>, strict: bool) -> Result<Self, Diagnostics> {
        let mut registry = Self { current_crate: context.config.current_crate_name.clone(), ..Self::default() };
        registry.index_module(module, Vec::new(), context)?;
        // These are source-module identities from the loader, not pathname-based
        // guesses. The root AST remains authoritative for inline declarations.
        let mut modules = sources.modules().collect::<Vec<_>>();
        modules.sort_by_key(|loaded| loaded.qualified_name.matches("::").count());
        for loaded in modules {
            let mut path: Vec<_> = loaded.qualified_name.split("::").map(String::from).collect();
            if path.first().map(String::as_str) == registry.current_crate.as_deref() { path.remove(0); }
            if !path.is_empty() {
                let bindings = registry.loaded_scope(&path);
                registry.index_module(&loaded.module, bindings, context)?;
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
        registry.bind_imports(strict, context)?;
        Ok(registry)
    }

    fn index_module(&mut self, module: &Module, path: BindingPath, context: &MacroExpansionContext<'_>) -> Result<(), Diagnostics> {
        if self.scopes.contains_key(&path) {
            return Err(proc_macro_error(format!("conflicting source macro module '{}'", display(&path)), module_span(module)));
        }
        let mut scope = Scope::default();
        for item in &module.top_levels {
            match item {
                TopLevel::MacroDecl(declaration) => {
                    context.record_definition_scope(declaration.id, path.clone())
                        .map_err(|message| proc_macro_error(message, declaration.name.span.clone()))?;
                    let identity = MacroIdentity::Source {
                        crate_identity: self.current_crate.clone().unwrap_or_else(|| "<current>".into()), definition: declaration.id,
                    };
                    if scope.declarations.insert((declaration.name.name.clone(), declaration.name.context), identity.clone()).is_some() {
                        return Err(proc_macro_error(format!("conflicting macro declaration '{}'", declaration.name.name), declaration.name.span.clone()));
                    }
                    self.entries.insert(identity.clone(), MacroEntry { identity, definition: MacroDefinition::Declarative(declaration.clone()) });
                }
                TopLevel::ProcMacroDecl(declaration) => {
                    let name = &declaration.function.name;
                    if context.config.target_role != crate::crate_system::TargetRole::ProcMacro {
                        return Err(proc_macro_error("source procedural macro declarations require a [lib] with proc_macro = true", name.span.clone()));
                    }
                    let identity = MacroIdentity::Source {
                        crate_identity: self.current_crate.clone().unwrap_or_else(|| "<current>".into()), definition: declaration.id,
                    };
                    let binding = (name.name.clone(), name.context);
                    if scope.declarations.insert(binding.clone(), identity.clone()).is_some() {
                        return Err(proc_macro_error(format!("conflicting macro declaration '{}'", name.name), name.span.clone()));
                    }
                    if declaration.function.exported { scope.exports.insert(binding, None); }
                    self.entries.insert(identity.clone(), MacroEntry { identity, definition: MacroDefinition::SourceProcedural(declaration.clone()) });
                }
                TopLevel::Import(import) => {
                    let segments = match import { Path::Ident(path) => &path.path, Path::Type(path) => &path.path };
                    if let Some(names) = crate::collect::resolver::binding_path(segments) {
                        let span = segments.first().map(|segment| match segment {
                            crate::ast::IdentOrType::Ident(ident) => ident.span.clone(),
                            crate::ast::IdentOrType::Type(ty) => ty.span(),
                        }).unwrap_or_else(|| module_span(module));
                        scope.imports.push((names, span));
                    }
                }
                TopLevel::GlobImport(path) => if let Some(path) = crate::collect::resolver::binding_path(path) { scope.globs.push(path); },
                TopLevel::Export(export) => {
                    let segments = match export { Path::Ident(path) => &path.path, Path::Type(path) => &path.path };
                    if let Some(names) = crate::collect::resolver::binding_path(segments) {
                        if let Some(name) = names.last().cloned() {
                            let source = (names.len() > 1).then_some(names);
                            if let Some(path) = &source {
                                let span = segments.first().map(|segment| match segment {
                                    crate::ast::IdentOrType::Ident(ident) => ident.span.clone(),
                                    crate::ast::IdentOrType::Type(ty) => ty.span(),
                                }).unwrap_or_else(|| module_span(module));
                                scope.imports.push((path.clone(), span));
                            }
                            scope.exports.insert(name, source);
                        }
                    }
                }
                TopLevel::Mod(name, exported) => {
                    let mut bindings = path.clone(); bindings.push((name.name.clone(), name.context));
                    let names = bindings.iter().map(|(name, _)| name.clone()).collect();
                    if self.file_scopes.get(&names).is_some_and(|previous| *previous != bindings) {
                        return Err(proc_macro_error("a source file module has conflicting binding contexts", name.span.clone()));
                    }
                    self.file_scopes.insert(names, bindings);
                    if *exported { scope.exports.insert((name.name.clone(), name.context), None); }
                }
                _ => {}
            }
        }
        scope.bindings = scope.declarations.clone();
        self.scopes.insert(path.clone(), scope);
        for item in &module.top_levels {
            if let TopLevel::Module(child) = item {
                if let Some(name) = &child.0.name {
                    let mut child_path = path.clone(); child_path.push((name.name.clone(), name.context));
                    self.index_module(&child.0, child_path, context)?;
                }
            }
        }
        Ok(())
    }

    fn bind_imports(&mut self, strict: bool, context: &MacroExpansionContext<'_>) -> Result<(), Diagnostics> {
        let imports: Vec<_> = self.scopes.iter().flat_map(|(scope, data)| data.imports.iter()
            .map(move |(path, span)| (scope.clone(), path.clone(), span.clone()))).collect();
        // Module aliases are ordinary explicit module imports. They never
        // populate this scope with every macro exported by the target module.
        for _ in 0..imports.len() {
            let mut changed = false;
            for (scope, path, span) in &imports {
                let mut selected = None;
                let mut failure = None;
                for (requester, names) in self.lookup_paths(scope, path, span, context)? {
                    let Some(target) = self.module_target(&requester, &names) else { continue; };
                    match self.check_module_access(&requester, &target, span) {
                        Ok(()) => { selected = Some(target); break; }
                        Err(diagnostics) => { failure.get_or_insert(diagnostics); }
                    }
                }
                if selected.is_none() && strict {
                    if let Some(diagnostics) = failure { return Err(diagnostics); }
                }
                if let Some(target) = selected {
                    let Some(alias) = path.last() else { continue; };
                    let data = self.scopes.get_mut(scope).ok_or_else(|| proc_macro_error("unknown macro scope", span.clone()))?;
                    match data.module_aliases.get(alias) {
                        Some(previous) if previous != &target => return Err(proc_macro_error(format!("conflicting module import '{}'", alias.0), span.clone())),
                        Some(_) => {}
                        None => { data.module_aliases.insert(alias.clone(), target); changed = true; }
                    }
                }
            }
            if !changed { break; }
        }
        for (scope, path, span) in imports {
            let mut resolved = None;
            let mut failure = None;
            for (requester, names) in self.lookup_paths(&scope, &path, &span, context)? {
                match self.resolve_identity(&requester, &names, &span, false) {
                    Ok(Some(identity)) => { resolved = Some(identity); break; }
                    Ok(None) => {}
                    Err(diagnostics) => { failure.get_or_insert(diagnostics); }
                }
            }
            if resolved.is_none() && strict {
                if let Some(diagnostics) = failure { return Err(diagnostics); }
            }
            if let Some(identity) = resolved {
                let Some(alias) = path.last() else { continue; };
                let data = self.scopes.get_mut(&scope).ok_or_else(|| proc_macro_error("unknown macro scope", span.clone()))?;
                if let Some(previous) = data.bindings.insert(alias.clone(), identity.clone()) {
                    if previous != identity { return Err(proc_macro_error(format!("conflicting macro import '{}'", alias.0), span)); }
                }
            }
        }
        Ok(())
    }

    pub fn resolve(&self, scope: &[String], path: &str, span: &Span) -> Result<Option<&MacroEntry>, Diagnostics> {
        self.resolve_bindings(&root_path(scope), &path.split("::").map(|name| (name.to_string(), SyntaxContext::ROOT)).collect::<Vec<_>>(), span)
    }

    /// Enumerate only direct, publicly reachable source declarations. Ordinary
    /// function exports are not macro registrations, and facades remain deferred.
    pub(crate) fn source_proc_exports(&self, crate_identity: &str) -> Result<Vec<SourceProcMacroExport>, Diagnostics> {
        let mut exports = BTreeMap::new();
        for (module, scope) in &self.scopes {
            if !(0..module.len()).all(|index| self.scopes.get(&module[..index])
                .is_some_and(|parent| parent.exports.contains_key(&module[index]))) { continue; }
            for name in scope.exports.keys() {
                let Some(identity) = scope.bindings.get(name) else { continue; };
                let Some(entry) = self.entries.get(identity) else { continue; };
                if !scope.declarations.contains_key(name) || scope.exports[name].is_some() {
                    if matches!(entry.definition, MacroDefinition::SourceProcedural(_) | MacroDefinition::Procedural(_)) {
                        return Err(proc_macro_error("macro re-export/facade publication is not implemented", match &entry.definition {
                            MacroDefinition::SourceProcedural(declaration) => declaration.function.name.span.clone(),
                            _ => Span::new("<macro-export>".into(), 0, 0),
                        }));
                    }
                    continue;
                }
                let MacroDefinition::SourceProcedural(declaration) = &entry.definition else { continue; };
                let mut path = module.clone(); path.push(name.clone());
                let public_name = display(&path);
                let export = SourceProcMacroExport {
                    definition: declaration.id, module: module.clone(),
                    export: ProcMacroExport { name: public_name.clone(), identity: format!("{crate_identity}::{public_name}"),
                        kind: declaration.kind.clone(),
                        input_shape: super::proc_macro::ProcMacroInputShape::TokenStream },
                };
                if exports.insert(public_name.clone(), export).is_some() {
                    return Err(proc_macro_error(format!("conflicting public macro name '{public_name}'"), declaration.function.name.span.clone()));
                }
            }
        }
        Ok(exports.into_values().collect())
    }

    pub(super) fn loaded_scope(&self, path: &[String]) -> BindingPath {
        self.file_scopes.get(path).cloned().unwrap_or_else(|| root_path(path))
    }

    pub(super) fn resolve_contextual(&self, scope: &[BindingName], path: &[BindingName], span: &Span, context: &MacroExpansionContext<'_>) -> Result<Option<&MacroEntry>, Diagnostics> {
        let mut failure = None;
        for (scope, path) in self.lookup_paths(scope, path, span, context)? {
            match self.resolve_bindings(&scope, &path, span) {
                Ok(Some(entry)) => return Ok(Some(entry)),
                Ok(None) => {}
                Err(diagnostics) => { failure.get_or_insert(diagnostics); }
            }
        }
        match failure { Some(diagnostics) => Err(diagnostics), None => Ok(None) }
    }

    fn lookup_paths(&self, scope: &[BindingName], path: &[BindingName], span: &Span, context: &MacroExpansionContext<'_>) -> Result<Vec<(BindingPath, BindingPath)>, Diagnostics> {
        let mut scope = scope.to_vec();
        let mut path = path.to_vec();
        let mut paths = Vec::new();
        for _ in 0..=context.binding_environments.borrow().len() {
            paths.push((scope.clone(), path.clone()));
            // File members start in ROOT even under a private template module.
            // Retain the selected module while peeling member template marks.
            let mut members = path.clone();
            for _ in 0..context.binding_environments.borrow().len().saturating_mul(path.len()) {
                let Some(split) = (1..members.len()).rev().find(|split| self.module_target(&scope, &members[..*split]).is_some()) else { break; };
                let Some((_, original)) = context.binding_environments.borrow().source_scope(members[split].1) else { break; };
                members[split].1 = original;
                paths.push((scope.clone(), members.clone()));
            }
            let Some((_, current)) = path.first() else { return Ok(paths); };
            let Some((definition, original)) = context.binding_environments.borrow().source_scope(*current) else { return Ok(paths); };
            if path.iter().any(|(_, other)| *other != SyntaxContext::ROOT && other != current) {
                return Err(proc_macro_error("conflicting macro path contexts", span.clone()));
            }
            scope = definition;
            path = path.iter().map(|(name, inherited)| (name.clone(), if inherited == current { original } else { *inherited })).collect();
        }
        Err(proc_macro_error("macro definition context ancestry exceeded", span.clone()))
    }

    fn resolve_bindings(&self, scope: &[BindingName], path: &[BindingName], span: &Span) -> Result<Option<&MacroEntry>, Diagnostics> {
        let names = path.to_vec();
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

    fn import_cycle(&self, scope: &[BindingName], path: &[BindingName]) -> Option<Vec<String>> {
        let data = self.scopes.get(scope)?;
        let mut current = path.first()?.clone();
        let mut chain = Vec::new();
        for _ in 0..=data.imports.len() {
            if data.bindings.contains_key(&current) || data.module_aliases.contains_key(&current) { return None; }
            if let Some(start) = chain.iter().position(|name| name == &current) {
                chain.push(current);
                return Some(chain[start..].iter().map(|(name, _): &BindingName| name.clone()).collect());
            }
            chain.push(current.clone());
            let (import, _) = data.imports.iter().find(|(import, _)| import.len() > 1 && import.last() == Some(&current))?;
            current = import[0].clone();
        }
        None
    }

    fn resolve_identity(&self, scope: &[BindingName], path: &[BindingName], span: &Span, macro_request: bool) -> Result<Option<MacroIdentity>, Diagnostics> {
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
                if name.1 != SyntaxContext::ROOT { return Ok(None); }
                prefix.push(name.0.clone());
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
                        return Err(proc_macro_error(format!("private macro '{}::{}'", display(&module), name.0), span.clone()));
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

    fn module_target(&self, scope: &[BindingName], path: &[BindingName]) -> Option<ModuleTarget> {
        if path.is_empty() { return Some(ModuleTarget::Source(scope.to_vec())); }
        if let Some(alias) = self.scopes.get(scope).and_then(|data| data.module_aliases.get(&path[0])) {
            let mut target = alias.clone();
            match &mut target {
                ModuleTarget::Source(module) => module.extend_from_slice(&path[1..]),
                ModuleTarget::Dependency { prefix, .. } => {
                    if path[1..].iter().any(|(_, context)| *context != SyntaxContext::ROOT) { return None; }
                    prefix.extend(path[1..].iter().map(|(name, _)| name.clone()));
                }
            }
            return self.target_exists(target);
        }
        if path.iter().all(|(_, context)| *context == SyntaxContext::ROOT) && self.dependencies.contains_key(&path[0].0) {
            return self.target_exists(ModuleTarget::Dependency { name: path[0].0.clone(), prefix: path[1..].iter().map(|(name, _)| name.clone()).collect() });
        }
        if path[0].1 == SyntaxContext::ROOT && self.current_crate.as_ref() == Some(&path[0].0) {
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

    fn check_module_access(&self, requester: &[BindingName], target: &ModuleTarget, span: &Span) -> Result<(), Diagnostics> {
        let ModuleTarget::Source(path) = target else { return Ok(()); };
        for index in 0..path.len() {
            let parent = &path[..index];
            if requester.starts_with(parent) { continue; }
            if !self.scopes.get(parent).is_some_and(|scope| scope.exports.contains_key(&path[index])) {
                return Err(proc_macro_error(format!("private source module '{}'", display(&path[..=index])), span.clone()));
            }
        }
        Ok(())
    }
}

fn module_span(module: &Module) -> Span {
    module.name.as_ref().map(|name| name.span.clone())
        .unwrap_or_else(|| Span::new(module.filepath.clone().unwrap_or_else(|| "<macro-scope>".into()), 0, 0))
}

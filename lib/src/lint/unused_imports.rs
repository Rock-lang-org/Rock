//! `unused_imports`: `>` imports not used in the importing module or its
//! submodules.
//!
//! Imports of functions, externs, structs, enums, type aliases and traits use
//! the semantic references recorded by collection and lowering, paired with
//! the written path head: locals, fields, declarations and fully qualified
//! references do not use a same-named import. Imports without a resolved item
//! target (variants, modules) fall back to path-head name occurrences. Macro
//! imports use expansion's selected-binding records.
//! A glob `> m::*` is used when a reference's head name resolves to `m::name`
//! for the same item, unless a nearer declaration or explicit import (which
//! shadows globs) binds that name first; members without recorded references
//! (variants, modules) use the path-head name check. The injected std prelude
//! is not an import.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::ast::visit::{
    walk_expression, walk_impl, walk_module, walk_parse_type_inner, walk_top_level,
    walk_trait_decl, Visitor,
};
use crate::ast::{self, IdentOrType, ParseTypeInner, TopLevel};
use crate::collect::path_names;
use crate::diagnostic::Diagnostic;
use crate::ids::DefId;
use crate::infer::ResolvedHirProgram;
use crate::lexer::Span;
use crate::source_map::SourceSymbol;
use crate::walk_list;

use super::{Lint, LintSink};

/// The current crate's modules after macro expansion: the root module and
/// every file module by crate-relative path (`a::b`).
pub struct SourceModules<'a> {
    pub root: &'a ast::Module,
    pub files: BTreeMap<String, &'a ast::Module>,
    pub current_crate_name: Option<&'a str>,
}

pub(crate) fn check_unused_imports(
    modules: &SourceModules<'_>,
    hir: &ResolvedHirProgram,
    macro_imports: &[crate::macro_expansion::MacroImportUsage],
    sink: &mut LintSink<'_>,
) {
    if !sink.is_enabled(Lint::UnusedImports) {
        return;
    }
    for import in macro_imports.iter().filter(|import| !import.used) {
        emit_unused_import(&import.written, &import.span, sink);
    }
    let mut imported_items = HashSet::new();
    let mut has_globs = false;
    let mut has_imports = collect_item_import_targets(
        modules.root,
        "",
        hir,
        modules,
        &mut imported_items,
        &mut has_globs,
    );
    for (path, module) in &modules.files {
        has_imports |= collect_item_import_targets(
            module,
            path,
            hir,
            modules,
            &mut imported_items,
            &mut has_globs,
        );
    }
    if !has_imports {
        return;
    }
    // Index only references to explicitly imported items, not every reference
    // in the crate; a glob can provide any item, so globs index all of them.
    let mut references: HashMap<_, Vec<_>> = HashMap::new();
    if has_globs || !imported_items.is_empty() {
        for reference in hir.source_map.references() {
            if let SourceSymbol::Definition(id) = reference.target {
                if has_globs || imported_items.contains(&id) {
                    references
                        .entry((reference.span.file_path.clone(), reference.span.start))
                        .or_default()
                        .push((reference.span.end, id));
                }
            }
        }
    }
    let mut scopes = Vec::new();
    collect_scopes(String::new(), modules.root, &references, &mut scopes);
    for (path, module) in &modules.files {
        collect_scopes(path.clone(), module, &references, &mut scopes);
    }

    for scope in &mut scopes {
        for import in &mut scope.imports {
            import.item_target = item_import_target(hir, modules, &scope.path, &import.written);
        }
    }
    let mut item_bindings: HashMap<_, Vec<_>> = HashMap::new();
    for (scope_index, scope) in scopes.iter().enumerate() {
        for (import_index, import) in scope.imports.iter().enumerate() {
            if let Some(id) = import.item_target {
                item_bindings
                    .entry((scope.path.as_str(), import.name.as_str()))
                    .or_default()
                    .push((id, scope_index, import_index));
            }
        }
    }
    let declared: HashMap<&str, &HashSet<String>> = scopes
        .iter()
        .map(|scope| (scope.path.as_str(), &scope.declared))
        .collect();
    let scope_by_path: HashMap<&str, usize> = scopes
        .iter()
        .enumerate()
        .map(|(index, scope)| (scope.path.as_str(), index))
        .collect();
    // What `M::name` names, for a glob `> M::*` in a scope, by (scope, glob, name).
    let mut glob_members: HashMap<(usize, usize, String), Option<DefId>> = HashMap::new();
    let mut glob_member = |scope: usize, glob: usize, name: &str| {
        *glob_members
            .entry((scope, glob, name.to_string()))
            .or_insert_with(|| {
                let scope = &scopes[scope];
                let written = format!("{}::{name}", scope.globs[glob].module);
                resolve_written(hir, modules, &scope.path, &written)
            })
    };
    let mut used_globs = HashSet::new();
    let mut used_items = HashSet::new();
    for scope in &scopes {
        let uses = scope
            .values
            .iter()
            .map(|(name, id)| (name, Some(*id)))
            .chain(scope.reexports.iter().map(|name| (name, None)));
        for (name, id) in uses {
            let mut path = scope.path.as_str();
            loop {
                // A module's own item shadows imports of enclosing modules.
                if declared.get(path).is_some_and(|names| names.contains(name)) {
                    break;
                }
                if let Some(bindings) = item_bindings.get(&(path, name.as_str())) {
                    // A nearer import owns this name even if its target differs.
                    if let Some((_, scope, import)) = bindings
                        .iter()
                        .rev()
                        .find(|(target, _, _)| id.is_none_or(|id| *target == id))
                    {
                        used_items.insert((*scope, *import));
                    }
                    break;
                }
                // Explicit imports shadow globs of the same module.
                if let Some(&scope_index) = scope_by_path.get(path) {
                    let globs = scopes[scope_index].globs.len();
                    let provider = (0..globs).find(|&glob| {
                        glob_member(scope_index, glob, name)
                            .is_some_and(|member| id.is_none_or(|id| member == id))
                    });
                    if let Some(glob) = provider {
                        used_globs.insert((scope_index, glob));
                        break;
                    }
                }
                if path.is_empty() {
                    break;
                }
                path = path.rsplit_once("::").map_or("", |(parent, _)| parent);
            }
        }
    }

    for (scope_index, scope) in scopes.iter().enumerate() {
        for (glob_index, glob) in scope.globs.iter().enumerate() {
            if used_globs.contains(&(scope_index, glob_index)) {
                continue;
            }
            // Members without recorded references (variants, modules) keep
            // the path-head name check.
            let used = scopes
                .iter()
                .filter(|other| is_within(&other.path, &scope.path))
                .flat_map(|other| &other.names)
                .any(|name| match glob_member(scope_index, glob_index, name) {
                    Some(member) => !is_referenced_kind(hir, member),
                    None => names_variant(
                        hir,
                        modules,
                        &scope.path,
                        &format!("{}::{name}", glob.module),
                    ),
                });
            if !used {
                emit_unused_import(&format!("{}::*", glob.module), &glob.span, sink);
            }
        }
    }

    for (scope_index, scope) in scopes.iter().enumerate() {
        for (import_index, import) in scope.imports.iter().enumerate() {
            let used = if import.item_target.is_some() {
                used_items.contains(&(scope_index, import_index))
            } else {
                scopes
                    .iter()
                    .filter(|other| is_within(&other.path, &scope.path))
                    .any(|other| other.names.contains(&import.name))
            };
            if !used {
                emit_unused_import(&import.written, &import.span, sink);
            }
        }
    }
}

fn emit_unused_import(written: &str, span: &Span, sink: &mut LintSink<'_>) {
    sink.emit(
        Lint::UnusedImports,
        Diagnostic::new(format!("unused import: `{written}`"), span.clone())
            .with_help("remove the unused import"),
    );
}

fn collect_item_import_targets(
    module: &ast::Module,
    scope: &str,
    hir: &ResolvedHirProgram,
    modules: &SourceModules<'_>,
    targets: &mut HashSet<DefId>,
    has_globs: &mut bool,
) -> bool {
    let mut has_imports = false;
    for item in &module.top_levels {
        match item {
            TopLevel::GlobImport(..) => {
                has_imports = true;
                *has_globs = true;
            }
            TopLevel::Import(path) => {
                has_imports = true;
                let written = path_names(path.segments()).join("::");
                if let Some(id) = item_import_target(hir, modules, scope, &written) {
                    targets.insert(id);
                }
            }
            TopLevel::Module(child) => {
                let path = child.0.name.as_ref().map(|name| {
                    if scope.is_empty() {
                        name.name.clone()
                    } else {
                        format!("{scope}::{}", name.name)
                    }
                });
                has_imports |= collect_item_import_targets(
                    &child.0,
                    path.as_deref().unwrap_or(scope),
                    hir,
                    modules,
                    targets,
                    has_globs,
                );
            }
            _ => {}
        }
    }
    has_imports
}

fn item_import_target(
    hir: &ResolvedHirProgram,
    modules: &SourceModules<'_>,
    scope: &str,
    written: &str,
) -> Option<DefId> {
    resolve_written(hir, modules, scope, written).filter(|id| is_referenced_kind(hir, *id))
}

/// Items whose uses the source map records as `Definition` references.
fn is_referenced_kind(hir: &ResolvedHirProgram, id: DefId) -> bool {
    let program = &hir.program;
    program.functions.contains_key(&id)
        || program.externs.contains_key(&id)
        || program.structs.contains_key(&id)
        || program.enums.contains_key(&id)
        || program.type_aliases.contains_key(&id)
        || program.traits.contains_key(&id)
}

/// Resolves a path as written in module `scope`, like an import does.
fn resolve_written(
    hir: &ResolvedHirProgram,
    modules: &SourceModules<'_>,
    scope: &str,
    written: &str,
) -> Option<DefId> {
    written_candidates(modules, scope, written)
        .find_map(|path| hir.resolver.resolve_item_or_alias(&path))
}

/// Whether a path written in `scope` names an enum variant export. Variants
/// are not items and record no `Definition` references.
fn names_variant(
    hir: &ResolvedHirProgram,
    modules: &SourceModules<'_>,
    scope: &str,
    written: &str,
) -> bool {
    written_candidates(modules, scope, written)
        .any(|path| hir.resolver.resolve_variant_alias(None, &path).is_some())
}

/// The spellings an import path may resolve under: as written, relative to the
/// importing module, and crate-qualified forms of both.
fn written_candidates(
    modules: &SourceModules<'_>,
    scope: &str,
    written: &str,
) -> impl Iterator<Item = String> {
    let relative = if scope.is_empty() {
        written.to_string()
    } else {
        format!("{scope}::{written}")
    };
    let qualified = modules
        .current_crate_name
        .map(|name| format!("{name}::{relative}"));
    let root_qualified = modules
        .current_crate_name
        .map(|name| format!("{name}::{written}"));
    [
        Some(written.to_string()),
        Some(relative),
        qualified,
        root_qualified,
    ]
    .into_iter()
    .flatten()
}

/// True when module `path` is `scope` or one of its submodules.
fn is_within(path: &str, scope: &str) -> bool {
    scope.is_empty()
        || path == scope
        || path
            .strip_prefix(scope)
            .is_some_and(|rest| rest.starts_with("::"))
}

struct ImportSite {
    name: String,
    written: String,
    span: Span,
    item_target: Option<DefId>,
}

/// One module body: its imports and every other name written in it, nested
/// named modules excluded (they are scopes of their own).
struct ModuleScope {
    path: String,
    imports: Vec<ImportSite>,
    names: HashSet<String>,
    values: Vec<(String, DefId)>,
    reexports: HashSet<String>,
    declared: HashSet<String>,
    globs: Vec<GlobSite>,
}

/// `> module::*`: `module` as written, span over the whole path.
struct GlobSite {
    module: String,
    span: Span,
}

type ValueReferences = HashMap<(crate::lexer::SourcePath, usize), Vec<(usize, DefId)>>;

fn collect_scopes(
    path: String,
    module: &ast::Module,
    references: &ValueReferences,
    scopes: &mut Vec<ModuleScope>,
) {
    let mut collector = ScopeCollector {
        names: HashSet::new(),
        imports: Vec::new(),
        children: Vec::new(),
        references,
        values: Vec::new(),
        reexports: HashSet::new(),
        declaring: false,
        declared: HashSet::new(),
        globs: Vec::new(),
    };
    walk_module(&mut collector, module);
    let ScopeCollector {
        names,
        imports,
        children,
        values,
        reexports,
        declared,
        globs,
        ..
    } = collector;
    scopes.push(ModuleScope {
        path: path.clone(),
        imports,
        names,
        values,
        reexports,
        declared,
        globs,
    });
    for (name, child) in children {
        let child_path = if path.is_empty() {
            name
        } else {
            format!("{path}::{name}")
        };
        collect_scopes(child_path, child, references, scopes);
    }
}

struct ScopeCollector<'a, 'r> {
    names: HashSet<String>,
    imports: Vec<ImportSite>,
    children: Vec<(String, &'a ast::Module)>,
    references: &'r ValueReferences,
    values: Vec<(String, DefId)>,
    reexports: HashSet<String>,
    declaring: bool,
    declared: HashSet<String>,
    globs: Vec<GlobSite>,
}

impl<'a> Visitor<'a> for ScopeCollector<'a, '_> {
    fn visit_name(&mut self, name: &str) {
        self.names.insert(name.to_string());
    }

    fn visit_identifier_path(&mut self, node: &'a ast::IdentifierPath) {
        if let Some(span) = path_span(&node.path) {
            if let Some(targets) = self.references.get(&(span.file_path.clone(), span.start)) {
                if let Some(head) = path_names(&node.path).first() {
                    for (_, id) in targets.iter().filter(|(end, _)| *end <= span.end) {
                        self.values.push((head.clone(), *id));
                    }
                }
            }
        }
        self.visit_path_head(&node.path);
    }

    fn visit_type_path(&mut self, node: &'a ast::TypePath) {
        self.visit_path_head(&node.path);
    }

    fn visit_struct_decl(&mut self, node: &'a ast::StructDecl) {
        self.declared.insert(node.name.name.clone());
        self.declaring = true;
        crate::ast::visit::walk_struct_decl(self, node);
    }

    fn visit_enum_decl(&mut self, node: &'a ast::EnumDecl) {
        self.declared.insert(node.name.name.clone());
        self.declaring = true;
        crate::ast::visit::walk_enum_decl(self, node);
    }

    fn visit_enum_variant(&mut self, node: &'a ast::EnumVariant) {
        self.declaring = true;
        crate::ast::visit::walk_enum_variant(self, node);
    }

    fn visit_parse_type_inner(&mut self, node: &'a ParseTypeInner) {
        // A declared item name is a definition, not a use of an import.
        if std::mem::take(&mut self.declaring) {
            walk_parse_type_inner(self, node);
            return;
        }
        let span = &node.span;
        if let Some(targets) = self.references.get(&(span.file_path.clone(), span.start)) {
            if let Some(head) = node.name.split("::").next() {
                for (_, id) in targets.iter().filter(|(end, _)| *end <= span.end) {
                    self.values.push((head.to_string(), *id));
                }
            }
        }
        // Only the head of `a::B` binds through an import; later segments
        // are members of that binding, not uses of a same-named import.
        if let Some(head) = node.name.split("::").next() {
            self.names.insert(head.to_string());
        }
        walk_parse_type_inner(self, node);
    }

    // The shared walker skips these type positions; names written there still
    // use imports.
    fn visit_trait_decl(&mut self, node: &'a ast::TraitDecl) {
        self.declared.insert(node.name.name.clone());
        self.declaring = true;
        walk_trait_decl(self, node);
        walk_list!(self, visit_where_clause, &node.where_clauses);
        for associated in &node.associated_types {
            if let Some(kind) = &associated.kind {
                self.visit_type_application(kind);
            }
        }
    }

    fn visit_impl(&mut self, node: &'a ast::Impl) {
        walk_impl(self, node);
        for associated in &node.associated_types {
            self.visit_parse_type(&associated.ty);
            if let Some(kind) = &associated.kind {
                self.visit_type_application(kind);
            }
        }
        for signature in node.signatures.values() {
            self.visit_function_sig(signature);
        }
    }

    fn visit_expression(&mut self, node: &'a ast::Expression) {
        if let ast::Expression::CastExpr(_, ty) = node {
            self.visit_parse_type(ty);
        }
        walk_expression(self, node);
    }

    fn visit_operator(&mut self, node: &'a ast::Operator) {
        let span = &node.span;
        if let Some(targets) = self.references.get(&(span.file_path.clone(), span.start)) {
            for (_, id) in targets.iter().filter(|(end, _)| *end == span.end) {
                self.values.push((node.value.clone(), *id));
            }
        }
    }

    fn visit_module_decl(&mut self, node: &'a ast::ModuleDecl) {
        match &node.0.name {
            Some(name) => self.children.push((name.name.clone(), &node.0)),
            // An anonymous inline module shares its parent's scope.
            None => walk_module(self, &node.0),
        }
    }

    fn visit_top_level(&mut self, node: &'a TopLevel) {
        match node {
            TopLevel::Import(path) => {
                let names = path_names(path.segments());
                if let (Some(name), Some(span)) = (names.last(), path_span(path.segments())) {
                    self.imports.push(ImportSite {
                        name: name.clone(),
                        written: names.join("::"),
                        span,
                        item_target: None,
                    });
                }
            }
            TopLevel::GlobImport(path, span) => self.globs.push(GlobSite {
                module: path.join("::"),
                span: span.clone(),
            }),
            TopLevel::Export(path) => {
                if let Some(head) = path_names(path.segments()).first() {
                    self.reexports.insert(head.clone());
                }
                walk_top_level(self, node);
            }
            _ => walk_top_level(self, node),
        }
    }
}

impl<'a> ScopeCollector<'a, '_> {
    /// Only the head of `a::B` binds through an import; later segments are
    /// members of that binding. Their generic arguments are still type uses.
    fn visit_path_head(&mut self, path: &'a [IdentOrType]) {
        for (index, segment) in path.iter().enumerate() {
            match segment {
                _ if index == 0 => crate::ast::visit::walk_ident_or_type(self, segment),
                IdentOrType::Type(ast::ParseType::Type(inner)) => {
                    walk_list!(self, visit_parse_type, &inner.generics);
                }
                _ => {}
            }
        }
    }
}

fn path_span(segments: &[IdentOrType]) -> Option<Span> {
    let segment_span = |segment: &IdentOrType| match segment {
        IdentOrType::Ident(ident) => ident.span.clone(),
        IdentOrType::Type(ty) => ty.span(),
    };
    let first = segment_span(segments.first()?);
    let last = segment_span(segments.last()?);
    Some(Span::new(first.file_path, first.start, last.end))
}

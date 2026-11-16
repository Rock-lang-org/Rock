//! `unused_imports`: `>` imports not used in the importing module or its
//! submodules.
//!
//! Value imports use the semantic references recorded during lowering, paired
//! with the written path head: locals, fields and fully qualified references
//! do not use a same-named import. Type/variant imports still use conservative
//! name occurrences. Macro imports use expansion's selected-binding records.
//! Glob provenance remains follow-up work; the injected std prelude is not an import.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use crate::ast::visit::{
    walk_expression, walk_impl, walk_module, walk_parse_type_inner, walk_top_level,
    walk_trait_decl, Visitor,
};
use crate::walk_list;
use crate::ast::{self, IdentOrType, ParseTypeInner, TopLevel};
use crate::collect::path_names;
use crate::diagnostic::Diagnostic;
use crate::ids::DefId;
use crate::infer::ResolvedHirProgram;
use crate::lexer::Span;
use crate::source_map::SourceSymbol;

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
    let mut imported_values = HashSet::new();
    let mut has_imports = collect_value_import_targets(
        modules.root, "", hir, modules, &mut imported_values,
    );
    for (path, module) in &modules.files {
        has_imports |= collect_value_import_targets(
            module, path, hir, modules, &mut imported_values,
        );
    }
    if !has_imports {
        return;
    }
    // Index only references to imported callables, not every callable in the
    // crate. Type-only imports need no value-reference index at all.
    let mut references: HashMap<_, Vec<_>> = HashMap::new();
    if !imported_values.is_empty() {
        for reference in hir.source_map.references() {
            if let SourceSymbol::Definition(id) = reference.target {
                if imported_values.contains(&id) {
                    references.entry((reference.span.file_path.clone(), reference.span.start))
                        .or_default().push((reference.span.end, id));
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
            import.value_target = value_import_target(hir, modules, &scope.path, &import.written);
        }
    }
    let mut value_bindings: HashMap<_, Vec<_>> = HashMap::new();
    for (scope_index, scope) in scopes.iter().enumerate() {
        for (import_index, import) in scope.imports.iter().enumerate() {
            if let Some(id) = import.value_target {
                value_bindings.entry((scope.path.as_str(), import.name.as_str()))
                    .or_default().push((id, scope_index, import_index));
            }
        }
    }
    let mut used_values = HashSet::new();
    for scope in &scopes {
        let uses = scope.values.iter().map(|(name, id)| (name, Some(*id)))
            .chain(scope.reexports.iter().map(|name| (name, None)));
        for (name, id) in uses {
            let mut path = scope.path.as_str();
            loop {
                if let Some(bindings) = value_bindings.get(&(path, name.as_str())) {
                    // A nearer import owns this name even if its target differs.
                    if let Some((_, scope, import)) = bindings.iter().rev()
                        .find(|(target, _, _)| id.is_none_or(|id| *target == id))
                    {
                        used_values.insert((*scope, *import));
                    }
                    break;
                }
                if path.is_empty() {
                    break;
                }
                path = path.rsplit_once("::").map_or("", |(parent, _)| parent);
            }
        }
    }

    for (scope_index, scope) in scopes.iter().enumerate() {
        for (import_index, import) in scope.imports.iter().enumerate() {
            let used = if import.value_target.is_some() {
                used_values.contains(&(scope_index, import_index))
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

fn collect_value_import_targets(
    module: &ast::Module,
    scope: &str,
    hir: &ResolvedHirProgram,
    modules: &SourceModules<'_>,
    targets: &mut HashSet<DefId>,
) -> bool {
    let mut has_imports = false;
    for item in &module.top_levels {
        match item {
            TopLevel::Import(path) => {
                has_imports = true;
                let written = path_names(path.segments()).join("::");
                if let Some(id) = value_import_target(hir, modules, scope, &written) {
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
                has_imports |= collect_value_import_targets(
                    &child.0, path.as_deref().unwrap_or(scope), hir, modules, targets,
                );
            }
            _ => {}
        }
    }
    has_imports
}

fn value_import_target(
    hir: &ResolvedHirProgram,
    modules: &SourceModules<'_>,
    scope: &str,
    written: &str,
) -> Option<DefId> {
    let resolver = &hir.resolver;
    let relative = if scope.is_empty() {
        written.to_string()
    } else {
        format!("{scope}::{written}")
    };
    let qualified = modules.current_crate_name.map(|name| format!("{name}::{relative}"));
    let root_qualified = modules.current_crate_name.map(|name| format!("{name}::{written}"));
    let id = resolver.resolve_item_or_alias(written)
        .or_else(|| resolver.resolve_item_or_alias(&relative))
        .or_else(|| qualified.as_deref().and_then(|name| resolver.resolve_item_or_alias(name)))
        .or_else(|| root_qualified.as_deref().and_then(|name| resolver.resolve_item_or_alias(name)))?;
    (hir.program.functions.contains_key(&id) || hir.program.externs.contains_key(&id))
        .then_some(id)
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
    value_target: Option<DefId>,
}

/// One module body: its imports and every other name written in it, nested
/// named modules excluded (they are scopes of their own).
struct ModuleScope {
    path: String,
    imports: Vec<ImportSite>,
    names: HashSet<String>,
    values: Vec<(String, DefId)>,
    reexports: HashSet<String>,
}

type ValueReferences = HashMap<(PathBuf, usize), Vec<(usize, DefId)>>;

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
    };
    walk_module(&mut collector, module);
    let ScopeCollector {
        names,
        imports,
        children,
        values,
        reexports,
        ..
    } = collector;
    scopes.push(ModuleScope {
        path: path.clone(),
        imports,
        names,
        values,
        reexports,
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
}

impl<'a> Visitor<'a> for ScopeCollector<'a, '_> {
    fn visit_name(&mut self, name: &str) {
        self.names.insert(name.to_string());
    }

    fn visit_identifier_path(&mut self, node: &'a ast::IdentifierPath) {
        if let Some(span) = path_span(&node.path) {
            if let Some(targets) = self.references.get(&(span.file_path, span.start)) {
                if let Some(head) = path_names(&node.path).first() {
                    for (_, id) in targets.iter().filter(|(end, _)| *end <= span.end) {
                        self.values.push((head.clone(), *id));
                    }
                }
            }
        }
        crate::ast::visit::walk_identifier_path(self, node);
    }

    fn visit_parse_type_inner(&mut self, node: &'a ParseTypeInner) {
        self.names
            .extend(node.name.split("::").map(str::to_string));
        walk_parse_type_inner(self, node);
    }

    // The shared walker skips these type positions; names written there still
    // use imports.
    fn visit_trait_decl(&mut self, node: &'a ast::TraitDecl) {
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
                        value_target: None,
                    });
                }
            }
            TopLevel::GlobImport(_) => {}
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

fn path_span(segments: &[IdentOrType]) -> Option<Span> {
    let segment_span = |segment: &IdentOrType| match segment {
        IdentOrType::Ident(ident) => ident.span.clone(),
        IdentOrType::Type(ty) => ty.span(),
    };
    let first = segment_span(segments.first()?);
    let last = segment_span(segments.last()?);
    Some(Span::new(first.file_path, first.start, last.end))
}

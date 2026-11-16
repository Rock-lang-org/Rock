//! `unused_imports`: `>` imports whose name never appears in the importing
//! module or its submodules.
//!
//! Works on the macro-expanded AST: an import is used when its short name is
//! written anywhere in its scope outside import statements (as a value, type,
//! path segment, pattern or re-export). This never reports an import that
//! resolution goes through; a same-named local or field may hide an unused
//! import (false negative only). Glob imports and macro imports are not
//! checked yet; the injected std prelude is not an import.

use std::collections::{BTreeMap, HashSet};

use crate::ast::visit::{
    walk_expression, walk_impl, walk_module, walk_parse_type_inner, walk_top_level,
    walk_trait_decl, Visitor,
};
use crate::walk_list;
use crate::ast::{self, IdentOrType, ParseTypeInner, TopLevel};
use crate::collect::path_names;
use crate::diagnostic::Diagnostic;
use crate::lexer::Span;

use super::{Lint, LintSink};

/// The current crate's modules after macro expansion: the root module and
/// every file module by crate-relative path (`a::b`).
pub struct SourceModules<'a> {
    pub root: &'a ast::Module,
    pub files: BTreeMap<String, &'a ast::Module>,
}

pub(crate) fn check_unused_imports(modules: &SourceModules<'_>, sink: &mut LintSink<'_>) {
    if !sink.is_enabled(Lint::UnusedImports) {
        return;
    }
    let mut scopes = Vec::new();
    collect_scopes(String::new(), modules.root, &mut scopes);
    for (path, module) in &modules.files {
        collect_scopes(path.clone(), module, &mut scopes);
    }

    for scope in &scopes {
        for import in &scope.imports {
            let used = scopes
                .iter()
                .filter(|other| is_within(&other.path, &scope.path))
                .any(|other| other.names.contains(&import.name));
            if !used {
                sink.emit(
                    Lint::UnusedImports,
                    Diagnostic::new(format!("unused import: `{}`", import.written), import.span.clone())
                        .with_help("remove the unused import"),
                );
            }
        }
    }
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
}

/// One module body: its imports and every other name written in it, nested
/// named modules excluded (they are scopes of their own).
struct ModuleScope {
    path: String,
    imports: Vec<ImportSite>,
    names: HashSet<String>,
}

fn collect_scopes(path: String, module: &ast::Module, scopes: &mut Vec<ModuleScope>) {
    let mut collector = ScopeCollector {
        names: HashSet::new(),
        imports: Vec::new(),
        children: Vec::new(),
    };
    walk_module(&mut collector, module);
    let ScopeCollector {
        names,
        imports,
        children,
    } = collector;
    scopes.push(ModuleScope {
        path: path.clone(),
        imports,
        names,
    });
    for (name, child) in children {
        let child_path = if path.is_empty() {
            name
        } else {
            format!("{path}::{name}")
        };
        collect_scopes(child_path, child, scopes);
    }
}

struct ScopeCollector<'a> {
    names: HashSet<String>,
    imports: Vec<ImportSite>,
    children: Vec<(String, &'a ast::Module)>,
}

impl<'a> Visitor<'a> for ScopeCollector<'a> {
    fn visit_name(&mut self, name: &str) {
        self.names.insert(name.to_string());
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
                    });
                }
            }
            TopLevel::GlobImport(_) => {}
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

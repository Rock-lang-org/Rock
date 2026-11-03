use super::type_names::path_names;
use crate::ast;
use std::collections::HashMap;

pub(crate) fn collect_exports(module: &ast::Module) -> HashMap<String, Option<String>> {
    let mut exports = HashMap::new();
    for top_level in &module.top_levels {
        match top_level {
            ast::TopLevel::Export(path) => {
                let names = match path {
                    ast::Path::Ident(ip) => path_names(&ip.path),
                    ast::Path::Type(tp) => path_names(&tp.path),
                };
                if let Some(short_name) = names.last().cloned() {
                    let source = if names.len() > 1 {
                        Some(names.join("::"))
                    } else {
                        None
                    };
                    exports.insert(short_name, source);
                }
            }
            ast::TopLevel::FunctionDecl(fd) if fd.exported => {
                exports.insert(fd.name.name.clone(), None);
            }
            ast::TopLevel::Extern(sig) if sig.exported => {
                exports.insert(sig.name.name.clone(), None);
            }
            ast::TopLevel::StructDecl(sd) if sd.exported => {
                exports.insert(sd.name.name.clone(), None);
            }
            ast::TopLevel::EnumDecl(ed) if ed.exported => {
                exports.insert(ed.name.name.clone(), None);
            }
            ast::TopLevel::TraitDecl(td) if td.exported => {
                exports.insert(td.name.name.clone(), None);
            }
            ast::TopLevel::Mod(ident, true) => {
                exports.insert(ident.name.clone(), None);
            }
            ast::TopLevel::GlobExport(module_path) => {
                let sentinel = format!("{}::*", module_path.join("::"));
                exports.insert(sentinel.clone(), Some(sentinel));
            }
            _ => {}
        }
    }
    exports
}

pub(super) fn find_inline_module<'a>(
    module: &'a ast::Module,
    name: &str,
) -> Option<&'a ast::Module> {
    module
        .top_levels
        .iter()
        .find_map(|top_level| match top_level {
            ast::TopLevel::Module(module_decl)
                if module_decl.0.name.as_ref().map(|ident| ident.name.as_str()) == Some(name) =>
            {
                Some(&module_decl.0)
            }
            _ => None,
        })
}

pub(crate) fn export_references_module(
    exports: &HashMap<String, Option<String>>,
    module_name: &str,
    qualified_module_name: &str,
) -> bool {
    exports.values().any(|source| {
        source
            .as_deref()
            .map(|source| {
                export_reference_tail(source, module_name, qualified_module_name).is_some()
            })
            .unwrap_or(false)
    })
}

fn export_reference_tail<'a>(
    source: &'a str,
    module_name: &str,
    qualified_module_name: &str,
) -> Option<&'a str> {
    [module_name, qualified_module_name]
        .into_iter()
        .find_map(|prefix| {
            if source == prefix {
                Some("")
            } else {
                source
                    .strip_prefix(prefix)
                    .and_then(|tail| tail.strip_prefix("::"))
            }
        })
}

pub(crate) fn child_export_references(
    exports: &HashMap<String, Option<String>>,
    module_name: &str,
    qualified_module_name: &str,
) -> HashMap<String, Option<String>> {
    let mut references = HashMap::new();
    for (export_name, source) in exports {
        let Some(source) = source.as_deref() else {
            continue;
        };
        let Some(tail) = export_reference_tail(source, module_name, qualified_module_name) else {
            continue;
        };
        if tail.is_empty() || tail == "*" {
            continue;
        }

        let key = if tail.ends_with("::*") {
            tail.to_string()
        } else {
            export_name.clone()
        };
        references.insert(key, Some(tail.to_string()));
    }

    references
}

pub(crate) fn merge_export_references(
    exports: &HashMap<String, Option<String>>,
    inherited_references: HashMap<String, Option<String>>,
) -> HashMap<String, Option<String>> {
    let mut references = exports.clone();
    for (key, value) in inherited_references {
        insert_export_reference(&mut references, key, value);
    }
    references
}

fn insert_export_reference(
    references: &mut HashMap<String, Option<String>>,
    key: String,
    value: Option<String>,
) {
    if references.values().any(|existing| existing == &value) {
        return;
    }

    if !references.contains_key(&key) {
        references.insert(key, value);
        return;
    }

    let fallback = value.as_deref().unwrap_or(&key);
    let mut candidate = fallback.to_string();
    let mut ordinal = 0;
    while references.contains_key(&candidate) {
        ordinal += 1;
        candidate = format!("{}#{}", fallback, ordinal);
    }
    references.insert(candidate, value);
}

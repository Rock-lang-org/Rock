//! Path-segment and module export helpers.

use std::collections::HashMap;

use crate::ast;

pub fn seg_name(seg: &ast::IdentOrType) -> Option<String> {
    match seg {
        ast::IdentOrType::Ident(ident) => Some(ident.name.clone()),
        ast::IdentOrType::Type(ty) => Some(ty.type_name()),
    }
}

pub fn path_names(path: &[ast::IdentOrType]) -> Vec<String> {
    path.iter()
        .filter_map(seg_name)
        .flat_map(|name| name.split("::").map(str::to_string).collect::<Vec<_>>())
        .collect()
}

pub fn collect_exports(module: &ast::Module) -> HashMap<String, Option<String>> {
    let mut exports = HashMap::new();
    for top_level in &module.top_levels {
        match top_level {
            ast::TopLevel::Export(path) => {
                // Re-export (multi-segment) or legacy standalone export (single-segment)
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
            // Inline exports via `< name = ...`, `< struct ...`, etc.
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
                // Store a sentinel: key = "module::path::*", source = Some("module::path::*")
                // Callers that have file-loading context will expand this into individual entries.
                let sentinel = format!("{}::*", module_path.join("::"));
                exports.insert(sentinel.clone(), Some(sentinel));
            }
            _ => {}
        }
    }
    exports
}

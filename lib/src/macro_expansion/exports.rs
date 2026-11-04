//! Declarative macros a crate exports to its dependents.

use std::collections::BTreeMap;

use crate::ast::{IdentOrType, Module, ModuleDecl, ParseType, Path, TopLevel};
use crate::products::ProductMacroExport;

/// Collects every `< macro` declaration of `module` and its inline modules.
/// `prefix` is the module's path inside its crate (empty for the crate root).
pub fn collect_exported_macros(module: &Module, prefix: &str, out: &mut Vec<ProductMacroExport>) {
    for top_level in &module.top_levels {
        match top_level {
            TopLevel::MacroDecl(decl) if decl.exported => out.push(ProductMacroExport {
                path: join_path(prefix, &decl.name.name),
                decl: decl.clone(),
            }),
            TopLevel::Module(ModuleDecl(inner)) => {
                if let Some(name) = &inner.name {
                    collect_exported_macros(inner, &join_path(prefix, &name.name), out);
                }
            }
            _ => {}
        }
    }
}

/// Maps each re-export (`< crate::path::name` or `< crate::path::*`) in the
/// prelude module that names an exported macro to its prelude alias.
pub fn prelude_macro_aliases(
    prelude: &Module,
    crate_name: &str,
    exported: &[ProductMacroExport],
) -> BTreeMap<String, String> {
    let crate_relative = |segments: Vec<String>| match segments.split_first() {
        Some((first, rest)) if first == crate_name => rest.join("::"),
        _ => segments.join("::"),
    };
    let mut aliases = BTreeMap::new();

    for top_level in &prelude.top_levels {
        match top_level {
            TopLevel::Export(path) => {
                let path = crate_relative(path_segments(path));
                if exported.iter().any(|export| export.path == path) {
                    let alias = path.rsplit("::").next().unwrap_or(&path).to_string();
                    aliases.insert(alias, path);
                }
            }
            TopLevel::GlobExport(segments) => {
                let module = crate_relative(segments.clone());
                for export in exported {
                    if let Some(alias) = direct_child(&export.path, &module) {
                        aliases.insert(alias.to_string(), export.path.clone());
                    }
                }
            }
            _ => {}
        }
    }

    aliases
}

/// Returns `name` when `path` is `module::name` with no further segments.
fn direct_child<'a>(path: &'a str, module: &str) -> Option<&'a str> {
    let rest = if module.is_empty() {
        path
    } else {
        path.strip_prefix(module)?.strip_prefix("::")?
    };
    (!rest.is_empty() && !rest.contains("::")).then_some(rest)
}

/// Path segments as written. Segments that are not plain names, which can
/// never name a macro or its module, become empty strings.
pub(crate) fn path_segments(path: &Path) -> Vec<String> {
    let segments = path.segments();
    segments
        .iter()
        .map(|segment| match segment {
            IdentOrType::Ident(ident) => ident.name.clone(),
            IdentOrType::Type(ParseType::Type(inner)) => inner.name.clone(),
            IdentOrType::Type(_) => String::new(),
        })
        .collect()
}

fn join_path(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}::{name}")
    }
}

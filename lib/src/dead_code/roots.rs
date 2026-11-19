use std::collections::HashSet;

use crate::ast::{Module, TopLevel};
use crate::ids::DefId;
use crate::infer::ResolvedHirProgram;
use crate::lint::SourceModules;
use crate::{Config, CrateKind};

pub(super) fn roots(
    hir: &ResolvedHirProgram,
    modules: &SourceModules<'_>,
    config: &Config,
) -> Vec<DefId> {
    let mut roots = Vec::new();
    if config.crate_kind == CrateKind::Bin {
        roots.extend(
            hir.program
                .functions
                .values()
                .filter(|function| {
                    function.id.crate_id == hir.root_crate_id
                        && hir.resolver.canonical_name(function.id) == Some("main")
                })
                .map(|function| function.id),
        );
    } else {
        let mut public_modules = HashSet::from([String::new()]);
        collect_public_modules("", modules.root, modules, &mut public_modules);
        let public = |path: &str| {
            let path = config
                .current_crate_name
                .as_deref()
                .and_then(|name| path.strip_prefix(name)?.strip_prefix("::"))
                .unwrap_or(path);
            let parent = path
                .rsplit_once("::")
                .map(|(parent, _)| parent)
                .unwrap_or("");
            public_modules.contains(parent)
        };
        roots.extend(
            hir.resolver
                .export_aliases
                .iter()
                .filter(|(path, _)| public(path))
                .map(|(_, id)| *id),
        );
        roots.extend(
            hir.resolver
                .variant_exports
                .iter()
                .filter(|(path, _)| public(path))
                .map(|(_, variant)| variant.enum_id),
        );
    }
    let lang = &hir.program.language_items;
    macro_rules! root {
        ($bundle:ident, $($field:ident),+) => {
            if let Some(items) = &lang.$bundle {
                $(roots.push(items.$field);)+
            }
        };
    }
    root!(sized, trait_id);
    root!(drop, trait_id, method_id);
    root!(fold, trait_id, method_id);
    root!(index, trait_id, method_id);
    root!(index_mut, trait_id, method_id);
    root!(fn_once, trait_id, method_id);
    root!(fn_mut, trait_id, method_id);
    root!(fn_trait, trait_id, method_id);
    root!(send, trait_id);
    root!(owned, trait_id);
    root!(sync, trait_id);
    root!(
        try_protocol,
        try_trait_id,
        branch_method_id,
        from_residual_trait_id,
        from_residual_method_id,
        control_flow_enum_id
    );
    root!(range, enum_id);
    roots
}

fn collect_public_modules(
    prefix: &str,
    module: &Module,
    modules: &SourceModules<'_>,
    public: &mut HashSet<String>,
) {
    for item in &module.top_levels {
        let name = match item {
            TopLevel::Mod(name, true) => name.name.clone(),
            TopLevel::Export(path) => crate::collect::path_names(path.segments()).join("::"),
            TopLevel::GlobExport(path) => path.join("::"),
            _ => continue,
        };
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}::{name}")
        };
        if modules.files.contains_key(&path) {
            if public.insert(path.clone()) {
                if let Some(child) = modules.files.get(&path) {
                    collect_public_modules(&path, child, modules, public);
                }
            }
        }
    }
}

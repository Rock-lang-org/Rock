use super::{CollectedIdEnvironment, CollectedTraitMemberIds};
use crate::ast;
use crate::collect::item_index::{IndexingIds, ItemIndex, ItemKind};
use std::collections::HashMap;

#[derive(Debug)]
struct TraitMemberSource {
    trait_id: crate::ids::DefId,
    trait_path: String,
    method_names: Vec<String>,
    signature_names: Vec<String>,
}

#[derive(Debug)]
struct ImplMemberSource {
    impl_id: crate::ids::DefId,
    method_names: Vec<String>,
}

pub(super) fn collected_id_environment_from_index(
    item_index: &ItemIndex,
    root_module_id: crate::ids::ModuleId,
) -> CollectedIdEnvironment {
    let current_source_items = item_index
        .items()
        .iter()
        .map(|item| (item.source, item.clone()))
        .collect();
    let mut current_source_module_paths = HashMap::from([(Vec::new(), root_module_id)]);
    while current_source_module_paths.len() < item_index.modules().len() {
        let mut inserted = false;
        for module in item_index.modules() {
            let (Some(parent), Some(name)) = (module.parent, module.name.as_ref()) else {
                continue;
            };
            let Some(parent_path) = current_source_module_paths
                .iter()
                .find_map(|(path, module_id)| (*module_id == parent).then(|| path.clone()))
            else {
                continue;
            };
            let mut path = parent_path;
            path.push(name.clone());
            inserted |= current_source_module_paths
                .insert(path, module.module_id)
                .is_none();
        }
        if !inserted {
            break;
        }
    }

    CollectedIdEnvironment {
        current_source_items,
        current_source_module_paths,
        trait_member_ids_by_owner: HashMap::new(),
        impl_method_ids_by_owner: HashMap::new(),
    }
}

pub(super) fn populate_member_id_environment(
    env: &mut CollectedIdEnvironment,
    ids: &mut IndexingIds,
    item_index: &ItemIndex,
    module: &ast::Module,
    current_crate_name: Option<&str>,
    source_modules: &crate::collect::item_index::SourceModuleMap,
) {
    let root_module_id = item_index
        .module_id_by_path(&[])
        .expect("indexed collection must have a root module");
    let mut module_path = current_crate_name
        .map(|name| vec![name.to_string()])
        .unwrap_or_default();
    let mut trait_sources = Vec::new();
    let mut impl_sources = Vec::new();

    collect_member_sources_in_module(
        module,
        root_module_id,
        &mut module_path,
        source_modules,
        item_index,
        &mut trait_sources,
        &mut impl_sources,
    );

    trait_sources.sort_by(|left, right| {
        left.trait_path
            .cmp(&right.trait_path)
            .then(left.trait_id.cmp(&right.trait_id))
    });

    for source in trait_sources {
        let mut member_ids = CollectedTraitMemberIds::default();
        let mut shared_member_ids = HashMap::new();

        for method_name in sorted_names(source.method_names) {
            let id = *shared_member_ids
                .entry(method_name.clone())
                .or_insert_with(|| ids.fresh_def_id());
            member_ids.methods.insert(method_name, id);
        }

        for signature_name in sorted_names(source.signature_names) {
            let id = *shared_member_ids
                .entry(signature_name.clone())
                .or_insert_with(|| ids.fresh_def_id());
            member_ids.signatures.insert(signature_name, id);
        }

        env.trait_member_ids_by_owner
            .insert(source.trait_id, member_ids);
    }

    for source in impl_sources {
        let mut method_ids = HashMap::new();
        for method_name in sorted_names(source.method_names) {
            method_ids.insert(method_name, ids.fresh_def_id());
        }
        env.impl_method_ids_by_owner
            .insert(source.impl_id, method_ids);
    }
}

fn collect_member_sources_in_module(
    module: &ast::Module,
    module_id: crate::ids::ModuleId,
    module_path: &mut Vec<String>,
    source_modules: &crate::collect::item_index::SourceModuleMap,
    item_index: &ItemIndex,
    trait_sources: &mut Vec<TraitMemberSource>,
    impl_sources: &mut Vec<ImplMemberSource>,
) {
    for (top_level_index, top_level) in module.top_levels.iter().enumerate() {
        match top_level {
            ast::TopLevel::Module(module_decl) => {
                let inner = &module_decl.0;
                if let Some(module_name) = &inner.name {
                    let Some(child_module_id) =
                        item_index.child_module_id(module_id, &module_name.name)
                    else {
                        continue;
                    };
                    module_path.push(module_name.name.clone());
                    collect_member_sources_in_module(
                        inner,
                        child_module_id,
                        module_path,
                        source_modules,
                        item_index,
                        trait_sources,
                        impl_sources,
                    );
                    module_path.pop();
                }
            }
            ast::TopLevel::Mod(name, _) => {
                let Some(child_module_id) = item_index.child_module_id(module_id, &name.name)
                else {
                    continue;
                };
                module_path.push(name.name.clone());
                if let Some(source_module) = source_modules.get(&*module_path) {
                    collect_member_sources_in_module(
                        source_module,
                        child_module_id,
                        module_path,
                        source_modules,
                        item_index,
                        trait_sources,
                        impl_sources,
                    );
                }
                module_path.pop();
            }
            ast::TopLevel::TraitDecl(trait_decl) => {
                if let Some(owner) = item_index
                    .item_at_source(module_id, top_level_index)
                    .filter(|item| item.kind == ItemKind::Trait)
                {
                    trait_sources.push(TraitMemberSource {
                        trait_id: owner.def_id,
                        trait_path: qualified_member_owner_path(module_path, &trait_decl.name.name),
                        method_names: trait_decl
                            .methods
                            .keys()
                            .map(|ident| ident.name.clone())
                            .collect(),
                        signature_names: trait_decl
                            .signatures
                            .keys()
                            .map(|ident| ident.name.clone())
                            .collect(),
                    });
                }
            }
            ast::TopLevel::Impl(impl_decl) => {
                if let Some(owner) = item_index
                    .item_at_source(module_id, top_level_index)
                    .filter(|item| item.kind == ItemKind::Impl)
                {
                    impl_sources.push(ImplMemberSource {
                        impl_id: owner.def_id,
                        method_names: impl_decl
                            .methods
                            .keys()
                            .map(|ident| ident.name.clone())
                            .collect(),
                    });
                }
            }
            _ => {}
        }
    }
}

fn qualified_member_owner_path(module_path: &[String], name: &str) -> String {
    if module_path.is_empty() {
        name.to_string()
    } else {
        format!("{}::{}", module_path.join("::"), name)
    }
}

fn sorted_names(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names.dedup();
    names
}

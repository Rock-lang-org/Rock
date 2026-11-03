use super::LocalCollector;
use crate::ast;
use crate::collect::context::{
    child_export_references, collect_exports, export_references_module, merge_export_references,
};
use crate::hir::{HirAssociatedTypeDecl, HirTrait};
use crate::ids::{AssocTypeId, DefId, ModuleId};
use std::collections::HashMap;

impl LocalCollector {
    pub(super) fn predeclare_source_backed_trait_identities(
        &mut self,
        module: &ast::Module,
        module_id: ModuleId,
        crate_name: &str,
        root_exports: &HashMap<String, Option<String>>,
        is_std: bool,
    ) {
        for (top_level_index, top_level) in module.top_levels.iter().enumerate() {
            match top_level {
                ast::TopLevel::TraitDecl(td) => {
                    let qualified_name = format!("{}::{}", crate_name, td.name.name);
                    let Some(id) = self.item_id_at_source(
                        module_id,
                        top_level_index,
                        top_level,
                        &td.name.span,
                    ) else {
                        continue;
                    };
                    self.predeclare_trait_identity(td, id, &qualified_name, root_exports);
                }
                ast::TopLevel::Module(module_decl) => {
                    let inner_module = &module_decl.0;
                    if let Some(ref mod_name) = inner_module.name {
                        let Some(child_module_id) =
                            self.child_module_id(module_id, mod_name, &mod_name.span)
                        else {
                            continue;
                        };
                        let qualified_prefix = format!("{}::{}", crate_name, mod_name.name);
                        if !root_exports.contains_key(&mod_name.name)
                            && !export_references_module(
                                root_exports,
                                &mod_name.name,
                                &qualified_prefix,
                            )
                        {
                            continue;
                        }

                        let inner_exports = self.context.expand_glob_exports_with_prefix(
                            &qualified_prefix,
                            collect_exports(inner_module),
                        );
                        let inherited_references = child_export_references(
                            root_exports,
                            &mod_name.name,
                            &qualified_prefix,
                        );
                        let reference_exports =
                            merge_export_references(&inner_exports, inherited_references);
                        self.predeclare_source_backed_qualified_trait_identities(
                            inner_module,
                            child_module_id,
                            &qualified_prefix,
                            &inner_exports,
                            &reference_exports,
                        );
                    }
                }
                ast::TopLevel::Mod(ident, _) => {
                    let Some(child_module_id) = self.child_module_id(module_id, ident, &ident.span)
                    else {
                        continue;
                    };
                    let is_prelude = is_std && ident.name == "prelude";
                    let qualified_prefix = format!("{}::{}", crate_name, ident.name);
                    if !root_exports.contains_key(&ident.name)
                        && !export_references_module(root_exports, &ident.name, &qualified_prefix)
                        && !is_prelude
                    {
                        continue;
                    }

                    self.predeclare_source_backed_mod_trait_identities(
                        ident,
                        child_module_id,
                        Some(crate_name),
                        Some(root_exports),
                    );
                }
                _ => {}
            }
        }
    }

    pub(super) fn predeclare_source_backed_mod_trait_identities(
        &mut self,
        ident: &ast::Ident,
        module_id: ModuleId,
        prefix: Option<&str>,
        parent_reference_exports: Option<&HashMap<String, Option<String>>>,
    ) {
        let module_name = &ident.name;
        let qualified_module_name = prefix
            .map(|prefix| format!("{}::{}", prefix, module_name))
            .unwrap_or_else(|| module_name.clone());

        let loaded_module = match self.context.load_module_with_prefix(module_name, prefix) {
            Ok(module) => module,
            Err(err) => {
                self.context.push_error_with_span(err, ident.span.clone());
                return;
            }
        };

        let Some(file_path) = self
            .context
            .loaded_module_path_for_prefix(module_name, prefix)
        else {
            self.context.push_error_with_span(
                format!(
                    "Module '{}' was not loaded by the source database",
                    qualified_module_name
                ),
                ident.span.clone(),
            );
            return;
        };

        let old_path = self.context.current_module_path.clone();
        self.context.current_module_path = file_path;

        let exports = self
            .context
            .expand_glob_exports(collect_exports(&loaded_module));
        let reference_exports = parent_reference_exports
            .map(|parent_reference_exports| {
                let inherited_references = child_export_references(
                    parent_reference_exports,
                    module_name,
                    &qualified_module_name,
                );
                merge_export_references(&exports, inherited_references)
            })
            .unwrap_or_else(|| exports.clone());
        self.predeclare_source_backed_qualified_trait_identities(
            &loaded_module,
            module_id,
            &qualified_module_name,
            &exports,
            &reference_exports,
        );

        self.context.current_module_path = old_path;
    }

    pub(super) fn predeclare_source_backed_qualified_trait_identities(
        &mut self,
        module: &ast::Module,
        module_id: ModuleId,
        prefix: &str,
        exports: &HashMap<String, Option<String>>,
        reference_exports: &HashMap<String, Option<String>>,
    ) {
        for (top_level_index, top_level) in module.top_levels.iter().enumerate() {
            match top_level {
                ast::TopLevel::TraitDecl(td) => {
                    let qualified_name = format!("{}::{}", prefix, td.name.name);
                    let Some(id) = self.item_id_at_source(
                        module_id,
                        top_level_index,
                        top_level,
                        &td.name.span,
                    ) else {
                        continue;
                    };
                    self.predeclare_trait_identity(td, id, &qualified_name, exports);
                }
                ast::TopLevel::Module(module_decl) => {
                    let inner = &module_decl.0;
                    if let Some(ref mod_name) = inner.name {
                        let Some(child_module_id) =
                            self.child_module_id(module_id, mod_name, &mod_name.span)
                        else {
                            continue;
                        };
                        let new_prefix = format!("{}::{}", prefix, mod_name.name);
                        if !exports.contains_key(&mod_name.name)
                            && !export_references_module(
                                reference_exports,
                                &mod_name.name,
                                &new_prefix,
                            )
                        {
                            continue;
                        }

                        let inner_exports = self
                            .context
                            .expand_glob_exports_with_prefix(&new_prefix, collect_exports(inner));
                        let inherited_references =
                            child_export_references(reference_exports, &mod_name.name, &new_prefix);
                        let next_reference_exports =
                            merge_export_references(&inner_exports, inherited_references);
                        self.predeclare_source_backed_qualified_trait_identities(
                            inner,
                            child_module_id,
                            &new_prefix,
                            &inner_exports,
                            &next_reference_exports,
                        );
                    }
                }
                ast::TopLevel::Mod(ident, _) => {
                    let Some(child_module_id) = self.child_module_id(module_id, ident, &ident.span)
                    else {
                        continue;
                    };
                    let new_prefix = format!("{}::{}", prefix, ident.name);
                    if !exports.contains_key(&ident.name)
                        && !export_references_module(reference_exports, &ident.name, &new_prefix)
                    {
                        continue;
                    }

                    self.predeclare_source_backed_mod_trait_identities(
                        ident,
                        child_module_id,
                        Some(prefix),
                        Some(reference_exports),
                    );
                }
                _ => {}
            }
        }
    }

    pub(super) fn predeclare_trait_identity(
        &mut self,
        td: &ast::TraitDecl,
        id: DefId,
        qualified_name: &str,
        _exports: &HashMap<String, Option<String>>,
    ) {
        if self.context.traits.contains_key(qualified_name) {
            return;
        }

        let generic_params =
            crate::type_lowering::lower_generic_param_decls(id, &td.generic_params);
        let associated_types = td
            .associated_types
            .iter()
            .enumerate()
            .map(|(index, assoc)| HirAssociatedTypeDecl {
                id: AssocTypeId(index as u32),
                name: assoc.name.name.clone(),
                kind: crate::type_lowering::lower_associated_type_kind(assoc.kind.as_ref()),
            })
            .collect();
        let trait_def = HirTrait {
            id,
            name: td.name.name.clone(),
            generic_params,
            target: td.for_.as_ref().map(|target| {
                crate::types::GenericParamDecl::new(
                    crate::types::GenericParamId {
                        owner: id,
                        index: td.generic_params.len() as u32,
                    },
                    target.name.name.clone(),
                    crate::type_lowering::lower_generic_param_kind(target.kind.as_ref()),
                )
            }),
            predicates: Vec::new(),
            associated_types,
            methods: HashMap::new(),
            signatures: HashMap::new(),
        };

        self.context
            .traits
            .insert(qualified_name.to_string(), trait_def);
    }
}

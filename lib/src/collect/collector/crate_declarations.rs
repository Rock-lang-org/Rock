use super::ordering::ordered_headers;
use super::LocalCollector;
use crate::ast;
use crate::collect::context::{
    child_export_references, collect_exports, export_references_module, merge_export_references,
};
use crate::collect::headers;
use crate::hir::{HirEnum, HirStruct, HirTypeAlias};
use crate::ids::ModuleId;
use crate::types::Type;

impl LocalCollector {
    pub(crate) fn collect_crate_declarations(
        &mut self,
        module: &ast::Module,
        root_module_id: ModuleId,
        crate_name: &str,
        is_std: bool,
    ) {
        let root_exports = collect_exports(module);
        self.predeclare_export_aliases(module, Some(crate_name), true);

        self.predeclare_source_backed_trait_identities(
            module,
            root_module_id,
            crate_name,
            &root_exports,
            is_std,
        );

        for (top_level_index, top_level) in module.top_levels.iter().enumerate() {
            let ast::TopLevel::NewType(name, params, _) = top_level else {
                continue;
            };
            let Some(id) =
                self.item_id_at_source(root_module_id, top_level_index, top_level, &name.span)
            else {
                continue;
            };
            let alias = HirTypeAlias {
                id,
                name: name.name.clone(),
                generic_params: crate::type_lowering::lower_generic_param_decls(id, params),
                ty: Type::Error,
            };
            self.context
                .type_aliases
                .insert(name.name.clone(), alias.clone());
            self.context
                .type_aliases
                .insert(format!("{}::{}", crate_name, name.name), alias);
        }

        for (top_level_index, top_level) in ordered_headers(module) {
            match top_level {
                ast::TopLevel::FunctionSig(sig) => {
                    let qualified_name = format!("{}::{}", crate_name, sig.name.name);
                    let Some(id) = self.item_id_at_source(
                        root_module_id,
                        top_level_index,
                        top_level,
                        &sig.name.span,
                    ) else {
                        continue;
                    };
                    self.collect_function_signature(sig, qualified_name, id);
                }
                ast::TopLevel::FunctionDecl(fd) => {
                    let qualified_name = format!("{}::{}", crate_name, fd.name.name);
                    let Some(id) = self.item_id_at_source(
                        root_module_id,
                        top_level_index,
                        top_level,
                        &fd.name.span,
                    ) else {
                        continue;
                    };
                    self.collect_function_header(fd, qualified_name, id);
                }
                ast::TopLevel::Extern(sig) => {
                    let qualified_name = format!("{}::{}", crate_name, sig.name.name);
                    let Some(id) = self.item_id_at_source(
                        root_module_id,
                        top_level_index,
                        top_level,
                        &sig.name.span,
                    ) else {
                        continue;
                    };
                    let hir_extern =
                        headers::build_extern_with_id(&mut self.context, sig, qualified_name, id);
                    self.context.externs.push(hir_extern);
                }
                ast::TopLevel::StructDecl(sd) => {
                    let qualified_name = format!("{}::{}", crate_name, sd.name.name);
                    let Some(id) = self.item_id_at_source(
                        root_module_id,
                        top_level_index,
                        top_level,
                        &sd.name.span,
                    ) else {
                        continue;
                    };
                    let strukt = self.collect_struct_header(sd, id);
                    self.context
                        .structs
                        .insert(sd.name.name.clone(), strukt.clone());
                    self.context.structs.insert(qualified_name, strukt);
                }
                ast::TopLevel::EnumDecl(ed) => {
                    let qualified_name = format!("{}::{}", crate_name, ed.name.name);
                    let Some(id) = self.item_id_at_source(
                        root_module_id,
                        top_level_index,
                        top_level,
                        &ed.name.span,
                    ) else {
                        continue;
                    };
                    let enum_ = self.collect_enum_header(ed, id);
                    self.context
                        .enums
                        .insert(ed.name.name.clone(), enum_.clone());
                    self.context.enums.insert(qualified_name, enum_);
                }
                ast::TopLevel::NewType(name, params, target) => {
                    let qualified_name = format!("{}::{}", crate_name, name.name);
                    let Some(id) = self.item_id_at_source(
                        root_module_id,
                        top_level_index,
                        top_level,
                        &name.span,
                    ) else {
                        continue;
                    };
                    let alias = headers::build_type_alias_with_id(
                        &mut self.context,
                        name,
                        params,
                        target,
                        id,
                    );
                    self.context
                        .type_aliases
                        .insert(name.name.clone(), alias.clone());
                    self.context.type_aliases.insert(qualified_name, alias);
                }
                ast::TopLevel::TraitDecl(td) => {
                    let qualified_name = format!("{}::{}", crate_name, td.name.name);
                    let Some(id) = self.item_id_at_source(
                        root_module_id,
                        top_level_index,
                        top_level,
                        &td.name.span,
                    ) else {
                        continue;
                    };
                    let Some(member_ids) = self.trait_member_ids_for(id, &qualified_name, td)
                    else {
                        continue;
                    };
                    let hir_trait =
                        headers::build_trait_with_id(&mut self.context, td, id, &member_ids);
                    self.context
                        .traits
                        .insert(td.name.name.clone(), hir_trait.clone());
                    self.context.traits.insert(qualified_name, hir_trait);
                }
                ast::TopLevel::Impl(imp) => {
                    let Some(id) = self.item_id_at_source(
                        root_module_id,
                        top_level_index,
                        top_level,
                        &imp.name.span(),
                    ) else {
                        continue;
                    };
                    let Some(method_ids) = self.impl_method_ids_for(id, crate_name, imp) else {
                        continue;
                    };
                    let hir_impl =
                        headers::build_impl_with_id(&mut self.context, imp, id, &method_ids);
                    self.context.impls.push(hir_impl);
                }
                ast::TopLevel::Module(module_decl) => {
                    let inner_module = &module_decl.0;
                    if let Some(ref mod_name) = inner_module.name {
                        let Some(child_module_id) =
                            self.child_module_id(root_module_id, mod_name, &mod_name.span)
                        else {
                            continue;
                        };
                        let qualified_prefix = format!("{}::{}", crate_name, mod_name.name);
                        if !root_exports.contains_key(&mod_name.name)
                            && !export_references_module(
                                &root_exports,
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
                            &root_exports,
                            &mod_name.name,
                            &qualified_prefix,
                        );
                        let reference_exports =
                            merge_export_references(&inner_exports, inherited_references);
                        self.collect_source_backed_qualified_declarations(
                            inner_module,
                            child_module_id,
                            &qualified_prefix,
                            &inner_exports,
                            &reference_exports,
                        );
                    }
                }
                ast::TopLevel::Mod(ident, _) => {
                    let is_prelude = is_std && ident.name == "prelude";
                    let qualified_prefix = format!("{}::{}", crate_name, ident.name);
                    if !root_exports.contains_key(&ident.name)
                        && !export_references_module(&root_exports, &ident.name, &qualified_prefix)
                        && !is_prelude
                    {
                        continue;
                    }

                    let Some(child_module_id) =
                        self.child_module_id(root_module_id, ident, &ident.span)
                    else {
                        continue;
                    };
                    self.handle_source_backed_mod_decl_in_context(
                        ident,
                        child_module_id,
                        crate_name,
                        &root_exports,
                    );
                }
                ast::TopLevel::InfixOperator(precedence, name) => {
                    self.context
                        .infix_precedence
                        .insert(name.clone(), *precedence);
                }
                _ => {}
            }
        }
    }

    pub(super) fn predeclare_nominal_identities(
        &mut self,
        module: &ast::Module,
        module_id: ModuleId,
        prefix: Option<&str>,
    ) {
        // Signatures need constructor identities and kinds, not completed fields.
        // Seed every local module before lowering headers, including private children.
        for (top_level_index, top_level) in module.top_levels.iter().enumerate() {
            match top_level {
                ast::TopLevel::StructDecl(sd) => {
                    let Some(id) = self.item_id_at_source(
                        module_id,
                        top_level_index,
                        top_level,
                        &sd.name.span,
                    ) else {
                        continue;
                    };
                    let name = prefix
                        .map(|prefix| format!("{}::{}", prefix, sd.name.name))
                        .unwrap_or_else(|| sd.name.name.clone());
                    self.context.structs.insert(
                        name,
                        HirStruct {
                            id,
                            name: sd.name.name.clone(),
                            generic_params: crate::type_lowering::lower_generic_param_decls(
                                id,
                                &sd.generic_params,
                            ),
                            fields: Vec::new(),
                        },
                    );
                }
                ast::TopLevel::EnumDecl(ed) => {
                    let Some(id) = self.item_id_at_source(
                        module_id,
                        top_level_index,
                        top_level,
                        &ed.name.span,
                    ) else {
                        continue;
                    };
                    let name = prefix
                        .map(|prefix| format!("{}::{}", prefix, ed.name.name))
                        .unwrap_or_else(|| ed.name.name.clone());
                    self.context.enums.insert(
                        name,
                        HirEnum {
                            id,
                            name: ed.name.name.clone(),
                            generic_params: crate::type_lowering::lower_generic_param_decls(
                                id,
                                &ed.generic_params,
                            ),
                            variants: Vec::new(),
                        },
                    );
                }
                ast::TopLevel::Module(module_decl) => {
                    let inner = &module_decl.0;
                    if let Some(name) = &inner.name {
                        let Some(child_id) = self.child_module_id(module_id, name, &name.span)
                        else {
                            continue;
                        };
                        let child_prefix = prefix
                            .map(|prefix| format!("{}::{}", prefix, name.name))
                            .unwrap_or_else(|| name.name.clone());
                        self.predeclare_nominal_identities(inner, child_id, Some(&child_prefix));
                    } else {
                        self.predeclare_nominal_identities(inner, module_id, prefix);
                    }
                }
                ast::TopLevel::Mod(ident, _) => {
                    let Some(child_id) = self.child_module_id(module_id, ident, &ident.span) else {
                        continue;
                    };
                    let loading_prefix = prefix
                        .map(str::to_owned)
                        .or_else(|| self.context.current_module_prefix());
                    let loaded = match self
                        .context
                        .load_module_with_prefix(&ident.name, loading_prefix.as_deref())
                    {
                        Ok(module) => module,
                        Err(err) => {
                            self.context.push_error_with_span(err, ident.span.clone());
                            continue;
                        }
                    };
                    let Some(path) = self
                        .context
                        .loaded_module_path_for_prefix(&ident.name, loading_prefix.as_deref())
                    else {
                        self.context.push_error_with_span(
                            format!(
                                "Module '{}' was not loaded by the source database",
                                ident.name
                            ),
                            ident.span.clone(),
                        );
                        continue;
                    };
                    let child_prefix = loading_prefix
                        .as_deref()
                        .map(|prefix| format!("{}::{}", prefix, ident.name))
                        .unwrap_or_else(|| ident.name.clone());
                    let old_path = std::mem::replace(&mut self.context.current_module_path, path);
                    self.predeclare_nominal_identities(&loaded, child_id, Some(&child_prefix));
                    self.context.current_module_path = old_path;
                }
                _ => {}
            }
        }
    }
}

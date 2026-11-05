use super::ordering::ordered_headers;
use super::path_qualification::local_item_lookup_name;
use super::LocalCollector;
use crate::ast;
use crate::collect::context::{child_export_references, collect_exports, merge_export_references};
use crate::collect::headers;
use crate::hir::HirTypeAlias;
use crate::ids::{DefId, ModuleId};
use crate::types::Type;
use std::collections::HashMap;

impl LocalCollector {
    pub(super) fn collect_declarations(&mut self, module: &ast::Module, module_id: ModuleId) {
        let reference_exports = collect_exports(module);
        self.collect_declarations_in_context(module, module_id, None, &reference_exports);
    }

    pub(super) fn collect_declarations_in_context(
        &mut self,
        module: &ast::Module,
        module_id: ModuleId,
        local_prefix: Option<&str>,
        reference_exports: &HashMap<String, Option<String>>,
    ) {
        // Every item is registered by name already, so imports bind before
        // the headers that use them.
        for top_level in &module.top_levels {
            match (top_level, local_prefix) {
                (ast::TopLevel::Import(path), Some(prefix)) => {
                    self.handle_import_in_context(path, prefix)
                }
                (ast::TopLevel::Import(path), None) => self.handle_import(path),
                (ast::TopLevel::GlobImport(module_path), Some(prefix)) => {
                    self.handle_glob_import_in_context(module_path, prefix)
                }
                (ast::TopLevel::GlobImport(module_path), None) => {
                    self.handle_glob_import(module_path)
                }
                _ => {}
            }
        }
        for (top_level_index, top_level) in module.top_levels.iter().enumerate() {
            let ast::TopLevel::NewType(name, params, _) = top_level else {
                continue;
            };
            let Some(id) =
                self.item_id_at_source(module_id, top_level_index, top_level, &name.span)
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
                .entry(name.name.clone())
                .or_insert_with(|| alias.clone());
            self.context
                .type_aliases
                .insert(local_item_lookup_name(local_prefix, &name.name), alias);
        }
        for (top_level_index, top_level) in ordered_headers(module) {
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
                    let strukt = self.collect_struct_header(sd, id);
                    if local_prefix.is_some() {
                        self.context
                            .structs
                            .entry(sd.name.name.clone())
                            .or_insert_with(|| strukt.clone());
                    }
                    self.context
                        .structs
                        .insert(local_item_lookup_name(local_prefix, &sd.name.name), strukt);
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
                    let enum_ = self.collect_enum_header(ed, id);
                    if local_prefix.is_some() {
                        self.context
                            .enums
                            .entry(ed.name.name.clone())
                            .or_insert_with(|| enum_.clone());
                    }
                    self.context
                        .enums
                        .insert(local_item_lookup_name(local_prefix, &ed.name.name), enum_);
                }
                ast::TopLevel::NewType(name, params, target) => {
                    let Some(id) =
                        self.item_id_at_source(module_id, top_level_index, top_level, &name.span)
                    else {
                        continue;
                    };
                    let alias = headers::build_type_alias_with_id(
                        &mut self.context,
                        name,
                        params,
                        target,
                        id,
                    );
                    if local_prefix.is_some() {
                        self.context
                            .type_aliases
                            .entry(name.name.clone())
                            .or_insert_with(|| alias.clone());
                    }
                    self.context
                        .type_aliases
                        .insert(local_item_lookup_name(local_prefix, &name.name), alias);
                }
                ast::TopLevel::TraitDecl(td) => {
                    let lookup_name = local_prefix
                        .map(|prefix| local_item_lookup_name(Some(prefix), &td.name.name))
                        .or_else(|| {
                            self.context
                                .current_crate_name
                                .as_ref()
                                .map(|crate_name| format!("{}::{}", crate_name, td.name.name))
                        })
                        .unwrap_or_else(|| td.name.name.clone());
                    let Some(id) = self.item_id_at_source(
                        module_id,
                        top_level_index,
                        top_level,
                        &td.name.span,
                    ) else {
                        continue;
                    };
                    let Some(member_ids) = self.trait_member_ids_for(id, &lookup_name, td) else {
                        continue;
                    };
                    let hir_trait =
                        headers::build_trait_with_id(&mut self.context, td, id, &member_ids);
                    if local_prefix.is_none() && self.context.current_crate_name.is_some() {
                        self.context
                            .canonical_import_aliases
                            .insert(td.name.name.clone(), id);
                        self.context
                            .import_aliases
                            .insert(td.name.name.clone(), lookup_name.clone());
                        self.context
                            .traits
                            .insert(td.name.name.clone(), hir_trait.clone());
                        self.context.traits.insert(lookup_name, hir_trait);
                    } else {
                        if local_prefix.is_some() {
                            self.context
                                .traits
                                .entry(td.name.name.clone())
                                .or_insert_with(|| hir_trait.clone());
                        }
                        self.context.traits.insert(lookup_name, hir_trait);
                    }
                }
                ast::TopLevel::FunctionSig(sig) => {
                    let lookup_name = local_item_lookup_name(local_prefix, &sig.name.name);
                    let Some(id) = self.item_id_at_source(
                        module_id,
                        top_level_index,
                        top_level,
                        &sig.name.span,
                    ) else {
                        continue;
                    };
                    self.collect_function_signature(sig, lookup_name, id);
                }
                ast::TopLevel::FunctionDecl(fd) => {
                    let lookup_name = local_item_lookup_name(local_prefix, &fd.name.name);
                    let had_short_function = self.context.functions.contains_key(&fd.name.name);
                    let Some(id) = self.item_id_at_source(
                        module_id,
                        top_level_index,
                        top_level,
                        &fd.name.span,
                    ) else {
                        continue;
                    };
                    self.collect_function_header(fd, lookup_name, id);
                    if local_prefix.is_some() && !had_short_function {
                        if let Some(func) = self.context.functions.get(&fd.name.name).cloned() {
                            let func_type = func.fn_type();
                            self.context
                                .scope
                                .define(fd.name.name.clone(), func_type, false);
                        } else if let Some(func) = self
                            .context
                            .functions
                            .get(&local_item_lookup_name(local_prefix, &fd.name.name))
                            .cloned()
                        {
                            let func_type = func.fn_type();
                            self.context
                                .scope
                                .define(fd.name.name.clone(), func_type, false);
                            self.context.functions.insert(fd.name.name.clone(), func);
                        }
                    }
                }
                ast::TopLevel::Extern(sig) => {
                    let Some(id) = self.item_id_at_source(
                        module_id,
                        top_level_index,
                        top_level,
                        &sig.name.span,
                    ) else {
                        continue;
                    };
                    let hir_extern = headers::build_extern_with_id(
                        &mut self.context,
                        sig,
                        sig.name.name.clone(),
                        id,
                    );
                    self.context.externs.push(hir_extern);
                }
                ast::TopLevel::Impl(imp) => {
                    let Some(id) = self.item_id_at_source(
                        module_id,
                        top_level_index,
                        top_level,
                        &imp.name.span(),
                    ) else {
                        continue;
                    };
                    let Some(method_ids) = self.impl_method_ids_for(id, "impl", imp) else {
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
                            self.child_module_id(module_id, mod_name, &mod_name.span)
                        else {
                            continue;
                        };
                        let next_prefix = local_prefix
                            .map(|prefix| format!("{}::{}", prefix, mod_name.name))
                            .unwrap_or_else(|| mod_name.name.clone());
                        let inner_exports = collect_exports(inner_module);
                        let inherited_references = child_export_references(
                            reference_exports,
                            &mod_name.name,
                            &next_prefix,
                        );
                        let next_reference_exports =
                            merge_export_references(&inner_exports, inherited_references);
                        self.collect_declarations_in_context(
                            inner_module,
                            child_module_id,
                            Some(&next_prefix),
                            &next_reference_exports,
                        );
                        self.collect_inline_qualified_declarations(
                            inner_module,
                            child_module_id,
                            &next_prefix,
                        );
                    } else {
                        self.collect_declarations_in_context(
                            inner_module,
                            module_id,
                            local_prefix,
                            reference_exports,
                        );
                    }
                }
                ast::TopLevel::Mod(ident, _) => {
                    if let Some(prefix) = local_prefix {
                        let Some(child_module_id) =
                            self.child_module_id(module_id, ident, &ident.span)
                        else {
                            continue;
                        };
                        self.handle_source_backed_mod_decl_in_context(
                            ident,
                            child_module_id,
                            prefix,
                            reference_exports,
                        );
                    } else {
                        let Some(child_module_id) =
                            self.child_module_id(module_id, ident, &ident.span)
                        else {
                            continue;
                        };
                        self.handle_source_backed_mod_decl_with_references(
                            ident,
                            child_module_id,
                            reference_exports,
                        );
                    }
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

    pub(super) fn collect_function_signature(
        &mut self,
        sig: &ast::FunctionSig,
        name: String,
        id: DefId,
    ) {
        let hir_sig = headers::build_function_sig_with_id(&mut self.context, sig, id);
        let func_type = hir_sig.fn_type();
        self.context.scope.define(name.clone(), func_type, false);
        self.context.function_sigs.insert(name.clone(), hir_sig);
    }

    pub(super) fn collect_function_header(
        &mut self,
        fd: &ast::FunctionDecl,
        name: String,
        function_id: DefId,
    ) {
        let func = if let Some(sig) = self.context.function_sigs.get(&name).cloned() {
            let mut func =
                headers::build_function_header_with_sig(&mut self.context, fd, &sig, function_id);
            self.context.function_sigs.remove(&name);
            func.is_unsafe = func.is_unsafe || sig.is_unsafe;
            func
        } else {
            headers::build_function_header_with_id(&mut self.context, fd, function_id)
        };

        let func_type = func.fn_type();
        self.context.scope.define(name.clone(), func_type, false);
        self.context.functions.insert(name, func);
    }
}

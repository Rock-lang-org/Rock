use std::collections::HashMap;

use crate::ast;
use crate::collect::context::{
    child_export_references, collect_exports, export_references_module, merge_export_references,
    path_names, CollectContext, LocalCollection,
};
use crate::collect::headers;
use crate::collect::{CollectedIdEnvironment, CollectedTraitMemberIds};
use crate::hir::{HirAssociatedTypeDecl, HirTrait, HirTypeAlias};
use crate::ids::{AssocTypeId, DefId, ModuleId};
use crate::types::Type;

pub(crate) struct LocalCollector {
    context: CollectContext,
    id_environment: CollectedIdEnvironment,
}

impl LocalCollector {
    #[cfg(test)]
    pub(crate) fn new(context: CollectContext) -> Self {
        Self::new_with_id_environment(context, CollectedIdEnvironment::default())
    }

    pub(crate) fn new_with_id_environment(
        context: CollectContext,
        id_environment: CollectedIdEnvironment,
    ) -> Self {
        Self {
            context,
            id_environment,
        }
    }

    pub fn collect_local_declarations(&mut self, module: &ast::Module, root_module_id: ModuleId) {
        if let Some(crate_name) = self.context.current_crate_name.clone() {
            let root_exports = collect_exports(module);
            self.predeclare_source_backed_trait_identities(
                module,
                root_module_id,
                &crate_name,
                &root_exports,
                crate_name == "stdlib",
            );
        }

        self.collect_declarations(module, root_module_id);
    }

    pub(crate) fn collect_crate_declarations(
        &mut self,
        module: &ast::Module,
        root_module_id: ModuleId,
        crate_name: &str,
        is_stdlib: bool,
    ) {
        let root_exports = collect_exports(module);

        self.predeclare_source_backed_trait_identities(
            module,
            root_module_id,
            crate_name,
            &root_exports,
            is_stdlib,
        );

        for (top_level_index, top_level) in module.top_levels.iter().enumerate() {
            let ast::TopLevel::NewType(name, _) = top_level else {
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
                generic_params: crate::type_lowering::lower_parse_generic_param_decls(
                    id,
                    &name.generics,
                ),
                ty: Type::Error,
            };
            self.context
                .type_aliases
                .insert(name.name.clone(), alias.clone());
            self.context
                .type_aliases
                .insert(format!("{}::{}", crate_name, name.name), alias);
        }

        for (top_level_index, top_level) in module.top_levels.iter().enumerate() {
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
                    let strukt = headers::build_struct_with_id(&mut self.context, sd, id);
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
                    let enum_ = headers::build_enum_with_id(&mut self.context, ed, id);
                    self.context
                        .enums
                        .insert(ed.name.name.clone(), enum_.clone());
                    self.context.enums.insert(qualified_name, enum_);
                }
                ast::TopLevel::NewType(name, target) => {
                    let qualified_name = format!("{}::{}", crate_name, name.name);
                    let Some(id) = self.item_id_at_source(
                        root_module_id,
                        top_level_index,
                        top_level,
                        &name.span,
                    ) else {
                        continue;
                    };
                    let alias =
                        headers::build_type_alias_with_id(&mut self.context, name, target, id);
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
                        &imp.name.span,
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
                    let is_prelude = is_stdlib && ident.name == "prelude";
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

    fn collect_declarations(&mut self, module: &ast::Module, module_id: ModuleId) {
        let reference_exports = collect_exports(module);
        self.collect_declarations_in_context(module, module_id, None, &reference_exports);
    }

    fn collect_declarations_in_context(
        &mut self,
        module: &ast::Module,
        module_id: ModuleId,
        local_prefix: Option<&str>,
        reference_exports: &HashMap<String, Option<String>>,
    ) {
        for (index, top_level) in module.top_levels.iter().enumerate() {
            if let ast::TopLevel::TraitDecl(declaration) = top_level {
                if let Some(id) = self.item_id_at_source(module_id, index, top_level, &declaration.name.span) {
                    let prefix = local_prefix.map(str::to_string).or_else(|| self.context.current_crate_name.clone());
                    let name = local_item_lookup_name(prefix.as_deref(), &declaration.name.name);
                    self.predeclare_trait_identity(declaration, id, &name, reference_exports);
                    if local_prefix.is_none() { self.context.canonical_import_aliases.insert(declaration.name.name.clone(), id); }
                }
            }
        }
        for (top_level_index, top_level) in module.top_levels.iter().enumerate() {
            let ast::TopLevel::NewType(name, _) = top_level else {
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
                generic_params: crate::type_lowering::lower_parse_generic_param_decls(
                    id,
                    &name.generics,
                ),
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
                    let strukt = headers::build_struct_with_id(&mut self.context, sd, id);
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
                    let enum_ = headers::build_enum_with_id(&mut self.context, ed, id);
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
                ast::TopLevel::NewType(name, target) => {
                    let Some(id) =
                        self.item_id_at_source(module_id, top_level_index, top_level, &name.span)
                    else {
                        continue;
                    };
                    let alias =
                        headers::build_type_alias_with_id(&mut self.context, name, target, id);
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
                            let param_types: Vec<Type> =
                                func.params.iter().map(|p| p.ty.clone()).collect();
                            let func_type = Type::function_with_safety(
                                param_types,
                                func.ret_type.clone(),
                                crate::types::FunctionSafety::from_is_unsafe(func.is_unsafe),
                            );
                            self.context
                                .scope
                                .define(fd.name.name.clone(), func_type, false);
                        } else if let Some(func) = self
                            .context
                            .functions
                            .get(&local_item_lookup_name(local_prefix, &fd.name.name))
                            .cloned()
                        {
                            let param_types: Vec<Type> =
                                func.params.iter().map(|p| p.ty.clone()).collect();
                            let func_type = Type::function_with_safety(
                                param_types,
                                func.ret_type.clone(),
                                crate::types::FunctionSafety::from_is_unsafe(func.is_unsafe),
                            );
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
                        &imp.name.span,
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
                ast::TopLevel::Import(path) => {
                    if let Some(prefix) = local_prefix {
                        self.handle_import_in_context(path, prefix);
                    } else {
                        self.handle_import(path);
                    }
                }
                ast::TopLevel::GlobImport(module_path) => {
                    if let Some(prefix) = local_prefix {
                        self.handle_glob_import_in_context(module_path, prefix);
                    } else {
                        self.handle_glob_import(module_path);
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

    fn collect_function_signature(&mut self, sig: &ast::FunctionSig, name: String, id: DefId) {
        let hir_sig = headers::build_function_sig_with_id(&mut self.context, sig, id);
        let func_type = Type::function_with_safety(
            hir_sig.params.clone(),
            hir_sig.ret.clone(),
            crate::types::FunctionSafety::from_is_unsafe(hir_sig.is_unsafe),
        );
        self.context.scope.define(name.clone(), func_type, false);
        self.context.function_sigs.insert(name.clone(), hir_sig);
    }

    fn collect_function_header(
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

        let param_types: Vec<Type> = func.params.iter().map(|p| p.ty.clone()).collect();
        let func_type = Type::function_with_safety(
            param_types,
            func.ret_type.clone(),
            crate::types::FunctionSafety::from_is_unsafe(func.is_unsafe),
        );
        self.context.scope.define(name.clone(), func_type, false);
        self.context.functions.insert(name, func);
    }

    fn collect_inline_qualified_declarations(
        &mut self,
        module: &ast::Module,
        module_id: ModuleId,
        prefix: &str,
    ) {
        for (top_level_index, top_level) in module.top_levels.iter().enumerate() {
            match top_level {
                ast::TopLevel::FunctionSig(sig) => {
                    let qualified_name = format!("{}::{}", prefix, sig.name.name);
                    let Some(id) = self.item_id_at_source(
                        module_id,
                        top_level_index,
                        top_level,
                        &sig.name.span,
                    ) else {
                        continue;
                    };
                    self.collect_function_signature(sig, qualified_name, id);
                }
                ast::TopLevel::FunctionDecl(fd) => {
                    let qualified_name = format!("{}::{}", prefix, fd.name.name);
                    if self.context.functions.contains_key(&qualified_name) {
                        continue;
                    }
                    let has_qualified_signature =
                        self.context.function_sigs.contains_key(&qualified_name);
                    if has_qualified_signature {
                        continue;
                    }
                    let Some(id) = self.item_id_at_source(
                        module_id,
                        top_level_index,
                        top_level,
                        &fd.name.span,
                    ) else {
                        continue;
                    };
                    self.collect_function_header(fd, qualified_name, id);
                }
                ast::TopLevel::StructDecl(sd) => {
                    let qualified_name = format!("{}::{}", prefix, sd.name.name);
                    let Some(id) = self.item_id_at_source(
                        module_id,
                        top_level_index,
                        top_level,
                        &sd.name.span,
                    ) else {
                        continue;
                    };
                    let strukt = headers::build_struct_with_id(&mut self.context, sd, id);
                    self.context.structs.insert(qualified_name, strukt);
                }
                ast::TopLevel::EnumDecl(ed) => {
                    let qualified_name = format!("{}::{}", prefix, ed.name.name);
                    let Some(id) = self.item_id_at_source(
                        module_id,
                        top_level_index,
                        top_level,
                        &ed.name.span,
                    ) else {
                        continue;
                    };
                    let enum_ = headers::build_enum_with_id(&mut self.context, ed, id);
                    self.context.enums.insert(qualified_name, enum_);
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
                        self.collect_inline_qualified_declarations(
                            inner,
                            child_module_id,
                            &new_prefix,
                        );
                    }
                }
                _ => {}
            }
        }
    }

    fn predeclare_source_backed_trait_identities(
        &mut self,
        module: &ast::Module,
        module_id: ModuleId,
        crate_name: &str,
        root_exports: &HashMap<String, Option<String>>,
        is_stdlib: bool,
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
                    let is_prelude = is_stdlib && ident.name == "prelude";
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

    fn predeclare_source_backed_mod_trait_identities(
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

    fn predeclare_source_backed_qualified_trait_identities(
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

    fn predeclare_trait_identity(
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

    fn handle_source_backed_mod_decl_with_references(
        &mut self,
        ident: &ast::Ident,
        module_id: ModuleId,
        reference_exports: &HashMap<String, Option<String>>,
    ) {
        let current_prefix = self.context.current_module_prefix();
        self.handle_source_backed_mod_decl_with_prefix(
            ident,
            module_id,
            current_prefix.as_deref(),
            Some(reference_exports),
        );
    }

    fn handle_source_backed_mod_decl_in_context(
        &mut self,
        ident: &ast::Ident,
        module_id: ModuleId,
        prefix: &str,
        reference_exports: &HashMap<String, Option<String>>,
    ) {
        self.handle_source_backed_mod_decl_with_prefix(
            ident,
            module_id,
            Some(prefix),
            Some(reference_exports),
        );
    }

    fn handle_source_backed_mod_decl_with_prefix(
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
        self.collect_source_backed_qualified_declarations(
            &loaded_module,
            module_id,
            &qualified_module_name,
            &exports,
            &reference_exports,
        );
        if self.context.current_crate_name.as_deref() == Some("stdlib")
            && qualified_module_name == "stdlib::prelude"
        {
            let export_ids = exports
                .iter()
                .filter_map(|(name, source)| {
                    let qualified_name = source
                        .clone()
                        .unwrap_or_else(|| format!("{}::{}", qualified_module_name, name));
                    self.context
                        .export_for_registered_item(&qualified_name)
                        .map(|export| (name.clone(), export))
                })
                .collect::<Vec<_>>();
            self.context.record_loaded_prelude_exports(export_ids);
        }

        self.context.current_module_path = old_path;
    }

    fn collect_source_backed_qualified_declarations(
        &mut self,
        module: &ast::Module,
        module_id: ModuleId,
        prefix: &str,
        exports: &HashMap<String, Option<String>>,
        reference_exports: &HashMap<String, Option<String>>,
    ) {
        for top_level in &module.top_levels {
            match top_level {
                ast::TopLevel::Import(path) => self.handle_import_in_context(path, prefix),
                ast::TopLevel::GlobImport(module_path) => {
                    self.handle_glob_import_in_context(module_path, prefix)
                }
                _ => {}
            }
        }

        for (top_level_index, top_level) in module.top_levels.iter().enumerate() {
            let ast::TopLevel::NewType(name, _) = top_level else {
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
                generic_params: crate::type_lowering::lower_parse_generic_param_decls(
                    id,
                    &name.generics,
                ),
                ty: Type::Error,
            };
            self.context
                .type_aliases
                .insert(format!("{}::{}", prefix, name.name), alias.clone());
            if exports.contains_key(&name.name) {
                self.context.type_aliases.insert(name.name.clone(), alias);
            }
        }

        for (top_level_index, top_level) in module.top_levels.iter().enumerate() {
            match top_level {
                ast::TopLevel::FunctionSig(sig) => {
                    let qualified_name = format!("{}::{}", prefix, sig.name.name);
                    let Some(id) = self.item_id_at_source(
                        module_id,
                        top_level_index,
                        top_level,
                        &sig.name.span,
                    ) else {
                        continue;
                    };
                    self.collect_function_signature(sig, qualified_name, id);
                }
                ast::TopLevel::FunctionDecl(fd) => {
                    let qualified_name = format!("{}::{}", prefix, fd.name.name);
                    let Some(id) = self.item_id_at_source(
                        module_id,
                        top_level_index,
                        top_level,
                        &fd.name.span,
                    ) else {
                        continue;
                    };
                    self.collect_function_header(fd, qualified_name, id);
                }
                ast::TopLevel::Extern(sig) => {
                    let qualified_name = format!("{}::{}", prefix, sig.name.name);
                    let Some(id) = self.item_id_at_source(
                        module_id,
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
                    let short_name = sd.name.name.clone();
                    let qualified_name = format!("{}::{}", prefix, short_name);
                    let Some(id) = self.item_id_at_source(
                        module_id,
                        top_level_index,
                        top_level,
                        &sd.name.span,
                    ) else {
                        continue;
                    };
                    let strukt = headers::build_struct_with_id(&mut self.context, sd, id);
                    self.context.structs.insert(qualified_name, strukt.clone());
                    if exports.contains_key(&short_name) {
                        self.context.structs.insert(short_name, strukt);
                    }
                }
                ast::TopLevel::EnumDecl(ed) => {
                    let short_name = ed.name.name.clone();
                    let qualified_name = format!("{}::{}", prefix, short_name);
                    let Some(id) = self.item_id_at_source(
                        module_id,
                        top_level_index,
                        top_level,
                        &ed.name.span,
                    ) else {
                        continue;
                    };
                    let enum_ = headers::build_enum_with_id(&mut self.context, ed, id);
                    self.context.enums.insert(qualified_name, enum_.clone());
                    if exports.contains_key(&short_name) {
                        self.context.enums.insert(short_name, enum_);
                    }
                }
                ast::TopLevel::NewType(name, target) => {
                    let short_name = name.name.clone();
                    let qualified_name = format!("{}::{}", prefix, short_name);
                    let Some(id) =
                        self.item_id_at_source(module_id, top_level_index, top_level, &name.span)
                    else {
                        continue;
                    };
                    let alias =
                        headers::build_type_alias_with_id(&mut self.context, name, target, id);
                    self.context
                        .type_aliases
                        .insert(qualified_name, alias.clone());
                    if exports.contains_key(&short_name) {
                        self.context.type_aliases.insert(short_name, alias);
                    }
                }
                ast::TopLevel::TraitDecl(td) => {
                    let trait_name = td.name.name.clone();
                    let qualified_name = format!("{}::{}", prefix, trait_name);
                    let Some(id) = self.item_id_at_source(
                        module_id,
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
                        .insert(qualified_name, hir_trait.clone());
                    if exports.contains_key(&trait_name) {
                        self.context.traits.insert(trait_name, hir_trait);
                    }
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
                        self.collect_source_backed_qualified_declarations(
                            inner,
                            child_module_id,
                            &new_prefix,
                            &inner_exports,
                            &next_reference_exports,
                        );
                    }
                }
                ast::TopLevel::Mod(ident, _) => {
                    let new_prefix = format!("{}::{}", prefix, ident.name);
                    if !exports.contains_key(&ident.name)
                        && !export_references_module(reference_exports, &ident.name, &new_prefix)
                    {
                        continue;
                    }

                    let Some(child_module_id) = self.child_module_id(module_id, ident, &ident.span)
                    else {
                        continue;
                    };
                    self.handle_source_backed_mod_decl_in_context(
                        ident,
                        child_module_id,
                        prefix,
                        reference_exports,
                    );
                }
                ast::TopLevel::Impl(imp) => {
                    let Some(id) = self.item_id_at_source(
                        module_id,
                        top_level_index,
                        top_level,
                        &imp.name.span,
                    ) else {
                        continue;
                    };
                    let Some(method_ids) = self.impl_method_ids_for(id, prefix, imp) else {
                        continue;
                    };
                    let hir_impl =
                        headers::build_impl_with_id(&mut self.context, imp, id, &method_ids);
                    self.context.impls.push(hir_impl);
                }
                ast::TopLevel::Import(path) => {
                    self.handle_import_in_context(path, prefix);
                }
                ast::TopLevel::GlobImport(module_path) => {
                    self.handle_glob_import_in_context(module_path, prefix);
                }
                ast::TopLevel::InfixOperator(precedence, name) => {
                    self.context
                        .infix_precedence
                        .insert(name.clone(), *precedence);
                }
                _ => {}
            }
        }

        self.register_export_aliases(prefix, exports);
    }

    fn register_export_aliases(&mut self, prefix: &str, exports: &HashMap<String, Option<String>>) {
        for (export_name, source) in exports {
            let Some(source) = source else {
                continue;
            };

            let qualified_export = format!("{}::{}", prefix, export_name);
            let qualified_source = qualify_export_source(&self.context, prefix, source);

            self.context
                .export_aliases
                .insert(qualified_export.clone(), qualified_source.clone());

            if let Some(func) = self.context.functions.get(&qualified_source) {
                let param_types: Vec<Type> = func.params.iter().map(|p| p.ty.clone()).collect();
                let func_type = Type::function_with_safety(
                    param_types,
                    func.ret_type.clone(),
                    crate::types::FunctionSafety::from_is_unsafe(func.is_unsafe),
                );
                self.context
                    .scope
                    .define(qualified_export.clone(), func_type, false);
            } else if let Some(binding) = self.context.scope.lookup(&qualified_source) {
                self.context
                    .scope
                    .define(qualified_export.clone(), binding.ty.clone(), false);
            }
        }
    }

    fn handle_import(&mut self, path: &ast::Path) {
        let canonical_prefix = self.context.current_module_prefix();
        self.context
            .handle_import(path, true, canonical_prefix.as_deref());
        let names = match path {
            ast::Path::Ident(path) => path_names(&path.path),
            ast::Path::Type(path) => path_names(&path.path),
        };
        if let Some(short_name) = names.last().cloned() {
            self.import_type_alias_local(
                canonical_prefix.as_deref().unwrap_or_default(),
                short_name,
                names.join("::"),
            );
        }

        if let Some(qualified_path) = qualify_local_path(&self.context, path) {
            self.context
                .handle_import(&qualified_path, true, canonical_prefix.as_deref());
            let names = match &qualified_path {
                ast::Path::Ident(path) => path_names(&path.path),
                ast::Path::Type(path) => path_names(&path.path),
            };
            if let Some(short_name) = names.last().cloned() {
                self.import_type_alias_local(
                    canonical_prefix.as_deref().unwrap_or_default(),
                    short_name,
                    names.join("::"),
                );
            }
        }
    }

    fn handle_import_in_context(&mut self, path: &ast::Path, prefix: &str) {
        let original_names = match path {
            ast::Path::Ident(ip) => path_names(&ip.path),
            ast::Path::Type(tp) => path_names(&tp.path),
        };
        if let (Some(first_segment), Some(short_name)) =
            (original_names.first(), original_names.last().cloned())
        {
            if local_path_segment_is_absolute(&self.context, first_segment) {
                self.context.handle_import(path, false, Some(prefix));
                let qualified_name = original_names.join("::");
                self.record_module_local_artifact_root_alias(prefix, &short_name, &qualified_name);
                self.import_type_alias_local(prefix, short_name.clone(), qualified_name.clone());
                self.context
                    .import_aliases
                    .insert(short_name, qualified_name);
            }
        }

        let resolved_path = qualify_local_path_with_prefix(&self.context, path, prefix)
            .unwrap_or_else(|| path.clone());
        self.context
            .handle_import(&resolved_path, false, Some(prefix));
        let names = match &resolved_path {
            ast::Path::Ident(ip) => path_names(&ip.path),
            ast::Path::Type(tp) => path_names(&tp.path),
        };

        if let Some(short_name) = names.last().cloned() {
            let qualified_name = names.join("::");
            self.record_module_local_artifact_root_alias(prefix, &short_name, &qualified_name);
            self.import_type_alias_local(prefix, short_name.clone(), qualified_name.clone());

            let first_segment = qualified_name.split("::").next().unwrap_or(&qualified_name);
            let is_dependency_prefix = self
                .context
                .loaded_module_paths
                .iter()
                .any(|(name, _)| name == first_segment);

            if !is_dependency_prefix {
                self.context
                    .import_aliases
                    .insert(short_name, qualified_name);
            }
        }
    }

    fn handle_glob_import(&mut self, module_path: &[String]) {
        self.context.handle_glob_import(module_path);

        if let Some(qualified_module_path) = qualify_local_module_path(&self.context, module_path) {
            self.context.handle_glob_import(&qualified_module_path);
        }
    }

    fn handle_glob_import_in_context(&mut self, module_path: &[String], prefix: &str) {
        let module_label = format!("{}::*", module_path.join("::"));
        match self.context.glob_import_targets(module_path) {
            Ok(targets) => {
                for (short_name, qualified_name) in targets {
                    let resolved_name =
                        qualify_local_import_target(&self.context, prefix, &qualified_name);
                    self.context.import_qualified_name(
                        short_name,
                        resolved_name,
                        false,
                        false,
                        Some(prefix),
                    );
                }
            }
            Err(err) => self
                .context
                .push_error(format!("Failed to import {}: {}", module_label, err)),
        }
    }

    fn import_type_alias_local(
        &mut self,
        prefix: &str,
        short_name: String,
        qualified_name: String,
    ) {
        if let Some(strukt) = self.context.structs.get(&qualified_name).cloned() {
            let item_id = Some(strukt.id);
            self.context.structs.insert(short_name.clone(), strukt);
            let resolver_alias = format!("{}::{}", prefix, short_name);
            self.record_import_type_alias(
                short_name,
                qualified_name,
                item_id,
                Some(resolver_alias),
            );
            return;
        }

        if let Some(enum_) = self.context.enums.get(&qualified_name).cloned() {
            let item_id = Some(enum_.id);
            self.context.enums.insert(short_name.clone(), enum_);
            let resolver_alias = format!("{}::{}", prefix, short_name);
            self.record_import_type_alias(
                short_name,
                qualified_name,
                item_id,
                Some(resolver_alias),
            );
            return;
        }

        if let Some(alias) = self.context.type_aliases.get(&qualified_name).cloned() {
            let item_id = Some(alias.id);
            self.context.type_aliases.insert(short_name.clone(), alias);
            let resolver_alias =
                (!prefix.is_empty()).then(|| format!("{}::{}", prefix, short_name));
            self.record_import_type_alias(short_name, qualified_name, item_id, resolver_alias);
            return;
        }

        if let Some(trait_) = self.context.traits.get(&qualified_name).cloned() {
            let item_id = Some(trait_.id);
            self.context.traits.insert(short_name.clone(), trait_);
            let resolver_alias = format!("{}::{}", prefix, short_name);
            self.record_import_type_alias(
                short_name,
                qualified_name,
                item_id,
                Some(resolver_alias),
            );
        }
    }

    fn record_module_local_artifact_root_alias(
        &mut self,
        prefix: &str,
        short_name: &str,
        qualified_name: &str,
    ) {
        let mut parts = qualified_name.split("::");
        let (Some(crate_name), Some(export_name), None) =
            (parts.next(), parts.next(), parts.next())
        else {
            return;
        };

        let Some(export) = self
            .context
            .dependency_root_export_ids
            .get(crate_name)
            .and_then(|exports| exports.get(export_name))
            .cloned()
        else {
            return;
        };

        let resolver_alias = format!("{}::{}", prefix, short_name);
        self.context
            .explicit_import_aliases
            .insert(resolver_alias.clone(), export.source.clone());
        self.context
            .canonical_import_aliases
            .insert(resolver_alias, export.id);
        self.context
            .canonical_names_by_id
            .insert(export.id, export.source);
    }

    fn record_import_type_alias(
        &mut self,
        short_name: String,
        qualified_name: String,
        item_id: Option<DefId>,
        local_resolver_alias: Option<String>,
    ) {
        let resolver_alias = local_resolver_alias.unwrap_or_else(|| short_name.clone());
        self.context
            .explicit_import_aliases
            .insert(resolver_alias.clone(), qualified_name.clone());
        if let Some(id) = item_id {
            self.context
                .canonical_import_aliases
                .insert(resolver_alias, id);
            self.context
                .canonical_names_by_id
                .insert(id, qualified_name);
        }
    }

    pub(crate) fn finish(
        self,
        item_index: &crate::collect::item_index::ItemIndex,
        root_module_id: crate::ids::ModuleId,
        current_crate_name: Option<&str>,
    ) -> LocalCollection {
        self.context
            .into_local_collection(item_index, root_module_id, current_crate_name)
    }

    fn item_id_at_source(
        &mut self,
        module_id: ModuleId,
        top_level_index: usize,
        top_level: &ast::TopLevel,
        span: &crate::lexer::Span,
    ) -> Option<DefId> {
        let Some(item) = self
            .id_environment
            .item_at_source(module_id, top_level_index)
        else {
            self.context.push_error_with_span(
                format!(
                    "missing canonical current-crate item identity at module {:?}, top-level {}",
                    module_id, top_level_index
                ),
                span.clone(),
            );
            return None;
        };
        if !item.kind.matches_top_level(top_level) {
            self.context.push_error_with_span(
                format!(
                    "current-crate item identity kind mismatch at module {:?}, top-level {}",
                    module_id, top_level_index
                ),
                span.clone(),
            );
            return None;
        }
        Some(item.def_id)
    }

    fn child_module_id(
        &mut self,
        parent_id: ModuleId,
        name: &ast::Ident,
        span: &crate::lexer::Span,
    ) -> Option<ModuleId> {
        let Some(module_id) = self.id_environment.child_module_id(parent_id, &name.name) else {
            self.context.push_error_with_span(
                format!(
                    "missing canonical current-crate module identity for {} under module {:?}",
                    name.name, parent_id
                ),
                span.clone(),
            );
            return None;
        };
        Some(module_id)
    }

    fn trait_member_ids_for(
        &mut self,
        trait_id: DefId,
        trait_name: &str,
        trait_decl: &ast::TraitDecl,
    ) -> Option<CollectedTraitMemberIds> {
        let member_ids = self
            .id_environment
            .trait_member_ids_by_owner
            .get(&trait_id)
            .cloned()
            .unwrap_or_default();
        let mut missing = false;

        for ident in trait_decl.methods.keys() {
            if !member_ids.methods.contains_key(&ident.name) {
                missing = true;
                self.context.push_error_with_span(
                    format!(
                        "missing canonical current-crate trait method identity for {trait_name}.{}",
                        ident.name
                    ),
                    ident.span.clone(),
                );
            }
        }

        for ident in trait_decl.signatures.keys() {
            if !member_ids.signatures.contains_key(&ident.name) {
                missing = true;
                self.context.push_error_with_span(
                    format!(
                    "missing canonical current-crate trait signature identity for {trait_name}.{}",
                    ident.name
                ),
                    ident.span.clone(),
                );
            }
        }

        if missing {
            None
        } else {
            Some(member_ids)
        }
    }

    fn impl_method_ids_for(
        &mut self,
        impl_id: DefId,
        impl_key: &str,
        impl_decl: &ast::Impl,
    ) -> Option<HashMap<String, DefId>> {
        let method_ids = self
            .id_environment
            .impl_method_ids_by_owner
            .get(&impl_id)
            .cloned()
            .unwrap_or_default();
        let mut missing = false;

        for ident in impl_decl.methods.keys() {
            if !method_ids.contains_key(&ident.name) {
                missing = true;
                self.context.push_error_with_span(
                    format!(
                        "missing canonical current-crate impl method identity for {impl_key}.{}",
                        ident.name
                    ),
                    ident.span.clone(),
                );
            }
        }

        if missing {
            None
        } else {
            Some(method_ids)
        }
    }
}

fn local_item_lookup_name(local_prefix: Option<&str>, name: &str) -> String {
    local_prefix
        .map(|prefix| format!("{}::{}", prefix, name))
        .unwrap_or_else(|| name.to_string())
}

fn qualify_export_source(context: &CollectContext, prefix: &str, source: &str) -> String {
    let first_segment = source.split("::").next().unwrap_or(source);
    let is_absolute = context
        .loaded_module_paths
        .iter()
        .any(|(name, _)| name == first_segment);

    if is_absolute {
        source.to_string()
    } else {
        format!("{}::{}", prefix, source)
    }
}

fn qualify_local_import_target(
    context: &CollectContext,
    prefix: &str,
    qualified_name: &str,
) -> String {
    if qualified_name == prefix || qualified_name.starts_with(&format!("{}::", prefix)) {
        return qualified_name.to_string();
    }

    let root_prefix = prefix.split("::").next().unwrap_or(prefix);
    if qualified_name == root_prefix || qualified_name.starts_with(&format!("{}::", root_prefix)) {
        return qualified_name.to_string();
    }

    let first_segment = qualified_name.split("::").next().unwrap_or(qualified_name);
    let is_absolute = context
        .loaded_module_paths
        .iter()
        .any(|(name, _)| name == first_segment);

    if is_absolute {
        qualified_name.to_string()
    } else {
        format!("{}::{}", prefix, qualified_name)
    }
}

fn qualify_local_path(context: &CollectContext, path: &ast::Path) -> Option<ast::Path> {
    let prefix = context.current_module_prefix()?;
    qualify_local_path_with_prefix(context, path, &prefix)
}

fn qualify_local_path_with_prefix(
    context: &CollectContext,
    path: &ast::Path,
    prefix: &str,
) -> Option<ast::Path> {
    match path {
        ast::Path::Ident(ip) => Some(ast::Path::Ident(ast::IdentifierPath {
            path: qualify_local_segments_with_prefix(context, &ip.path, prefix)?,
        })),
        ast::Path::Type(tp) => Some(ast::Path::Type(ast::TypePath {
            path: qualify_local_segments_with_prefix(context, &tp.path, prefix)?,
        })),
    }
}

fn qualify_local_module_path(
    context: &CollectContext,
    module_path: &[String],
) -> Option<Vec<String>> {
    let prefix = context.current_module_prefix()?;
    qualify_local_module_path_with_prefix(context, module_path, &prefix)
}

fn qualify_local_module_path_with_prefix(
    context: &CollectContext,
    module_path: &[String],
    prefix: &str,
) -> Option<Vec<String>> {
    let first_segment = module_path.first()?;
    if local_path_segment_is_absolute(context, first_segment) {
        return None;
    }

    let mut qualified: Vec<String> = prefix.split("::").map(ToString::to_string).collect();
    qualified.extend(module_path.iter().cloned());
    Some(qualified)
}

fn qualify_local_segments_with_prefix(
    context: &CollectContext,
    segments: &[ast::IdentOrType],
    prefix: &str,
) -> Option<Vec<ast::IdentOrType>> {
    let (first_segment, source_span) = match segments.first()? {
        ast::IdentOrType::Ident(ident) => (ident.name.as_str(), &ident.span),
        ast::IdentOrType::Type(ast::ParseType::Type(inner)) => (inner.name.as_str(), &inner.span),
        _ => return None,
    };

    if local_path_segment_is_absolute(context, first_segment) {
        return None;
    }

    let mut qualified: Vec<ast::IdentOrType> = prefix
        .split("::")
        .map(|segment| {
            ast::IdentOrType::Ident(ast::Ident {
                name: segment.to_string(),
                span: source_span.clone(),
            })
        })
        .collect();
    qualified.extend(segments.iter().cloned());
    Some(qualified)
}

fn local_path_segment_is_absolute(context: &CollectContext, first_segment: &str) -> bool {
    if context.current_crate_name.as_deref() == Some(first_segment) {
        return true;
    }

    if context
        .loaded_module_paths
        .iter()
        .any(|(name, _)| name == first_segment)
    {
        return true;
    }

    let prefix = format!("{}::", first_segment);
    context
        .functions
        .keys()
        .any(|name| name.starts_with(&prefix))
        || context.structs.keys().any(|name| name.starts_with(&prefix))
        || context.enums.keys().any(|name| name.starts_with(&prefix))
        || context.traits.keys().any(|name| name.starts_with(&prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ast::{
        FunctionDecl, FunctionSig, Ident, Impl, LambdaArrowKind, LambdaDecl, Module, ParseType,
        ParseTypeInner, StructDecl, TopLevel, TraitDecl,
    };
    use crate::collect::item_index::{ItemIndex, ItemKind, ItemRecord, ItemSourceId};
    use crate::ids::{CrateId, LocalDefId, ModuleId};
    use crate::lexer::Span;

    fn test_span(text: &str) -> Span {
        Span {
            file_path: "/test.rk".into(),
            start: 0,
            end: text.len(),
        }
    }

    fn type_inner(name: &str) -> ParseTypeInner {
        ParseTypeInner {
            name: name.to_string(),
            generics: vec![],
            span: test_span(name),
        }
    }

    fn ident(name: &str) -> Ident {
        Ident {
            name: name.to_string(),
            span: test_span(name),
        }
    }

    fn function_decl(name: &str) -> FunctionDecl {
        FunctionDecl {
            name: ident(name),
            lambda: LambdaDecl {
                parameters: vec![],
                body: crate::ast::Block { statements: vec![] },
                arrow_kind: LambdaArrowKind::Normal,
                span: crate::lexer::Span::test(),
            },
            self_receiver: None,
            is_unsafe: false,
            exported: false,
        }
    }

    fn function_sig(name: &str) -> FunctionSig {
        FunctionSig {
            name: ident(name),
            sig: ParseType::Unit(crate::lexer::Span::test()),
            where_clauses: vec![],
            self_receiver: None,
            is_unsafe: false,
            exported: false,
        }
    }

    #[test]
    fn local_collector_collects_root_struct_declarations() {
        let module = Module {
            name: None,
            top_levels: vec![TopLevel::StructDecl(StructDecl {
                name: type_inner("Point"),
                generic_params: vec![],
                fields: vec![],
                exported: false,
            })],
            is_inline: false,
            filepath: None,
        };

        let mut id_environment = CollectedIdEnvironment::default();
        let source = ItemSourceId {
            module_id: ModuleId(0),
            top_level_index: 0,
        };
        id_environment.current_source_items.insert(
            source,
            ItemRecord {
                name_span: test_span("Point"),
                def_id: DefId::new(CrateId(0), LocalDefId(0)),
                module_id: ModuleId(0),
                source,
                name: "Point".to_string(),
                kind: ItemKind::Struct,
            },
        );

        let context = CollectContext::bootstrap_for_collection(false, Some("test"), None);
        let mut collector = LocalCollector::new_with_id_environment(context, id_environment);
        collector.collect_local_declarations(&module, ModuleId(0));
        let collected = collector.finish(&ItemIndex::new(), ModuleId(0), Some("test"));

        assert!(collected.structs.contains_key("Point"));
    }

    #[test]
    fn local_collector_errors_when_member_id_environment_is_missing() {
        let module = Module {
            name: None,
            top_levels: vec![
                TopLevel::StructDecl(StructDecl {
                    name: type_inner("Box"),
                    generic_params: vec![],
                    fields: vec![],
                    exported: false,
                }),
                TopLevel::TraitDecl(TraitDecl {
                    where_clauses: Vec::new(),
                    name: type_inner("Show"),
                    generic_params: vec![],
                    for_: None,
                    associated_types: vec![],
                    methods: HashMap::from([(ident("show"), function_decl("show"))]),
                    signatures: HashMap::from([(ident("convert"), function_sig("convert"))]),
                    exported: false,
                    language_items: Default::default(),
                }),
                TopLevel::Impl(Impl {
                    name: type_inner("Show"),
                    for_: Some(ParseType::Type(type_inner("Box"))),
                    associated_types: vec![],
                    methods: HashMap::from([(ident("show"), function_decl("show"))]),
                    signatures: HashMap::new(),
                    where_clauses: vec![],
                }),
            ],
            is_inline: false,
            filepath: None,
        };
        let mut id_environment = CollectedIdEnvironment::default();
        for (top_level_index, name, kind) in [
            (0, "Box", ItemKind::Struct),
            (1, "Show", ItemKind::Trait),
            (2, "Show", ItemKind::Impl),
        ] {
            let source = ItemSourceId {
                module_id: ModuleId(0),
                top_level_index,
            };
            id_environment.current_source_items.insert(
                source,
                ItemRecord {
                    name_span: test_span(name),
                    def_id: DefId::new(CrateId(0), LocalDefId(top_level_index)),
                    module_id: ModuleId(0),
                    source,
                    name: name.to_string(),
                    kind,
                },
            );
        }

        let context = CollectContext::bootstrap_for_collection(false, Some("test"), None);
        let mut collector = LocalCollector::new_with_id_environment(context, id_environment);
        collector.collect_local_declarations(&module, ModuleId(0));
        let collected = collector.finish(&ItemIndex::new(), ModuleId(0), Some("test"));

        assert!(collected.errors.iter().any(|error| error.message.contains(
            "missing canonical current-crate trait method identity for test::Show.show"
        )));
        assert!(collected.errors.iter().any(|error| error.message.contains(
            "missing canonical current-crate trait signature identity for test::Show.convert"
        )));
        assert!(collected.errors.iter().any(|error| error
            .message
            .contains("missing canonical current-crate impl method identity for impl.show")));
    }
}

use super::ordering::ordered_headers;
use super::LocalCollector;
use crate::ast;
use crate::collect::context::{child_export_references, collect_exports, merge_export_references};
use crate::collect::headers;
use crate::hir::HirTypeAlias;
use crate::ids::ModuleId;
use crate::types::Type;
use std::collections::HashMap;

impl LocalCollector {
    pub(super) fn collect_inline_qualified_declarations(
        &mut self,
        module: &ast::Module,
        module_id: ModuleId,
        prefix: &str,
    ) {
        for (top_level_index, top_level) in ordered_headers(module) {
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
                    let strukt = self.collect_struct_header(sd, id);
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
                    let enum_ = self.collect_enum_header(ed, id);
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

    pub(super) fn handle_source_backed_mod_decl_with_references(
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

    pub(super) fn handle_source_backed_mod_decl_in_context(
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

    pub(super) fn handle_source_backed_mod_decl_with_prefix(
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
        if self.context.current_crate_name.as_deref() == Some("std")
            && qualified_module_name == "std::prelude"
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

    pub(super) fn collect_source_backed_qualified_declarations(
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
                .insert(format!("{}::{}", prefix, name.name), alias.clone());
            if exports.contains_key(&name.name) {
                self.context.type_aliases.insert(name.name.clone(), alias);
            }
        }

        for (top_level_index, top_level) in ordered_headers(module) {
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
                    let strukt = self.collect_struct_header(sd, id);
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
                    let enum_ = self.collect_enum_header(ed, id);
                    self.context.enums.insert(qualified_name, enum_.clone());
                    if exports.contains_key(&short_name) {
                        self.context.enums.insert(short_name, enum_);
                    }
                }
                ast::TopLevel::NewType(name, params, target) => {
                    let short_name = name.name.clone();
                    let qualified_name = format!("{}::{}", prefix, short_name);
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
                        &imp.name.span(),
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
}

use std::collections::HashMap;

use crate::ast;
use crate::collect::item_index::ItemIndex;
use crate::ids::{DefId, ModuleId};
use crate::lexer::Span;

use super::declaration_spans::{
    collect_parse_type_name_spans, top_level_declaration_span, top_level_name_span,
};
use super::{SemanticSourceMap, SourceSymbol};

impl SemanticSourceMap {
    pub fn from_item_index(index: &ItemIndex) -> Self {
        let mut map = Self::default();
        for item in index.items() {
            map.insert_definition_symbol(
                SourceSymbol::Definition(item.def_id),
                item.name_span.clone(),
                item.name_span.clone(),
                None,
            );
        }
        map
    }

    /// Complete the source map from the source AST and collected declaration IDs.
    /// This is deliberately kept outside artifact serialization: source locations are
    /// tied to the current source graph, not to reusable semantic products.
    pub fn from_declarations(
        index: &ItemIndex,
        root: &ast::Module,
        items: &crate::collect::DeclarationItems,
        loaded_modules: &crate::collect::item_index::SourceModuleMap,
        resolver: &crate::collect::resolver::ResolverTables,
    ) -> Self {
        let mut map = Self::from_item_index(index);
        let mut modules = HashMap::new();
        collect_source_modules(
            index,
            root,
            index
                .modules()
                .iter()
                .find(|module| module.parent.is_none())
                .map(|module| module.module_id),
            String::new(),
            loaded_modules,
            &mut modules,
        );

        for (module_id, module) in modules {
            map.record_module_declarations(index, module_id, &module, items, resolver);
        }
        map
    }

    fn record_module_declarations(
        &mut self,
        index: &ItemIndex,
        module_id: ModuleId,
        module: &ast::Module,
        items: &crate::collect::DeclarationItems,
        resolver: &crate::collect::resolver::ResolverTables,
    ) {
        for (ordinal, top_level) in module.top_levels.iter().enumerate() {
            let Some(item) = index.item_at_source(module_id, ordinal) else {
                continue;
            };
            let name_span = top_level_name_span(top_level, &item.name_span);
            let declaration_span = top_level_declaration_span(top_level, &name_span);
            self.insert_definition_symbol(
                SourceSymbol::Definition(item.def_id),
                name_span,
                declaration_span,
                None,
            );
            self.record_top_level_type_references(top_level, item.def_id, items, resolver);

            match top_level {
                ast::TopLevel::FunctionSig(signature) => {
                    if let Some(function) = items.function_sig(item.def_id) {
                        self.record_function_generics_from_parse(
                            function.id,
                            &function.generic_params,
                            &signature.sig,
                        );
                    }
                }
                ast::TopLevel::FunctionDecl(_function) => {
                    if let Some(function_decl) = items.function(item.def_id) {
                        self.record_generic_decls(
                            &function_decl.generic_params,
                            &[],
                            function_decl.id,
                        );
                    }
                }
                ast::TopLevel::StructDecl(structure) => {
                    if let Some(structure_decl) = items.structure(item.def_id) {
                        self.record_generic_decls(
                            &structure_decl.generic_params,
                            &structure.generic_params,
                            structure_decl.id,
                        );
                        for (field, source_field) in
                            structure_decl.fields.iter().zip(&structure.fields)
                        {
                            self.insert_field(
                                structure_decl.id,
                                field.id,
                                source_field.name.span.clone(),
                            );
                        }
                    }
                }
                ast::TopLevel::EnumDecl(enumeration) => {
                    if let Some(enum_decl) = items.enumeration(item.def_id) {
                        self.record_generic_decls(
                            &enum_decl.generic_params,
                            &enumeration.generic_params,
                            enum_decl.id,
                        );
                        for (variant, source_variant) in
                            enum_decl.variants.iter().zip(&enumeration.variants)
                        {
                            self.insert_variant(
                                enum_decl.id,
                                variant.id,
                                source_variant.name.span.clone(),
                            );
                            if let (
                                crate::hir::HirVariantFields::Named(fields),
                                ast::NamedFieldsOrTypesList::NamedFields(source_fields),
                            ) = (&variant.fields, &source_variant.fields)
                            {
                                for (field, source_field) in fields.iter().zip(source_fields) {
                                    self.insert_field(
                                        enum_decl.id,
                                        field.id,
                                        source_field.name.span.clone(),
                                    );
                                }
                            }
                        }
                    }
                }
                ast::TopLevel::NewType(_, params, _) => {
                    if let Some(alias) = items.type_alias(item.def_id) {
                        self.record_generic_decls(&alias.generic_params, params, alias.id);
                    }
                }
                ast::TopLevel::TraitDecl(trait_decl) => {
                    if let Some(trait_def) = items.trait_def(item.def_id) {
                        self.record_generic_decls(
                            &trait_def.generic_params,
                            &trait_decl.generic_params,
                            trait_def.id,
                        );
                        if let Some(target) = &trait_def.target {
                            if let Some(source_target) = &trait_decl.for_ {
                                self.insert_generic(
                                    target.id,
                                    source_target.name.span.clone(),
                                    trait_def.id,
                                );
                            }
                        }
                        for (associated, source_associated) in trait_def
                            .associated_types
                            .iter()
                            .zip(&trait_decl.associated_types)
                        {
                            self.insert_associated_type(
                                trait_def.id,
                                associated.id,
                                source_associated.name.span.clone(),
                            );
                        }
                        self.record_trait_members(trait_def, trait_decl);
                    }
                }
                ast::TopLevel::Impl(implementation) => {
                    if let Some(impl_def) = items.impl_def(item.def_id) {
                        self.record_impl_generics(impl_def, implementation);
                        for (associated, source_associated) in impl_def
                            .associated_types
                            .iter()
                            .zip(&implementation.associated_types)
                        {
                            self.insert_associated_type(
                                impl_def.id,
                                associated.id,
                                source_associated.name.span.clone(),
                            );
                        }
                        for (name, method) in &impl_def.methods {
                            if let Some(source_method) = implementation
                                .methods
                                .iter()
                                .find(|(ident, _)| ident.name == name.as_str())
                            {
                                self.record_function_definition(
                                    method,
                                    source_method.0.span.clone(),
                                    source_method.1,
                                );
                            } else if let Some(source_signature) = implementation
                                .signatures
                                .iter()
                                .find(|(ident, _)| ident.name == name.as_str())
                            {
                                self.record_signature_definition_from_function(
                                    method,
                                    source_signature.0.span.clone(),
                                    source_signature.1,
                                );
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn record_trait_members(&mut self, trait_def: &crate::hir::HirTrait, source: &ast::TraitDecl) {
        for (name, method) in &trait_def.methods {
            if let Some(source_method) = source
                .methods
                .iter()
                .find(|(ident, _)| ident.name == name.as_str())
            {
                self.record_function_definition(
                    method,
                    source_method.0.span.clone(),
                    source_method.1,
                );
            }
        }
        for (name, signature) in &trait_def.signatures {
            if let Some(source_signature) = source
                .signatures
                .iter()
                .find(|(ident, _)| ident.name == name.as_str())
            {
                self.record_signature_definition_from_sig(
                    signature,
                    source_signature.0.span.clone(),
                    source_signature.1,
                );
            }
        }
    }

    fn record_function_definition(
        &mut self,
        function: &crate::hir::HirFunction,
        name_span: Span,
        source: &ast::FunctionDecl,
    ) {
        let id = function.id;
        self.insert_definition_symbol(
            SourceSymbol::Definition(id),
            name_span,
            top_level_declaration_span(
                &ast::TopLevel::FunctionDecl(source.clone()),
                &source.name.span,
            ),
            None,
        );
    }

    fn record_signature_definition_from_function(
        &mut self,
        function: &crate::hir::HirFunction,
        name_span: Span,
        source: &ast::FunctionSig,
    ) {
        let id = function.id;
        self.insert_definition_symbol(
            SourceSymbol::Definition(id),
            name_span,
            top_level_declaration_span(
                &ast::TopLevel::FunctionSig(source.clone()),
                &source.name.span,
            ),
            None,
        );
        self.record_function_generics_from_parse(id, &function.generic_params, &source.sig);
    }

    fn record_signature_definition_from_sig(
        &mut self,
        signature: &crate::hir::HirFunctionSig,
        name_span: Span,
        source: &ast::FunctionSig,
    ) {
        let id = signature.id;
        self.insert_definition_symbol(
            SourceSymbol::Definition(id),
            name_span,
            top_level_declaration_span(
                &ast::TopLevel::FunctionSig(source.clone()),
                &source.name.span,
            ),
            None,
        );
        self.record_function_generics_from_parse(id, &signature.generic_params, &source.sig);
    }

    fn record_generic_decls(
        &mut self,
        generic_params: &[crate::types::GenericParamDecl],
        source_params: &[ast::GenericParamDecl],
        owner: DefId,
    ) {
        for generic in generic_params {
            if let Some(source) = source_params
                .iter()
                .find(|source| source.name.name == generic.name)
            {
                self.insert_generic(generic.id, source.name.span.clone(), owner);
            }
        }
    }

    fn record_function_generics_from_parse(
        &mut self,
        owner: DefId,
        generic_params: &[crate::types::GenericParamDecl],
        source: &ast::ParseType,
    ) {
        let mut spans = HashMap::new();
        collect_parse_type_name_spans(source, &mut spans);
        for generic in generic_params {
            if let Some(span) = spans.get(&generic.name) {
                self.insert_generic(generic.id, span.clone(), owner);
            }
        }
    }

    fn record_impl_generics(&mut self, impl_def: &crate::hir::HirImpl, source: &ast::Impl) {
        let mut spans = HashMap::new();
        collect_parse_type_name_spans(&source.name, &mut spans);
        if let Some(for_type) = &source.for_ {
            collect_parse_type_name_spans(for_type, &mut spans);
        }
        for generic in impl_def
            .type_generics
            .iter()
            .chain(&impl_def.trait_generics)
        {
            if let Some(span) = spans.get(&generic.name) {
                self.insert_generic(generic.id, span.clone(), impl_def.id);
            }
        }
    }
}

fn collect_source_modules(
    index: &ItemIndex,
    module: &ast::Module,
    module_id: Option<ModuleId>,
    prefix: String,
    loaded_modules: &crate::collect::item_index::SourceModuleMap,
    output: &mut HashMap<ModuleId, ast::Module>,
) {
    let Some(module_id) = module_id else {
        return;
    };
    output.insert(module_id, module.clone());

    for top_level in &module.top_levels {
        let (name, child_module) = match top_level {
            ast::TopLevel::Module(ast::ModuleDecl(inline)) => {
                let Some(name) = inline.name.as_ref() else {
                    continue;
                };
                (name.name.clone(), Some(inline.clone()))
            }
            ast::TopLevel::Mod(name, _) => (name.name.clone(), None),
            _ => continue,
        };
        let Some(child_id) = index.child_module_id(module_id, &name) else {
            continue;
        };
        let child_prefix = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}::{name}")
        };
        let child_module = child_module.or_else(|| {
            loaded_modules
                .get(
                    &child_prefix
                        .split("::")
                        .map(str::to_string)
                        .collect::<Vec<_>>(),
                )
                .cloned()
                .or_else(|| {
                    loaded_modules.iter().find_map(|(path, module)| {
                        (path.last().map(String::as_str) == Some(name.as_str())
                            && path.ends_with(
                                &child_prefix
                                    .split("::")
                                    .map(str::to_string)
                                    .collect::<Vec<_>>(),
                            ))
                        .then_some(module.clone())
                    })
                })
        });
        if let Some(child_module) = child_module {
            collect_source_modules(
                index,
                &child_module,
                Some(child_id),
                child_prefix,
                loaded_modules,
                output,
            );
        }
    }
}

use std::collections::HashMap;

use crate::ast;
use crate::collect::item_index::ItemIndex;
use crate::ids::{AssocTypeId, DefId, FieldId, HirLocalId, ModuleId, VariantId};
use crate::lexer::Span;
use crate::types::GenericParamId;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SourceSymbol {
    Definition(DefId),
    Local {
        owner: DefId,
        local: HirLocalId,
    },
    Field {
        owner: DefId,
        field: FieldId,
    },
    Variant {
        owner: DefId,
        variant: VariantId,
    },
    AssociatedType {
        owner: DefId,
        associated: AssocTypeId,
    },
    Generic(GenericParamId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSymbolInfo {
    pub name_span: Span,
    pub declaration_span: Option<Span>,
    pub lexical_owner: Option<DefId>,
    pub scope_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceReference {
    pub span: Span,
    pub target: SourceSymbol,
    pub lexical_owner: Option<DefId>,
    pub scope_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceScope {
    pub id: u32,
    pub owner: DefId,
    pub parent: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct SemanticSourceMap {
    symbols: HashMap<SourceSymbol, SourceSymbolInfo>,
    references: Vec<SourceReference>,
    expressions: Vec<Span>,
    type_annotations: Vec<Span>,
    scopes: Vec<SourceScope>,
}

impl SemanticSourceMap {
    /// Return the narrowest declaration or reference containing a byte offset.
    pub fn symbol_at(&self, path: &std::path::Path, offset: usize) -> Option<(SourceSymbol, Span)> {
        let references = self.references.iter().filter_map(|reference| {
            span_contains(&reference.span, path, offset)
                .then(|| (reference.target.clone(), reference.span.clone()))
        });
        let declarations = self.symbols.iter().filter_map(|(symbol, info)| {
            span_contains(&info.name_span, path, offset)
                .then(|| (symbol.clone(), info.name_span.clone()))
        });

        references
            .chain(declarations)
            .min_by_key(|(_, span)| span.end.saturating_sub(span.start))
    }

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
                        self.record_generic_decls(&enum_decl.generic_params, &[], enum_decl.id);
                        self.record_generic_spans(
                            enum_decl.id,
                            &enum_decl.generic_params,
                            &enumeration.name.generics,
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

    fn record_top_level_type_references(
        &mut self,
        top_level: &ast::TopLevel,
        owner: DefId,
        items: &crate::collect::DeclarationItems,
        resolver: &crate::collect::resolver::ResolverTables,
    ) {
        let generic_params = match top_level {
            ast::TopLevel::StructDecl(_) => items
                .structure(owner)
                .map(|item| item.generic_params.as_slice()),
            ast::TopLevel::EnumDecl(_) => items
                .enumeration(owner)
                .map(|item| item.generic_params.as_slice()),
            ast::TopLevel::FunctionSig(_) => items
                .function_sig(owner)
                .map(|item| item.generic_params.as_slice()),
            _ => None,
        }
        .unwrap_or_default();
        let mut generic_ids = generic_params
            .iter()
            .map(|param| (param.name.clone(), param.id))
            .collect::<HashMap<_, _>>();
        match top_level {
            ast::TopLevel::TraitDecl(_) => {
                if let Some(trait_def) = items.trait_def(owner) {
                    for generic in &trait_def.generic_params {
                        generic_ids.insert(generic.name.clone(), generic.id);
                    }
                    if let Some(target) = &trait_def.target {
                        generic_ids.insert(target.name.clone(), target.id);
                    }
                }
            }
            ast::TopLevel::Impl(_) => {
                if let Some(impl_def) = items.impl_def(owner) {
                    for generic in impl_def
                        .type_generics
                        .iter()
                        .chain(&impl_def.trait_generics)
                    {
                        generic_ids.insert(generic.name.clone(), generic.id);
                    }
                }
            }
            _ => {}
        }
        match top_level {
            ast::TopLevel::StructDecl(structure) => {
                for field in &structure.fields {
                    record_type_references(&field.ty, &generic_ids, resolver, self);
                }
            }
            ast::TopLevel::EnumDecl(enumeration) => {
                for variant in &enumeration.variants {
                    match &variant.fields {
                        ast::NamedFieldsOrTypesList::NamedFields(fields) => {
                            for field in fields {
                                record_type_references(&field.ty, &generic_ids, resolver, self);
                            }
                        }
                        ast::NamedFieldsOrTypesList::TypesList(types) => {
                            for ty in types {
                                record_type_references(ty, &generic_ids, resolver, self);
                            }
                        }
                    }
                }
            }
            ast::TopLevel::FunctionSig(signature) | ast::TopLevel::Extern(signature) => {
                record_type_references(&signature.sig, &generic_ids, resolver, self);
                for clause in &signature.where_clauses {
                    record_type_references(&clause.subject, &generic_ids, resolver, self);
                    if let Some(bound) = &clause.trait_bound {
                        record_type_references(bound, &generic_ids, resolver, self);
                    }
                }
            }
            ast::TopLevel::TraitDecl(trait_decl) => {
                let Some(trait_def) = items.trait_def(owner) else {
                    return;
                };
                let associated_ids = trait_def
                    .associated_types
                    .iter()
                    .map(|associated| (associated.name.clone(), associated.id))
                    .collect::<HashMap<_, _>>();
                for generic in &trait_decl.name.generics {
                    record_type_references(generic, &generic_ids, resolver, self);
                }
                if let Some(target) = &trait_decl.for_ {
                    if let Some(kind) = &target.kind {
                        record_type_application(kind, &generic_ids, resolver, self);
                    }
                }
                for associated in &trait_decl.associated_types {
                    if let Some(kind) = &associated.kind {
                        record_type_application(kind, &generic_ids, resolver, self);
                    }
                }
                for clause in &trait_decl.where_clauses {
                    record_type_references(&clause.subject, &generic_ids, resolver, self);
                    record_associated_type_references(
                        &clause.subject,
                        trait_def.id,
                        &associated_ids,
                        self,
                    );
                    if let Some(bound) = &clause.trait_bound {
                        record_type_references(bound, &generic_ids, resolver, self);
                        record_associated_type_references(
                            bound,
                            trait_def.id,
                            &associated_ids,
                            self,
                        );
                    }
                }
                for (name, signature) in &trait_decl.signatures {
                    let Some(hir_signature) = trait_def.signatures.get(&name.name) else {
                        continue;
                    };
                    let mut method_generics = generic_ids.clone();
                    for generic in &hir_signature.generic_params {
                        method_generics.insert(generic.name.clone(), generic.id);
                    }
                    record_type_references(&signature.sig, &method_generics, resolver, self);
                    record_associated_type_references(
                        &signature.sig,
                        trait_def.id,
                        &associated_ids,
                        self,
                    );
                    for clause in &signature.where_clauses {
                        record_type_references(&clause.subject, &method_generics, resolver, self);
                        record_associated_type_references(
                            &clause.subject,
                            trait_def.id,
                            &associated_ids,
                            self,
                        );
                        if let Some(bound) = &clause.trait_bound {
                            record_type_references(bound, &method_generics, resolver, self);
                            record_associated_type_references(
                                bound,
                                trait_def.id,
                                &associated_ids,
                                self,
                            );
                        }
                    }
                }
            }
            ast::TopLevel::Impl(implementation) => {
                let Some(impl_def) = items.impl_def(owner) else {
                    return;
                };
                let associated_ids = impl_def
                    .associated_types
                    .iter()
                    .map(|associated| (associated.name.clone(), associated.id))
                    .collect::<HashMap<_, _>>();
                if let Some(trait_id) = impl_def.trait_id {
                    self.record_reference(
                        type_inner_name_span(&implementation.name),
                        SourceSymbol::Definition(trait_id),
                        None,
                    );
                    for generic in &implementation.name.generics {
                        record_type_references(generic, &generic_ids, resolver, self);
                    }
                } else {
                    record_type_inner_reference(&implementation.name, &generic_ids, resolver, self);
                }
                if let Some(receiver) = &implementation.for_ {
                    record_type_references(receiver, &generic_ids, resolver, self);
                    record_associated_type_references(receiver, impl_def.id, &associated_ids, self);
                }
                for clause in &implementation.where_clauses {
                    record_type_references(&clause.subject, &generic_ids, resolver, self);
                    record_associated_type_references(
                        &clause.subject,
                        impl_def.id,
                        &associated_ids,
                        self,
                    );
                    if let Some(bound) = &clause.trait_bound {
                        record_type_references(bound, &generic_ids, resolver, self);
                        record_associated_type_references(
                            bound,
                            impl_def.id,
                            &associated_ids,
                            self,
                        );
                    }
                }
                for associated in &implementation.associated_types {
                    record_type_references(&associated.ty, &generic_ids, resolver, self);
                    record_associated_type_references(
                        &associated.ty,
                        impl_def.id,
                        &associated_ids,
                        self,
                    );
                    if let Some(kind) = &associated.kind {
                        record_type_application(kind, &generic_ids, resolver, self);
                    }
                }
                for (name, signature) in &implementation.signatures {
                    let Some(hir_method) = impl_def.methods.get(&name.name) else {
                        continue;
                    };
                    let mut method_generics = generic_ids.clone();
                    for generic in &hir_method.generic_params {
                        method_generics.insert(generic.name.clone(), generic.id);
                    }
                    record_type_references(&signature.sig, &method_generics, resolver, self);
                    record_associated_type_references(
                        &signature.sig,
                        impl_def.id,
                        &associated_ids,
                        self,
                    );
                    for clause in &signature.where_clauses {
                        record_type_references(&clause.subject, &method_generics, resolver, self);
                        record_associated_type_references(
                            &clause.subject,
                            impl_def.id,
                            &associated_ids,
                            self,
                        );
                        if let Some(bound) = &clause.trait_bound {
                            record_type_references(bound, &method_generics, resolver, self);
                            record_associated_type_references(
                                bound,
                                impl_def.id,
                                &associated_ids,
                                self,
                            );
                        }
                    }
                }
            }
            _ => {}
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

    fn record_generic_spans(
        &mut self,
        owner: DefId,
        generic_params: &[crate::types::GenericParamDecl],
        sources: &[ast::ParseType],
    ) {
        let mut spans = HashMap::new();
        for source in sources {
            collect_parse_type_name_spans(source, &mut spans);
        }
        for generic in generic_params {
            if let Some(span) = spans.get(&generic.name) {
                self.insert_generic(generic.id, span.clone(), owner);
            }
        }
    }

    fn record_impl_generics(&mut self, impl_def: &crate::hir::HirImpl, source: &ast::Impl) {
        let mut spans = HashMap::new();
        collect_parse_type_name_spans(&ast::ParseType::Type(source.name.clone()), &mut spans);
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

    pub fn insert_symbol(
        &mut self,
        symbol: SourceSymbol,
        name_span: Span,
        declaration_span: Option<Span>,
        lexical_owner: Option<DefId>,
    ) {
        self.symbols
            .entry(symbol)
            .and_modify(|info| {
                if declaration_span.is_some() {
                    info.declaration_span = declaration_span.clone();
                }
                if lexical_owner.is_some() {
                    info.lexical_owner = lexical_owner;
                }
            })
            .or_insert(SourceSymbolInfo {
                name_span,
                declaration_span,
                lexical_owner,
                scope_id: None,
            });
    }

    fn insert_definition_symbol(
        &mut self,
        symbol: SourceSymbol,
        name_span: Span,
        declaration_span: Span,
        lexical_owner: Option<DefId>,
    ) {
        self.insert_symbol(symbol, name_span, Some(declaration_span), lexical_owner);
    }

    pub fn symbol(&self, symbol: &SourceSymbol) -> Option<&SourceSymbolInfo> {
        self.symbols.get(symbol)
    }

    pub fn definition_span(&self, id: DefId) -> Option<&Span> {
        self.symbol(&SourceSymbol::Definition(id))
            .map(|info| &info.name_span)
    }

    pub fn definition_declaration_span(&self, id: DefId) -> Option<&Span> {
        self.symbol(&SourceSymbol::Definition(id))
            .and_then(|info| info.declaration_span.as_ref())
    }

    pub fn insert_definition(&mut self, id: DefId, span: Span) {
        self.insert_symbol(SourceSymbol::Definition(id), span.clone(), Some(span), None);
    }

    pub fn insert_local(&mut self, owner: DefId, local: HirLocalId, span: Span) {
        self.insert_local_in_scope(owner, local, span, None);
    }

    pub fn insert_local_in_scope(
        &mut self,
        owner: DefId,
        local: HirLocalId,
        span: Span,
        scope_id: Option<u32>,
    ) {
        self.insert_symbol(
            SourceSymbol::Local { owner, local },
            span.clone(),
            Some(span),
            Some(owner),
        );
        if let Some(info) = self.symbols.get_mut(&SourceSymbol::Local { owner, local }) {
            info.scope_id = scope_id;
        }
    }

    pub fn local_span(&self, owner: DefId, local: HirLocalId) -> Option<&Span> {
        self.symbol(&SourceSymbol::Local { owner, local })
            .map(|info| &info.name_span)
    }

    pub fn insert_field(&mut self, owner: DefId, field: FieldId, span: Span) {
        self.insert_symbol(
            SourceSymbol::Field { owner, field },
            span.clone(),
            Some(span),
            Some(owner),
        );
    }

    pub fn insert_variant(&mut self, owner: DefId, variant: VariantId, span: Span) {
        self.insert_symbol(
            SourceSymbol::Variant { owner, variant },
            span.clone(),
            Some(span),
            Some(owner),
        );
    }

    pub fn insert_associated_type(&mut self, owner: DefId, associated: AssocTypeId, span: Span) {
        self.insert_symbol(
            SourceSymbol::AssociatedType { owner, associated },
            span.clone(),
            Some(span),
            Some(owner),
        );
    }

    pub fn insert_generic(&mut self, id: GenericParamId, span: Span, owner: DefId) {
        self.insert_symbol(
            SourceSymbol::Generic(id),
            span.clone(),
            Some(span),
            Some(owner),
        );
    }

    pub fn record_reference(
        &mut self,
        span: Span,
        target: SourceSymbol,
        lexical_owner: Option<DefId>,
    ) {
        self.record_reference_in_scope(span, target, lexical_owner, None);
    }

    pub fn record_reference_in_scope(
        &mut self,
        span: Span,
        target: SourceSymbol,
        lexical_owner: Option<DefId>,
        scope_id: Option<u32>,
    ) {
        if self.references.iter().any(|reference| {
            reference.span == span
                && reference.target == target
                && reference.lexical_owner == lexical_owner
                && reference.scope_id == scope_id
        }) {
            return;
        }
        self.references.push(SourceReference {
            span,
            target,
            lexical_owner,
            scope_id,
        });
    }

    pub fn references(&self) -> &[SourceReference] {
        &self.references
    }

    pub fn reference_spans<'a>(
        &'a self,
        target: &'a SourceSymbol,
    ) -> impl Iterator<Item = &'a Span> + 'a {
        self.references
            .iter()
            .filter(move |reference| &reference.target == target)
            .map(|reference| &reference.span)
    }

    pub fn record_expression(&mut self, span: Span) {
        self.expressions.push(span);
    }

    pub fn expression_spans(&self) -> &[Span] {
        &self.expressions
    }

    pub fn record_type_annotation(&mut self, span: Span) {
        self.type_annotations.push(span);
    }

    pub fn type_annotation_spans(&self) -> &[Span] {
        &self.type_annotations
    }

    pub fn record_scope(&mut self, id: u32, owner: DefId, parent: Option<u32>) {
        self.scopes.push(SourceScope { id, owner, parent });
    }

    pub fn insert_scope(&mut self, owner: DefId, parent: Option<u32>) -> u32 {
        let id = self.scopes.len() as u32;
        self.record_scope(id, owner, parent);
        id
    }

    pub fn scopes(&self) -> &[SourceScope] {
        &self.scopes
    }

    /// Copy source ownership metadata from a semantic owner to a generated owner.
    ///
    /// Generated trait-default methods receive fresh `DefId`s during conformance,
    /// while their parameters and generic binders still originate in the trait
    /// declaration. This session-only alias keeps those spans available to MIR
    /// without putting editor metadata in artifacts.
    pub fn remap_owner(&mut self, from: DefId, to: DefId) {
        let aliases = self
            .symbols
            .iter()
            .filter_map(|(symbol, info)| {
                remap_source_symbol(symbol, from, to).map(|symbol| (symbol, info.clone()))
            })
            .collect::<Vec<_>>();
        for (symbol, mut info) in aliases {
            if info.lexical_owner == Some(from) {
                info.lexical_owner = Some(to);
            }
            self.symbols.entry(symbol).or_insert(info);
        }

        let references = self
            .references
            .iter()
            .filter_map(|reference| {
                remap_source_symbol(&reference.target, from, to).map(|target| SourceReference {
                    span: reference.span.clone(),
                    target,
                    lexical_owner: reference.lexical_owner.map(|owner| {
                        if owner == from {
                            to
                        } else {
                            owner
                        }
                    }),
                    scope_id: reference.scope_id,
                })
            })
            .collect::<Vec<_>>();
        self.references.extend(references);

        for scope in &mut self.scopes {
            if scope.owner == from {
                scope.owner = to;
            }
        }
    }
}

fn span_contains(span: &Span, path: &std::path::Path, offset: usize) -> bool {
    span.file_path == path && span.start <= offset && offset < span.end
}

fn remap_source_symbol(symbol: &SourceSymbol, from: DefId, to: DefId) -> Option<SourceSymbol> {
    match symbol {
        SourceSymbol::Definition(owner) if *owner == from => Some(SourceSymbol::Definition(to)),
        SourceSymbol::Local { owner, local } if *owner == from => Some(SourceSymbol::Local {
            owner: to,
            local: *local,
        }),
        SourceSymbol::Field { owner, field } if *owner == from => Some(SourceSymbol::Field {
            owner: to,
            field: *field,
        }),
        SourceSymbol::Variant { owner, variant } if *owner == from => Some(SourceSymbol::Variant {
            owner: to,
            variant: *variant,
        }),
        SourceSymbol::AssociatedType { owner, associated } if *owner == from => {
            Some(SourceSymbol::AssociatedType {
                owner: to,
                associated: *associated,
            })
        }
        SourceSymbol::Generic(param) if param.owner == from => {
            Some(SourceSymbol::Generic(GenericParamId {
                owner: to,
                ..*param
            }))
        }
        _ => None,
    }
}

fn record_type_references(
    ty: &ast::ParseType,
    generic_ids: &HashMap<String, GenericParamId>,
    resolver: &crate::collect::resolver::ResolverTables,
    map: &mut SemanticSourceMap,
) {
    match ty {
        ast::ParseType::Object(object) => {
            for ty in object.types() { record_type_references(ty, generic_ids, resolver, map); }
        }
        ast::ParseType::Type(inner) => {
            if let Some(generic) = generic_ids.get(&inner.name) {
                map.record_reference(
                    type_inner_name_span(inner),
                    SourceSymbol::Generic(*generic),
                    Some(generic.owner),
                );
            } else if let Some(def_id) = resolver.resolve_item_or_alias(&inner.name) {
                map.record_reference(
                    type_inner_name_span(inner),
                    SourceSymbol::Definition(def_id),
                    None,
                );
            }
            for generic in &inner.generics {
                record_type_references(generic, generic_ids, resolver, map);
            }
        }
        ast::ParseType::Application(application) => {
            record_type_references(&application.constructor, generic_ids, resolver, map);
            for arg in &application.args {
                record_type_references(arg, generic_ids, resolver, map);
            }
        }
        ast::ParseType::Function(types) | ast::ParseType::Tuple(types) => {
            for ty in types {
                record_type_references(ty, generic_ids, resolver, map);
            }
        }
        ast::ParseType::Lambda(lambda) => {
            record_type_references(&lambda.body, generic_ids, resolver, map);
        }
        ast::ParseType::Associated { base, .. } => {
            if let Some(generic) = generic_ids.get(&base.name) {
                map.record_reference(
                    type_inner_name_span(base),
                    SourceSymbol::Generic(*generic),
                    Some(generic.owner),
                );
            } else if let Some(def_id) = resolver.resolve_item_or_alias(&base.name) {
                map.record_reference(
                    type_inner_name_span(base),
                    SourceSymbol::Definition(def_id),
                    None,
                );
            }
            for generic in &base.generics {
                record_type_references(generic, generic_ids, resolver, map);
            }
        }
        ast::ParseType::Slice(inner)
        | ast::ParseType::Array { inner, .. }
        | ast::ParseType::Reference { pointee: inner, .. }
        | ast::ParseType::Pointer(inner) => {
            record_type_references(inner, generic_ids, resolver, map);
        }
        ast::ParseType::Hole(_) | ast::ParseType::Unit(_) => {}
    }
}

fn record_type_inner_reference(
    inner: &ast::ParseTypeInner,
    generic_ids: &HashMap<String, GenericParamId>,
    resolver: &crate::collect::resolver::ResolverTables,
    map: &mut SemanticSourceMap,
) {
    if let Some(generic) = generic_ids.get(&inner.name) {
        map.record_reference(
            type_inner_name_span(inner),
            SourceSymbol::Generic(*generic),
            Some(generic.owner),
        );
    } else if let Some(def_id) = resolver.resolve_item_or_alias(&inner.name) {
        map.record_reference(
            type_inner_name_span(inner),
            SourceSymbol::Definition(def_id),
            None,
        );
    }
    for generic in &inner.generics {
        record_type_references(generic, generic_ids, resolver, map);
    }
}

fn type_inner_name_span(inner: &ast::ParseTypeInner) -> Span {
    Span::new(
        inner.span.file_path.clone(),
        inner.span.start,
        inner.span.start + inner.name.len(),
    )
}

fn record_type_application(
    application: &ast::TypeApplication,
    generic_ids: &HashMap<String, GenericParamId>,
    resolver: &crate::collect::resolver::ResolverTables,
    map: &mut SemanticSourceMap,
) {
    record_type_references(&application.constructor, generic_ids, resolver, map);
    for arg in &application.args {
        record_type_references(arg, generic_ids, resolver, map);
    }
}

fn record_associated_type_references(
    ty: &ast::ParseType,
    owner: DefId,
    associated_types: &HashMap<String, crate::ids::AssocTypeId>,
    map: &mut SemanticSourceMap,
) {
    match ty {
        ast::ParseType::Object(object) => {
            for ty in object.types() { record_associated_type_references(ty, owner, associated_types, map); }
        }
        ast::ParseType::Associated { base, member } => {
            if let Some(associated) = associated_types.get(&member.name) {
                map.record_reference_in_scope(
                    member.span.clone(),
                    SourceSymbol::AssociatedType {
                        owner,
                        associated: *associated,
                    },
                    None,
                    None,
                );
            }
            for generic in &base.generics {
                record_associated_type_references(generic, owner, associated_types, map);
            }
        }
        ast::ParseType::Type(inner) => {
            for generic in &inner.generics {
                record_associated_type_references(generic, owner, associated_types, map);
            }
        }
        ast::ParseType::Application(application) => {
            record_associated_type_references(
                &application.constructor,
                owner,
                associated_types,
                map,
            );
            for arg in &application.args {
                record_associated_type_references(arg, owner, associated_types, map);
            }
        }
        ast::ParseType::Function(types) | ast::ParseType::Tuple(types) => {
            for ty in types {
                record_associated_type_references(ty, owner, associated_types, map);
            }
        }
        ast::ParseType::Lambda(lambda) => {
            record_associated_type_references(&lambda.body, owner, associated_types, map);
        }
        ast::ParseType::Slice(inner)
        | ast::ParseType::Array { inner, .. }
        | ast::ParseType::Reference { pointee: inner, .. }
        | ast::ParseType::Pointer(inner) => {
            record_associated_type_references(inner, owner, associated_types, map);
        }
        ast::ParseType::Hole(_) | ast::ParseType::Unit(_) => {}
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

fn top_level_declaration_span(top_level: &ast::TopLevel, name_span: &Span) -> Span {
    let mut end = name_span.end;
    let mut extend = |span: Span| {
        if span.file_path == name_span.file_path {
            end = end.max(span.end);
        }
    };
    match top_level {
        ast::TopLevel::FunctionDecl(function) => {
            for statement in &function.lambda.body.statements {
                if let Some(span) = ast_statement_span(statement) {
                    extend(span);
                }
            }
        }
        ast::TopLevel::FunctionSig(signature) | ast::TopLevel::Extern(signature) => {
            extend(signature.sig.span());
        }
        ast::TopLevel::StructDecl(structure) => {
            for generic in &structure.generic_params {
                extend(generic.span.clone());
            }
            for field in &structure.fields {
                extend(field.name.span.clone());
                extend(field.ty.span());
            }
        }
        ast::TopLevel::EnumDecl(enumeration) => {
            for variant in &enumeration.variants {
                extend(variant.name.span.clone());
                match &variant.fields {
                    ast::NamedFieldsOrTypesList::NamedFields(fields) => {
                        for field in fields {
                            extend(field.name.span.clone());
                            extend(field.ty.span());
                        }
                    }
                    ast::NamedFieldsOrTypesList::TypesList(types) => {
                        for ty in types {
                            extend(ty.span());
                        }
                    }
                }
            }
        }
        ast::TopLevel::TraitDecl(trait_decl) => {
            for generic in &trait_decl.generic_params {
                extend(generic.span.clone());
            }
            for clause in &trait_decl.where_clauses {
                extend(clause.subject.span());
                if let Some(bound) = &clause.trait_bound {
                    extend(bound.span());
                }
            }
            for method in trait_decl.methods.values() {
                extend(method.name.span.clone());
            }
            for signature in trait_decl.signatures.values() {
                extend(signature.sig.span());
            }
        }
        ast::TopLevel::Impl(implementation) => {
            for generic in &implementation.name.generics {
                extend(generic.span());
            }
            if let Some(receiver) = &implementation.for_ {
                extend(receiver.span());
            }
            for associated in &implementation.associated_types {
                extend(associated.name.span.clone());
                extend(associated.ty.span());
            }
            for method in implementation.methods.values() {
                extend(method.name.span.clone());
            }
            for signature in implementation.signatures.values() {
                extend(signature.sig.span());
            }
        }
        _ => {}
    }
    Span::new(name_span.file_path.clone(), name_span.start, end)
}

fn top_level_name_span(top_level: &ast::TopLevel, indexed: &Span) -> Span {
    let (name, span) = match top_level {
        ast::TopLevel::StructDecl(decl) => (&decl.name.name, &decl.name.span),
        ast::TopLevel::EnumDecl(decl) => (&decl.name.name, &decl.name.span),
        ast::TopLevel::TraitDecl(decl) => (&decl.name.name, &decl.name.span),
        ast::TopLevel::Impl(decl) => (&decl.name.name, &decl.name.span),
        _ => return indexed.clone(),
    };
    Span::new(span.file_path.clone(), span.start, span.start + name.len())
}

fn ast_statement_span(statement: &ast::Statement) -> Option<Span> {
    match statement {
        ast::Statement::Assignment(assignment) => ast_expression_span(&assignment.rhs),
        ast::Statement::Expression(expression) => ast_expression_span(expression),
        ast::Statement::Return(expression)
        | ast::Statement::Continue(expression)
        | ast::Statement::Break(expression) => expression.as_ref().and_then(ast_expression_span),
    }
}

fn ast_expression_span(expression: &ast::Expression) -> Option<Span> {
    match expression {
        ast::Expression::BinopExpr(_, operator, _) => Some(operator.span.clone()),
        ast::Expression::UnaryExpr(unary) => match unary {
            ast::UnaryExpr::UnaryExpr(operator, _) => Some(operator.span.clone()),
            ast::UnaryExpr::PrimaryExpr(primary) => match &primary.operand {
                ast::Operand::Literal(literal) => Some(literal.span.clone()),
                ast::Operand::Ident(path) => path.path.first().map(|segment| match segment {
                    ast::IdentOrType::Ident(ident) => ident.span.clone(),
                    ast::IdentOrType::Type(ty) => ty.span(),
                }),
                ast::Operand::SelfIdent(ident) => Some(ident.span.clone()),
                ast::Operand::CallHole(span) => Some(span.clone()),
                ast::Operand::Open(open) => Some(open.span.clone()),
                ast::Operand::Instance(instance) => {
                    instance.name.path.first().map(|segment| match segment {
                        ast::IdentOrType::Ident(ident) => ident.span.clone(),
                        ast::IdentOrType::Type(ty) => ty.span(),
                    })
                }
                ast::Operand::Expression(expression) => ast_expression_span(expression),
                _ => None,
            },
        },
        ast::Expression::CastExpr(expression, ty) => {
            ast_expression_span(expression).or_else(|| Some(ty.span()))
        }
        ast::Expression::Range(range) => Some(range.span.clone()),
    }
}

fn collect_parse_type_name_spans(ty: &ast::ParseType, output: &mut HashMap<String, Span>) {
    match ty {
        ast::ParseType::Object(object) => {
            for ty in object.types() { collect_parse_type_name_spans(ty, output); }
        }
        ast::ParseType::Type(inner) => {
            output
                .entry(inner.name.clone())
                .or_insert(inner.span.clone());
            for generic in &inner.generics {
                collect_parse_type_name_spans(generic, output);
            }
        }
        ast::ParseType::Function(types) | ast::ParseType::Tuple(types) => {
            for ty in types {
                collect_parse_type_name_spans(ty, output);
            }
        }
        ast::ParseType::Application(application) => {
            collect_parse_type_name_spans(&application.constructor, output);
            for arg in &application.args {
                collect_parse_type_name_spans(arg, output);
            }
        }
        ast::ParseType::Lambda(lambda) => {
            for param in &lambda.params {
                output
                    .entry(param.name.name.clone())
                    .or_insert(param.name.span.clone());
            }
            collect_parse_type_name_spans(&lambda.body, output);
        }
        ast::ParseType::Associated { base, member } => {
            output.entry(base.name.clone()).or_insert(base.span.clone());
            output
                .entry(member.name.clone())
                .or_insert(member.span.clone());
        }
        ast::ParseType::Slice(inner)
        | ast::ParseType::Array { inner, .. }
        | ast::ParseType::Reference { pointee: inner, .. }
        | ast::ParseType::Pointer(inner) => collect_parse_type_name_spans(inner, output),
        ast::ParseType::Hole(_) | ast::ParseType::Unit(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use crate::collect::item_index::{index_root_module_items, IndexingIds};
    use crate::ids::{CrateId, LocalDefId};

    fn span(start: usize, end: usize) -> Span {
        Span {
            file_path: PathBuf::from("/virtual/main.rk"),
            start,
            end,
        }
    }

    #[test]
    fn semantic_source_map_preserves_indexed_definition_name_span() {
        let path = PathBuf::from("/virtual/main.rk");
        let module = crate::parser::parse_source(
            path.clone(),
            "struct Widget\n\nmain = -> 0\n",
            &crate::Config::default(),
        )
        .expect("source should parse");
        let mut ids = IndexingIds::new_root();
        let index = index_root_module_items(&mut ids, &module);
        let widget = index
            .items()
            .iter()
            .find(|item| item.name == "Widget")
            .expect("Widget definition");

        let sources = SemanticSourceMap::from_item_index(&index);
        let span = sources
            .definition_span(widget.def_id)
            .expect("definition span");
        assert_eq!(span.file_path, path);
        assert_eq!(
            &"struct Widget\n\nmain = -> 0\n"[span.start..span.end],
            "Widget"
        );
    }

    #[test]
    fn semantic_source_map_records_collected_declarations_from_real_source() {
        let path = PathBuf::from("/virtual/decls.rk");
        let text = "struct Widget T\n    < value: T\n\nmain = -> 0\n";
        let module = crate::parser::parse_source(path.clone(), text, &crate::Config::default())
            .expect("source should parse");
        let program = crate::ast::Program { module };
        let declarations = crate::collect::collect(
            &program,
            &crate::crate_system::CrateContext::new(),
            false,
            None,
        )
        .expect("declarations should collect");
        let widget_id = declarations.item_index.defs_named("Widget")[0];
        let widget = declarations
            .items
            .structure(widget_id)
            .expect("Widget struct");
        let generic = widget.generic_params[0].id;
        let field = widget.fields[0].id;
        let sources = declarations.source_map;

        let declaration = sources
            .symbol(&SourceSymbol::Definition(widget_id))
            .expect("Widget definition");
        assert_eq!(
            &text[declaration.name_span.start..declaration.name_span.end],
            "Widget"
        );
        assert!(declaration.declaration_span.as_ref().unwrap().end > declaration.name_span.end);
        assert_eq!(
            &text[sources
                .symbol(&SourceSymbol::Generic(generic))
                .unwrap()
                .name_span
                .start
                ..sources
                    .symbol(&SourceSymbol::Generic(generic))
                    .unwrap()
                    .name_span
                    .end],
            "T"
        );
        let field_span = &sources
            .symbol(&SourceSymbol::Field {
                owner: widget_id,
                field,
            })
            .expect("field declaration")
            .name_span;
        assert_eq!(&text[field_span.start..field_span.end], "value");
        assert!(sources.references().iter().any(|reference| {
            reference.target == SourceSymbol::Generic(generic)
                && &text[reference.span.start..reference.span.end] == "T"
        }));
    }

    #[test]
    fn semantic_source_map_records_trait_and_impl_type_references_from_real_source() {
        let path = PathBuf::from("/virtual/trait_refs.rk");
        let text = "struct Box T\n    value: T\n\ntrait Parent T\n    type Output\n    parent: T -> &Self::Output\n\ntrait Child T\n    type Output\n    child: T -> T\n\nimpl Child I64 for Box I64 where I64: Parent I64\n    type Output = I64\n    child: I64 -> I64\n\nmain = -> 0\n";
        let module = crate::parser::parse_source(path, text, &crate::Config::default())
            .expect("trait and impl source should parse");
        let declarations = crate::collect::collect(
            &crate::ast::Program { module },
            &crate::crate_system::CrateContext::new(),
            false,
            None,
        )
        .expect("trait and impl declarations should collect");

        let references = declarations.source_map.references();
        let has_text = |name: &str| {
            references.iter().any(|reference| {
                &text[reference.span.start..reference.span.end] == name
                    && matches!(reference.target, SourceSymbol::Definition(_))
            })
        };
        assert!(has_text("Box"));
        assert!(has_text("Parent"));
        assert!(
            has_text("Child"),
            "recorded references: {:?}",
            references
                .iter()
                .map(|reference| &text[reference.span.start..reference.span.end])
                .collect::<Vec<_>>()
        );
        assert!(references.iter().any(|reference| {
            &text[reference.span.start..reference.span.end] == "T"
                && matches!(reference.target, SourceSymbol::Generic(_))
        }));
        assert!(references.iter().any(|reference| {
            &text[reference.span.start..reference.span.end] == "Output"
                && matches!(reference.target, SourceSymbol::AssociatedType { .. })
        }));
    }

    #[test]
    fn semantic_source_map_tracks_child_and_generic_symbols() {
        let owner = DefId::new(CrateId(0), LocalDefId(1));
        let child = DefId::new(CrateId(0), LocalDefId(2));
        let generic = GenericParamId { owner, index: 0 };
        let mut sources = SemanticSourceMap::default();
        sources.insert_definition(owner, span(1, 5));
        sources.insert_definition(child, span(23, 28));
        sources.insert_generic(generic, span(6, 7), owner);
        sources.insert_field(owner, FieldId(0), span(8, 9));
        sources.insert_variant(owner, VariantId(0), span(10, 15));
        sources.insert_associated_type(owner, AssocTypeId(0), span(16, 22));

        assert_eq!(
            &sources
                .symbol(&SourceSymbol::Generic(generic))
                .unwrap()
                .name_span,
            &span(6, 7)
        );
        assert!(sources.symbol(&SourceSymbol::Definition(child)).is_some());
        assert!(sources
            .symbol(&SourceSymbol::Field {
                owner,
                field: FieldId(0)
            })
            .is_some());
        assert!(sources
            .symbol(&SourceSymbol::Variant {
                owner,
                variant: VariantId(0)
            })
            .is_some());
        assert!(sources
            .symbol(&SourceSymbol::AssociatedType {
                owner,
                associated: AssocTypeId(0)
            })
            .is_some());
    }

    #[test]
    fn semantic_source_map_keeps_cross_module_and_shadowed_local_targets_distinct() {
        let owner = DefId::new(CrateId(0), LocalDefId(1));
        let imported = DefId::new(CrateId(2), LocalDefId(9));
        let first = HirLocalId(3);
        let second = HirLocalId(4);
        let mut sources = SemanticSourceMap::default();
        sources.insert_local(owner, first, span(1, 2));
        sources.insert_local(owner, second, span(3, 4));
        sources.record_reference(
            span(10, 15),
            SourceSymbol::Definition(imported),
            Some(owner),
        );
        sources.record_reference(
            span(20, 21),
            SourceSymbol::Local {
                owner,
                local: first,
            },
            Some(owner),
        );
        sources.record_reference(
            span(30, 31),
            SourceSymbol::Local {
                owner,
                local: second,
            },
            Some(owner),
        );

        assert_eq!(
            sources
                .reference_spans(&SourceSymbol::Definition(imported))
                .count(),
            1
        );
        assert_eq!(
            sources
                .reference_spans(&SourceSymbol::Local {
                    owner,
                    local: first
                })
                .count(),
            1
        );
        assert_eq!(
            sources
                .reference_spans(&SourceSymbol::Local {
                    owner,
                    local: second
                })
                .count(),
            1
        );
    }

    #[test]
    fn semantic_source_map_remaps_generated_method_owners_without_serialization() {
        let from = DefId::new(CrateId(0), LocalDefId(10));
        let to = DefId::new(CrateId(0), LocalDefId(20));
        let local = HirLocalId(3);
        let generic = GenericParamId {
            owner: from,
            index: 0,
        };
        let mut sources = SemanticSourceMap::default();
        sources.insert_definition(from, span(1, 4));
        sources.insert_local(from, local, span(5, 6));
        sources.insert_generic(generic, span(7, 8), from);
        sources.record_reference(
            span(10, 11),
            SourceSymbol::Local { owner: from, local },
            Some(from),
        );

        sources.remap_owner(from, to);

        assert_eq!(sources.definition_span(to), Some(&span(1, 4)));
        assert_eq!(sources.local_span(to, local), Some(&span(5, 6)));
        assert!(sources
            .symbol(&SourceSymbol::Generic(GenericParamId {
                owner: to,
                index: 0
            }))
            .is_some());
        assert_eq!(
            sources
                .reference_spans(&SourceSymbol::Local { owner: to, local })
                .count(),
            1
        );
    }

    #[test]
    fn semantic_source_map_records_active_scope_for_shadowed_references() {
        let owner = DefId::new(CrateId(0), LocalDefId(10));
        let local = HirLocalId(1);
        let mut sources = SemanticSourceMap::default();
        sources.insert_local_in_scope(owner, local, span(0, 1), Some(3));
        sources.record_reference_in_scope(
            span(1, 2),
            SourceSymbol::Local { owner, local },
            Some(owner),
            Some(3),
        );
        sources.record_reference_in_scope(
            span(4, 5),
            SourceSymbol::Local { owner, local },
            Some(owner),
            Some(4),
        );

        assert_eq!(sources.references()[0].scope_id, Some(3));
        assert_eq!(sources.references()[1].scope_id, Some(4));
        assert_eq!(
            sources
                .symbol(&SourceSymbol::Local { owner, local })
                .unwrap()
                .scope_id,
            Some(3)
        );
        assert_ne!(
            (
                sources.references()[0].target.clone(),
                sources.references()[0].scope_id
            ),
            (
                sources.references()[1].target.clone(),
                sources.references()[1].scope_id
            )
        );
    }
}

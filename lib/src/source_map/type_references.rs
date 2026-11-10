use std::collections::HashMap;

use crate::ast;
use crate::ids::DefId;
use crate::lexer::Span;
use crate::types::GenericParamId;

use super::{SemanticSourceMap, SourceSymbol};

impl SemanticSourceMap {
    pub(super) fn record_top_level_type_references(
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
            ast::TopLevel::NewType(_, _, _) => items
                .type_alias(owner)
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
        let params = match top_level {
            ast::TopLevel::StructDecl(item) => item.generic_params.as_slice(),
            ast::TopLevel::EnumDecl(item) => item.generic_params.as_slice(),
            ast::TopLevel::TraitDecl(item) => item.generic_params.as_slice(),
            ast::TopLevel::NewType(_, params, _) => params.as_slice(),
            _ => &[],
        };
        for param in params {
            if let Some(default) = &param.default {
                record_type_references(default, &generic_ids, resolver, self);
            }
        }
        match top_level {
            ast::TopLevel::NewType(_, _, target) => {
                record_type_references(target, &generic_ids, resolver, self);
            }
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
                        implementation
                            .name
                            .named_inner()
                            .map(type_inner_name_span)
                            .unwrap_or_else(|| implementation.name.span()),
                        SourceSymbol::Definition(trait_id),
                        None,
                    );
                    for generic in implementation.name.generics() {
                        record_type_references(generic, &generic_ids, resolver, self);
                    }
                } else {
                    record_type_references(&implementation.name, &generic_ids, resolver, self);
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
}

fn record_type_references(
    ty: &ast::ParseType,
    generic_ids: &HashMap<String, GenericParamId>,
    resolver: &crate::collect::resolver::ResolverTables,
    map: &mut SemanticSourceMap,
) {
    match ty {
        ast::ParseType::Type(inner) => {
            record_type_inner_reference(inner, generic_ids, resolver, map);
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
        ast::ParseType::Hole(_) | ast::ParseType::Unit(_) | ast::ParseType::ConstValue { .. } => {}
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

pub(super) fn type_inner_name_span(inner: &ast::ParseTypeInner) -> Span {
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
        ast::ParseType::Hole(_) | ast::ParseType::Unit(_) | ast::ParseType::ConstValue { .. } => {}
    }
}

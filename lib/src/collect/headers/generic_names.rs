use crate::ast;
use crate::collect::context::CollectContext;
use crate::types::{GenericParamId, Type};
use std::collections::HashMap;

pub(super) fn collect_generic_ids_from_type(ty: &Type, generic_params: &mut Vec<GenericParamId>) {
    crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
        if let Type::Generic(param) = nested {
            if !generic_params.contains(param) {
                generic_params.push(*param);
            }
        }
    });
}

fn is_builtin_type_name(name: &str) -> bool {
    matches!(
        name,
        "I8" | "I16"
            | "I32"
            | "I64"
            | "U8"
            | "U16"
            | "U32"
            | "U64"
            | "F32"
            | "F64"
            | "Bool"
            | "Str"
            | "Char"
            | "()"
    )
}

pub(super) fn collect_generic_names_from_parse_type<F>(
    ty: &ast::ParseType,
    generic_params: &mut Vec<String>,
    is_known_type_name: &F,
) where
    F: Fn(&str) -> bool,
{
    match ty {
        ast::ParseType::Type(inner) => {
            if inner.generics.is_empty()
                && !is_builtin_type_name(&inner.name)
                && !is_known_type_name(&inner.name)
                && !generic_params.contains(&inner.name)
            {
                generic_params.push(inner.name.clone());
            }
            for generic in &inner.generics {
                collect_generic_names_from_parse_type(generic, generic_params, is_known_type_name);
            }
        }
        ast::ParseType::Application(application) => {
            collect_generic_names_from_parse_type(
                &application.constructor,
                generic_params,
                is_known_type_name,
            );
            for arg in &application.args {
                collect_generic_names_from_parse_type(arg, generic_params, is_known_type_name);
            }
        }
        ast::ParseType::Lambda(lambda) => {
            collect_generic_names_from_parse_type(&lambda.body, generic_params, is_known_type_name);
        }
        ast::ParseType::Hole(_) => {}
        ast::ParseType::Associated { base, .. } => {
            for generic in &base.generics {
                collect_generic_names_from_parse_type(generic, generic_params, is_known_type_name);
            }
        }
        ast::ParseType::Slice(inner)
        | ast::ParseType::Reference { pointee: inner, .. }
        | ast::ParseType::Pointer(inner) => {
            collect_generic_names_from_parse_type(inner, generic_params, is_known_type_name);
        }
        ast::ParseType::Array { inner, .. } => {
            collect_generic_names_from_parse_type(inner, generic_params, is_known_type_name);
        }
        ast::ParseType::Function(args) | ast::ParseType::Tuple(args) => {
            for arg in args {
                collect_generic_names_from_parse_type(arg, generic_params, is_known_type_name);
            }
        }
        ast::ParseType::Unit(_) => {}
    }
}

pub(super) fn collect_constructor_generic_kinds<F>(
    ty: &ast::ParseType,
    kinds: &mut HashMap<String, crate::type_services::kind::Kind>,
    is_known_type_name: &F,
) where
    F: Fn(&str) -> bool,
{
    match ty {
        ast::ParseType::Application(application) => {
            if let ast::ParseType::Type(head) = application.constructor.as_ref() {
                if head.generics.is_empty()
                    && application
                        .args
                        .iter()
                        .all(|arg| matches!(arg, ast::ParseType::Hole(_)))
                    && !is_known_type_name(&head.name)
                {
                    kinds.entry(head.name.clone()).or_insert_with(|| {
                        crate::type_lowering::constructor_kind_from_arity(application.args.len())
                    });
                }
            }
            collect_constructor_generic_kinds(&application.constructor, kinds, is_known_type_name);
            for arg in &application.args {
                collect_constructor_generic_kinds(arg, kinds, is_known_type_name);
            }
        }
        ast::ParseType::Type(inner) => {
            for generic in &inner.generics {
                collect_constructor_generic_kinds(generic, kinds, is_known_type_name);
            }
        }
        ast::ParseType::Lambda(lambda) => {
            collect_constructor_generic_kinds(&lambda.body, kinds, is_known_type_name);
        }
        ast::ParseType::Associated { base, .. } => {
            for generic in &base.generics {
                collect_constructor_generic_kinds(generic, kinds, is_known_type_name);
            }
        }
        ast::ParseType::Slice(inner)
        | ast::ParseType::Reference { pointee: inner, .. }
        | ast::ParseType::Pointer(inner)
        | ast::ParseType::Array { inner, .. } => {
            collect_constructor_generic_kinds(inner, kinds, is_known_type_name);
        }
        ast::ParseType::Function(types) | ast::ParseType::Tuple(types) => {
            for ty in types {
                collect_constructor_generic_kinds(ty, kinds, is_known_type_name);
            }
        }
        ast::ParseType::Hole(_) | ast::ParseType::Unit(_) => {}
    }
}

pub(super) fn parse_type_name_span(ty: &ast::ParseType, name: &str) -> Option<crate::lexer::Span> {
    match ty {
        ast::ParseType::Type(inner) => {
            if inner.name == name {
                return Some(inner.span.clone());
            }
            inner
                .generics
                .iter()
                .find_map(|generic| parse_type_name_span(generic, name))
        }
        ast::ParseType::Application(application) => {
            parse_type_name_span(&application.constructor, name).or_else(|| {
                application
                    .args
                    .iter()
                    .find_map(|arg| parse_type_name_span(arg, name))
            })
        }
        ast::ParseType::Function(types) | ast::ParseType::Tuple(types) => {
            types.iter().find_map(|ty| parse_type_name_span(ty, name))
        }
        ast::ParseType::Lambda(lambda) => parse_type_name_span(&lambda.body, name),
        ast::ParseType::Associated { base, .. } => {
            if base.name == name {
                Some(base.span.clone())
            } else {
                base.generics
                    .iter()
                    .find_map(|generic| parse_type_name_span(generic, name))
            }
        }
        ast::ParseType::Slice(inner)
        | ast::ParseType::Array { inner, .. }
        | ast::ParseType::Reference { pointee: inner, .. }
        | ast::ParseType::Pointer(inner) => parse_type_name_span(inner, name),
        ast::ParseType::Hole(_) | ast::ParseType::Unit(_) => None,
    }
}

pub(super) fn collect_declared_generic_kinds<F>(
    ty: &ast::ParseType,
    kinds: &mut HashMap<String, crate::type_services::kind::Kind>,
    conflicts: &mut Vec<(
        String,
        crate::type_services::kind::Kind,
        crate::type_services::kind::Kind,
    )>,
    is_known_type_name: &F,
) where
    F: Fn(&str) -> bool,
{
    fn record(
        name: &str,
        kind: crate::type_services::kind::Kind,
        kinds: &mut HashMap<String, crate::type_services::kind::Kind>,
        conflicts: &mut Vec<(
            String,
            crate::type_services::kind::Kind,
            crate::type_services::kind::Kind,
        )>,
    ) {
        if let Some(declared) = kinds.get(name) {
            if declared != &kind && !conflicts.iter().any(|(conflict, _, _)| conflict == name) {
                conflicts.push((name.to_string(), declared.clone(), kind));
            }
        } else {
            kinds.insert(name.to_string(), kind);
        }
    }

    match ty {
        ast::ParseType::Application(application) => {
            if let ast::ParseType::Type(head) = application.constructor.as_ref() {
                if head.generics.is_empty()
                    && application
                        .args
                        .iter()
                        .all(|arg| matches!(arg, ast::ParseType::Hole(_)))
                    && !is_known_type_name(&head.name)
                {
                    record(
                        &head.name,
                        crate::type_lowering::constructor_kind_from_arity(application.args.len()),
                        kinds,
                        conflicts,
                    );
                }
            }
            for arg in &application.args {
                collect_declared_generic_kinds(arg, kinds, conflicts, is_known_type_name);
            }
        }
        ast::ParseType::Type(inner) => {
            if inner.generics.is_empty()
                && !is_builtin_type_name(&inner.name)
                && !is_known_type_name(&inner.name)
            {
                record(
                    &inner.name,
                    crate::type_services::kind::Kind::Type,
                    kinds,
                    conflicts,
                );
            }
            for generic in &inner.generics {
                collect_declared_generic_kinds(generic, kinds, conflicts, is_known_type_name);
            }
        }
        ast::ParseType::Lambda(lambda) => {
            collect_declared_generic_kinds(&lambda.body, kinds, conflicts, is_known_type_name);
        }
        ast::ParseType::Associated { base, .. } => {
            for generic in &base.generics {
                collect_declared_generic_kinds(generic, kinds, conflicts, is_known_type_name);
            }
        }
        ast::ParseType::Slice(inner)
        | ast::ParseType::Reference { pointee: inner, .. }
        | ast::ParseType::Pointer(inner)
        | ast::ParseType::Array { inner, .. } => {
            collect_declared_generic_kinds(inner, kinds, conflicts, is_known_type_name);
        }
        ast::ParseType::Function(types) | ast::ParseType::Tuple(types) => {
            for ty in types {
                collect_declared_generic_kinds(ty, kinds, conflicts, is_known_type_name);
            }
        }
        ast::ParseType::Hole(_) | ast::ParseType::Unit(_) => {}
    }
}

pub(super) fn is_known_nominal_type_name(context: &CollectContext, name: &str) -> bool {
    if context.structs.contains_key(name) || context.enums.contains_key(name) {
        return true;
    }

    if let Some(qualified) = context.import_aliases.get(name) {
        if context.structs.contains_key(qualified) || context.enums.contains_key(qualified) {
            return true;
        }
    }

    if let Some(id) = context.canonical_import_aliases.get(name) {
        return context
            .structs
            .values()
            .any(|structure| structure.id == *id)
            || context.enums.values().any(|enum_def| enum_def.id == *id);
    }

    false
}

use crate::ast;
pub(super) use crate::collect::context::is_known_nominal_type_name;
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
    // Const generics used as array lengths follow the type generics.
    crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
        if let Type::Array(_, crate::types::ArrayLen::Param(param)) = nested {
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
        ast::ParseType::Hole(_) | ast::ParseType::ConstValue { .. } => {}
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
        ast::ParseType::Array { inner, len } => {
            collect_generic_names_from_parse_type(inner, generic_params, is_known_type_name);
            // A length name is an implicit const generic, introduced like `T`.
            if let ast::ParseArrayLen::Param(name) = len {
                if !is_known_type_name(&name.name) && !generic_params.contains(&name.name) {
                    generic_params.push(name.name.clone());
                }
            }
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
        ast::ParseType::Hole(_) | ast::ParseType::Unit(_) | ast::ParseType::ConstValue { .. } => {}
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
        ast::ParseType::Hole(_) | ast::ParseType::Unit(_) | ast::ParseType::ConstValue { .. } => None,
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
        | ast::ParseType::Pointer(inner) => {
            collect_declared_generic_kinds(inner, kinds, conflicts, is_known_type_name);
        }
        ast::ParseType::Array { inner, len } => {
            collect_declared_generic_kinds(inner, kinds, conflicts, is_known_type_name);
            if let ast::ParseArrayLen::Param(name) = len {
                if !is_known_type_name(&name.name) {
                    record(
                        &name.name,
                        crate::type_services::kind::Kind::Usize,
                        kinds,
                        conflicts,
                    );
                }
            }
        }
        ast::ParseType::Function(types) | ast::ParseType::Tuple(types) => {
            for ty in types {
                collect_declared_generic_kinds(ty, kinds, conflicts, is_known_type_name);
            }
        }
        ast::ParseType::Hole(_) | ast::ParseType::Unit(_) | ast::ParseType::ConstValue { .. } => {}
    }
}



/// Names written as plain type arguments in a position the nominal declares as `usize`
/// (`N` in `impl Buf T, N` for `struct Buf T, N: usize`). Such names are implicit const
/// generics of the impl, not type generics.
pub(super) fn collect_usize_argument_names(
    context: &crate::collect::context::CollectContext,
    ty: &ast::ParseType,
    names: &mut std::collections::HashSet<String>,
) {
    fn record_args(
        context: &crate::collect::context::CollectContext,
        head: &str,
        args: &[ast::ParseType],
        names: &mut std::collections::HashSet<String>,
    ) {
        let params = context
            .structs
            .get(head)
            .map(|structure| &structure.generic_params)
            .or_else(|| context.enums.get(head).map(|enum_def| &enum_def.generic_params));
        if let Some(params) = params {
            for (arg, param) in args.iter().zip(params) {
                if let ast::ParseType::Type(inner) = arg {
                    if param.kind == crate::type_services::kind::Kind::Usize
                        && inner.generics.is_empty()
                    {
                        names.insert(inner.name.clone());
                    }
                }
            }
        }
        for arg in args {
            collect_usize_argument_names(context, arg, names);
        }
    }

    match ty {
        // `impl Buf T, N` carries its arguments on the type itself.
        ast::ParseType::Type(inner) => record_args(context, &inner.name, &inner.generics, names),
        ast::ParseType::Application(application) => {
            if let ast::ParseType::Type(head) = application.constructor.as_ref() {
                record_args(context, &head.name, &application.args, names);
            } else {
                for arg in &application.args {
                    collect_usize_argument_names(context, arg, names);
                }
            }
        }
        ast::ParseType::Slice(inner)
        | ast::ParseType::Array { inner, .. }
        | ast::ParseType::Reference { pointee: inner, .. }
        | ast::ParseType::Pointer(inner) => collect_usize_argument_names(context, inner, names),
        ast::ParseType::Tuple(types) | ast::ParseType::Function(types) => {
            for ty in types {
                collect_usize_argument_names(context, ty, names);
            }
        }
        _ => {}
    }
}

use super::CollectContext;
use crate::ast;

pub(crate) fn seg_name(seg: &ast::IdentOrType) -> Option<String> {
    match seg {
        ast::IdentOrType::Ident(ident) => Some(ident.name.clone()),
        ast::IdentOrType::Type(ast::ParseType::Type(inner)) => Some(inner.name.clone()),
        _ => None,
    }
}

pub(crate) fn path_names(path: &[ast::IdentOrType]) -> Vec<String> {
    path.iter().filter_map(seg_name).collect()
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

pub(crate) fn is_known_nominal_type_name(context: &CollectContext, name: &str) -> bool {
    if context.structs.contains_key(name) || context.enums.contains_key(name) {
        return true;
    }

    if let Some(qualified) = context.names.import_alias(name) {
        if context.structs.contains_key(qualified) || context.enums.contains_key(qualified) {
            return true;
        }
    }

    if let Some(id) = context.names.canonical_alias(name) {
        return context.structs.values().any(|structure| structure.id == id)
            || context.enums.values().any(|enum_def| enum_def.id == id);
    }

    false
}

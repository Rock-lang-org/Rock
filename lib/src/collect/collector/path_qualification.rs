use crate::ast;
use crate::collect::context::CollectContext;

pub(super) fn local_item_lookup_name(local_prefix: Option<&str>, name: &str) -> String {
    local_prefix
        .map(|prefix| format!("{}::{}", prefix, name))
        .unwrap_or_else(|| name.to_string())
}

pub(super) fn qualify_export_source(
    context: &CollectContext,
    prefix: &str,
    source: &str,
) -> String {
    let first_segment = source.split("::").next().unwrap_or(source);
    let is_absolute = context.modules.is_loaded_name(first_segment);

    if is_absolute {
        source.to_string()
    } else {
        format!("{}::{}", prefix, source)
    }
}

pub(super) fn qualify_local_import_target(
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
    let is_absolute = context.modules.is_loaded_name(first_segment);

    if is_absolute {
        qualified_name.to_string()
    } else {
        format!("{}::{}", prefix, qualified_name)
    }
}

pub(super) fn qualify_local_path(context: &CollectContext, path: &ast::Path) -> Option<ast::Path> {
    let prefix = context.current_module_prefix()?;
    qualify_local_path_with_prefix(context, path, &prefix)
}

pub(super) fn qualify_local_path_with_prefix(
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

pub(super) fn qualify_local_module_path(
    context: &CollectContext,
    module_path: &[String],
) -> Option<Vec<String>> {
    let prefix = context.current_module_prefix()?;
    qualify_local_module_path_with_prefix(context, module_path, &prefix)
}

pub(super) fn qualify_local_module_path_with_prefix(
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

pub(super) fn qualify_local_segments_with_prefix(
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

pub(super) fn local_path_segment_is_absolute(
    context: &CollectContext,
    first_segment: &str,
) -> bool {
    if context.current_crate_name.as_deref() == Some(first_segment) {
        return true;
    }

    if context.modules.is_loaded_name(first_segment) {
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

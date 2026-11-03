use super::Declarations;
use crate::ast;
use crate::lower::ResolveError;

pub(super) fn validate_supported_current_crate_ids(
    decls: &Declarations,
) -> Result<(), Vec<ResolveError>> {
    fn push_missing(errors: &mut Vec<ResolveError>, kind: &str, name: &str, id: crate::ids::DefId) {
        if id.crate_id == crate::ids::CrateId(u32::MAX) {
            errors.push(ResolveError::non_source(format!(
                "missing canonical {kind} identity for {name}"
            )));
        }
    }

    let mut errors = Vec::new();

    for function in decls.items.functions().values() {
        push_missing(&mut errors, "function", &function.name, function.id);
        for generic in &function.generic_params {
            push_missing(
                &mut errors,
                "function generic owner",
                &function.name,
                generic.id.owner,
            );
        }
    }
    for sig in decls.items.function_sigs().values() {
        push_missing(&mut errors, "function signature", &sig.name, sig.id);
        for generic in &sig.generic_params {
            push_missing(
                &mut errors,
                "signature generic owner",
                &sig.name,
                generic.id.owner,
            );
        }
    }
    for structure in decls.items.structs().values() {
        push_missing(&mut errors, "struct", &structure.name, structure.id);
    }
    for enumeration in decls.items.enums().values() {
        push_missing(&mut errors, "enum", &enumeration.name, enumeration.id);
    }
    for trait_def in decls.items.traits().values() {
        push_missing(&mut errors, "trait", &trait_def.name, trait_def.id);
        for (method_name, method) in &trait_def.methods {
            push_missing(
                &mut errors,
                "trait method",
                &format!("{}.{}", trait_def.name, method_name),
                method.id,
            );
        }
        for (signature_name, signature) in &trait_def.signatures {
            push_missing(
                &mut errors,
                "trait signature",
                &format!("{}.{}", trait_def.name, signature_name),
                signature.id,
            );
        }
    }
    for imp in decls.items.impls().values() {
        push_missing(&mut errors, "impl", &imp.type_name, imp.id);
        for (method_name, method) in &imp.methods {
            push_missing(
                &mut errors,
                "impl method",
                &format!("{}.{}", imp.type_name, method_name),
                method.id,
            );
        }
    }
    for ext in decls.items.externs().values() {
        push_missing(&mut errors, "extern", &ext.name, ext.id);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub(super) fn supertrait_operation_span(
    module: &ast::Module,
    message: &str,
    source_modules: &crate::collect::item_index::SourceModuleMap,
    prefix: &[String],
) -> Option<crate::lexer::Span> {
    let owner = message.split('\'').nth(1).or_else(|| {
        message
            .strip_prefix("supertrait cycle detected: ")
            .and_then(|path| path.split(" -> ").next())
    });
    for top_level in &module.top_levels {
        if let ast::TopLevel::TraitDecl(trait_decl) = top_level {
            if owner.is_some_and(|owner| {
                owner == trait_decl.name.name
                    || owner.rsplit("::").next() == Some(trait_decl.name.name.as_str())
            }) {
                if let Some(span) = trait_decl
                    .where_clauses
                    .iter()
                    .find_map(|clause| clause.trait_bound.as_ref().map(ast::ParseType::span))
                {
                    return Some(span);
                }
                return Some(trait_decl.name.span.clone());
            }
        }
    }

    for top_level in &module.top_levels {
        let ast::TopLevel::Module(ast::ModuleDecl(inline)) = top_level else {
            continue;
        };
        let Some(name) = inline.name.as_ref() else {
            continue;
        };
        let mut path = prefix.to_vec();
        path.push(name.name.clone());
        if let Some(span) = supertrait_operation_span(inline, message, source_modules, &path) {
            return Some(span);
        }
    }
    for top_level in &module.top_levels {
        let ast::TopLevel::Mod(ident, _) = top_level else {
            continue;
        };
        let mut path = prefix.to_vec();
        path.push(ident.name.clone());
        let Some(child) = source_modules.get(&path) else {
            continue;
        };
        if let Some(span) = supertrait_operation_span(child, message, source_modules, &path) {
            return Some(span);
        }
    }
    None
}

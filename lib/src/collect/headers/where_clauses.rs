use crate::ast;
use crate::collect::context::CollectContext;
use crate::hir::*;
use crate::types::{GenericParamId, TraitBound, Type};

fn simple_trait_bound(
    trait_bound: &ast::ParseType,
) -> Option<(&ast::ParseTypeInner, &[ast::ParseType])> {
    match trait_bound {
        ast::ParseType::Type(inner) => Some((inner, &inner.generics)),
        ast::ParseType::Application(application) => {
            let ast::ParseType::Type(inner) = application.constructor.as_ref() else {
                return None;
            };
            if !inner.generics.is_empty() {
                return None;
            }
            Some((inner, application.args.as_slice()))
        }
        _ => None,
    }
}

pub(super) fn lower_where_clauses(
    context: &mut CollectContext,
    clauses: &[ast::WhereClause],
) -> crate::hir::HirGenericBounds {
    let mut bounds = crate::hir::HirGenericBounds::new();
    for clause in clauses {
        let Some(trait_bound) = clause.trait_bound.as_ref() else {
            continue;
        };
        let Some((trait_bound, type_args)) = simple_trait_bound(trait_bound) else {
            context.push_error_with_span(
                "unsupported trait bound shape in where clause".to_string(),
                clause.trait_bound.as_ref().unwrap().span(),
            );
            continue;
        };
        let Some(trait_id) = context
            .trait_by_name(&trait_bound.name)
            .map(|trait_def| trait_def.id)
            .or_else(|| {
                let suffix = format!("::{}", trait_bound.name);
                let mut candidates =
                    context
                        .canonical_names_by_id
                        .iter()
                        .filter_map(|(id, name)| {
                            (name == &trait_bound.name || name.ends_with(&suffix)).then_some(*id)
                        });
                let candidate = candidates.next()?;
                candidates.next().is_none().then_some(candidate)
            })
        else {
            context.push_error_with_span(
                format!("unknown trait '{}' in where clause", trait_bound.name),
                clause.trait_bound.as_ref().unwrap().span(),
            );
            continue;
        };

        let subject = match &clause.subject {
            ast::ParseType::Type(inner)
                if !inner.generics.is_empty()
                    && inner
                        .generics
                        .iter()
                        .all(|arg| matches!(arg, ast::ParseType::Hole(_))) =>
            {
                context.lower_parse_type(&ast::ParseType::Type(ast::ParseTypeInner {
                    name: inner.name.clone(),
                    generics: Vec::new(),
                    span: inner.span.clone(),
                }))
            }
            subject => context.lower_parse_type(subject),
        };
        let argument_kinds = context
            .traits
            .values()
            .find(|item| item.id == trait_id)
            .map(|item| {
                item.generic_params
                    .iter()
                    .map(|param| param.kind.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut type_args = type_args
            .iter()
            .enumerate()
            .map(|(index, generic)| {
                crate::type_lowering::TypeLowerer::lower_type_argument(
                    context,
                    generic,
                    argument_kinds
                        .get(index)
                        .unwrap_or(&crate::type_services::kind::Kind::Type),
                )
            })
            .collect::<Vec<_>>();
        if let Some(trait_def) = context.traits.values().find(|item| item.id == trait_id) {
            match crate::type_lowering::complete_generic_args(
                &trait_def.generic_params,
                type_args.clone(),
                Some(&subject),
            ) {
                Ok(completed) => type_args = completed,
                Err(message) => context
                    .push_error_with_span(message, clause.trait_bound.as_ref().unwrap().span()),
            }
        }
        bounds.predicates.push(crate::types::Predicate::Trait {
            subject: subject.clone(),
            trait_id,
            args: type_args.clone(),
        });
        if let Type::Generic(param) = subject {
            bounds.entry(param).or_default().push(TraitBound {
                trait_id,
                type_args,
            });
        }
    }
    bounds
}

pub(super) fn remap_generic_param_ids_in_type(
    ty: &mut Type,
    generic_ids: &[GenericParamId],
    new_owner: crate::ids::DefId,
) {
    crate::type_services::visit::remap_generic_params_in_place(ty, &mut |mut param| {
        if let Some(index) = generic_ids.iter().position(|id| *id == param) {
            param.owner = new_owner;
            param.index = index as u32;
        }
        param
    });
}

pub(super) fn remap_generic_bounds_owner(
    bounds: &HirGenericBounds,
    generic_ids: &[GenericParamId],
    new_owner: crate::ids::DefId,
) -> HirGenericBounds {
    let mut remapped: HirGenericBounds = bounds
        .iter()
        .map(|(param, trait_bounds)| {
            let mut remapped_param = *param;
            if let Some(index) = generic_ids.iter().position(|id| id == param) {
                remapped_param.owner = new_owner;
                remapped_param.index = index as u32;
            }

            let mut trait_bounds = trait_bounds.clone();
            for bound in &mut trait_bounds {
                for ty in &mut bound.type_args {
                    remap_generic_param_ids_in_type(ty, generic_ids, new_owner);
                }
            }

            (remapped_param, trait_bounds)
        })
        .collect();
    remapped.predicates = bounds.predicates.clone();
    for predicate in &mut remapped.predicates {
        match predicate {
            crate::types::Predicate::Trait { subject, args, .. } => {
                remap_generic_param_ids_in_type(subject, generic_ids, new_owner);
                for arg in args {
                    remap_generic_param_ids_in_type(arg, generic_ids, new_owner);
                }
            }
        }
    }
    remapped
}

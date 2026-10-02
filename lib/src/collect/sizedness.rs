use std::collections::HashMap;

use crate::hir::{HirGenericBounds, HirTrait};
use crate::ids::DefId;
use crate::lexer::Span;
use crate::type_services::layout::{Sizedness, TypeLayout};
use crate::types::{GenericParamDecl, TraitBound, Type};

pub(crate) struct SizedStorageUse {
    pub ty: Type,
    pub generic_params: Vec<GenericParamDecl>,
    pub bounds: HirGenericBounds,
    pub span: Span,
}

pub(crate) fn storage_is_unsized(
    ty: &Type,
    bounds: &HirGenericBounds,
    traits: &HashMap<DefId, HirTrait>,
    sized: Option<DefId>,
) -> bool {
    let status = TypeLayout::sizedness(ty, &|parameter| {
        if !bounds.relaxed_sized.contains(&parameter) {
            return Sizedness::Sized;
        }
        let assumptions = bounds.get(&parameter).cloned().unwrap_or_default();
        let implied = crate::traits::evidence::implied_trait_bounds(
            traits,
            &Type::Generic(parameter),
            &assumptions,
        );
        if sized.is_some_and(|id| {
            implied.contains(&TraitBound {
                trait_id: id,
                type_args: Vec::new(),
            })
        }) {
            Sizedness::Sized
        } else {
            Sizedness::Unknown
        }
    });
    if status == Sizedness::Unsized {
        return true;
    }
    match ty {
        Type::Generic(parameter) => {
            bounds.relaxed_sized.contains(parameter) && status != Sizedness::Sized
        }
        Type::Array(element, _) => storage_is_unsized(element, bounds, traits, sized),
        Type::Tuple(elements) => elements
            .iter()
            .any(|element| storage_is_unsized(element, bounds, traits, sized)),
        _ => false,
    }
}

pub(crate) fn validate(context: &mut super::context::CollectContext) {
    use crate::type_lowering::TypeLoweringContext;
    let mut environment = crate::type_services::normalize::TypeNormalizationEnv::new();
    context.populate_type_normalization_env(&mut environment);
    let traits = context
        .traits
        .values()
        .map(|definition| (definition.id, definition.clone()))
        .collect();
    let sized = context
        .language_items
        .sized
        .as_ref()
        .map(|item| item.trait_id);
    let nominal_parameters: HashMap<_, _> = context
        .structs
        .values()
        .map(|definition| (definition.id, definition.generic_params.clone()))
        .chain(
            context
                .enums
                .values()
                .map(|definition| (definition.id, definition.generic_params.clone())),
        )
        .collect();
    for mut usage in std::mem::take(&mut context.sized_storage_uses) {
        // Function headers consume and remap signature declarations before this
        // deferred check runs; keep the kinds belonging to the checked types.
        for parameter in &usage.generic_params {
            environment.register_generic_kind(parameter.id, parameter.kind.clone());
        }
        match crate::type_services::normalize::TypeNormalizer::new(&environment)
            .normalize(&usage.ty)
        {
            Ok(ty) => usage.ty = ty,
            Err(error) => {
                context.push_error_with_span(error.to_string(), usage.span);
                continue;
            }
        }
        let mut invalid_argument = false;
        crate::type_services::visit::visit_type(&usage.ty, &mut |ty: &Type| {
            if let Type::Struct { id, args } | Type::Enum { id, args } = ty {
                if let Some(parameters) = nominal_parameters.get(id) {
                    invalid_argument |= parameters.iter().zip(args).any(|(parameter, argument)| {
                        !parameter.maybe_unsized
                            && storage_is_unsized(argument, &usage.bounds, &traits, sized)
                    });
                }
            }
        });
        if invalid_argument || storage_is_unsized(&usage.ty, &usage.bounds, &traits, sized) {
            context.push_error_with_span("by-value storage requires a sized type; use a pointer, reference, or sized owner for a ?Sized parameter".into(), usage.span);
        }
    }
}

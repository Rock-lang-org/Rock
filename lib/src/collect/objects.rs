use std::collections::HashMap;

use crate::hir::{
    HirFunction, HirFunctionSig, HirGenericBounds, HirImplReceiverPattern, HirVariantFields,
};
use crate::lexer::Span;
use crate::type_lowering::TypeLoweringContext;
use crate::type_services::normalize::TypeNormalizationEnv;
use crate::types::{Predicate, Type};

use super::context::CollectContext;

pub(crate) struct ObjectTypeUse {
    pub ty: Type,
    pub span: Span,
}

pub(crate) struct ObjectQualifierUse {
    pub placeholder: Type,
    pub object: crate::types::ObjectType,
    pub bindings: Vec<crate::type_lowering::ObjectBindingSyntax>,
    pub parameters: Vec<crate::types::GenericParamId>,
    pub groups: Vec<Vec<crate::type_services::kind::Kind>>,
    pub kind: crate::type_services::kind::Kind,
    pub span: Span,
}

struct ResolveQualifiers(HashMap<Type, Type>);

impl crate::type_services::visit::TypeFolder for ResolveQualifiers {
    fn fold_type(&mut self, ty: Type) -> Type {
        if let Some(resolved) = self.0.get(&ty) {
            return resolved.clone();
        }
        crate::type_services::visit::fold_type_children(ty, self)
    }
}

fn predicates(predicates: &mut [Predicate], visit: &mut impl FnMut(&mut Type)) {
    for predicate in predicates {
        let Predicate::Trait { subject, args, .. } = predicate;
        visit(subject);
        for arg in args {
            visit(arg);
        }
    }
}

fn bounds(bounds: &mut HirGenericBounds, visit: &mut impl FnMut(&mut Type)) {
    for bounds in bounds.values_mut() {
        for bound in bounds {
            for arg in &mut bound.type_args {
                visit(arg);
            }
        }
    }
    predicates(&mut bounds.predicates, visit);
}

fn function(function: &mut HirFunction, visit: &mut impl FnMut(&mut Type)) {
    for param in &mut function.params {
        visit(&mut param.ty);
    }
    visit(&mut function.ret_type);
    visit(&mut function.body.ty);
    bounds(&mut function.generic_bounds, visit);
}

fn signature(signature: &mut HirFunctionSig, visit: &mut impl FnMut(&mut Type)) {
    for param in &mut signature.params {
        visit(param);
    }
    visit(&mut signature.ret);
    bounds(&mut signature.generic_bounds, visit);
}

/// Declarations record source-owned uses first; incomplete forward trait headers
/// must not freeze a partial object view into a signature.
pub(crate) fn admit_collected_objects(context: &mut CollectContext) {
    if context.object_type_uses.is_empty() && context.object_qualifier_uses.is_empty() {
        return;
    }
    let mut environment = TypeNormalizationEnv::new();
    context.populate_type_normalization_env(&mut environment);
    let traits: HashMap<_, _> = context
        .traits
        .values()
        .map(|definition| (definition.id, definition.clone()))
        .collect();
    let mut resolved = ResolveQualifiers(HashMap::new());
    for usage in std::mem::take(&mut context.object_qualifier_uses) {
        let ty = match crate::type_lowering::resolve_object_bindings(
            usage.object,
            usage.bindings,
            &traits,
            usage.span.clone(),
        ) {
            Ok(mut ty) => {
                crate::type_services::visit::fold_type_in_place(&mut ty, &mut resolved);
                if !usage.parameters.is_empty() {
                    let depth = (usage.groups.len() - 1) as u32;
                    let substitution = usage
                        .parameters
                        .iter()
                        .enumerate()
                        .map(|(index, parameter)| {
                            (
                                *parameter,
                                Type::BoundVar {
                                    depth,
                                    index: index as u32,
                                    kind: usage.groups[0][index].clone(),
                                },
                            )
                        })
                        .collect();
                    ty = ty.substitute_generics(&substitution);
                }
                for group in usage.groups.iter().rev() {
                    ty = Type::Lambda {
                        params: group.clone(),
                        body: Box::new(ty),
                    };
                }
                context.object_type_uses.push(ObjectTypeUse {
                    ty: ty.clone(),
                    span: usage.span,
                });
                ty
            }
            Err((message, span)) => {
                context.push_error_with_span(message, span);
                Type::Error
            }
        };
        resolved.0.insert(usage.placeholder, ty);
    }
    for alias in context.type_aliases.values_mut() {
        crate::type_services::visit::fold_type_in_place(&mut alias.ty, &mut resolved);
    }
    crate::type_lowering::register_type_aliases(&mut environment, context.type_aliases.values());
    for usage in &mut context.sized_storage_uses {
        crate::type_services::visit::fold_type_in_place(&mut usage.ty, &mut resolved);
    }
    let uses = std::mem::take(&mut context.object_type_uses);
    let mut invalid = false;
    for mut usage in uses {
        crate::type_services::visit::fold_type_in_place(&mut usage.ty, &mut resolved);
        match crate::type_services::normalize::TypeNormalizer::new(&environment)
            .normalize(&usage.ty)
        {
            Ok(ty) => usage.ty = ty,
            Err(error) => {
                context.push_error_with_span(error.to_string(), usage.span);
                invalid = true;
                continue;
            }
        }
        if let Err(error) =
            crate::traits::objects::admit_type_objects(&usage.ty, &traits, &environment)
        {
            context.push_error_with_span(error.to_string(), usage.span);
            invalid = true;
        }
    }
    if invalid {
        return;
    }
    let mut derived_errors = Vec::new();
    let mut admit = |ty: &mut Type| {
        crate::type_services::visit::fold_type_in_place(ty, &mut resolved);
        match crate::type_services::normalize::TypeNormalizer::new(&environment).normalize(ty) {
            Ok(normalized) => *ty = normalized,
            Err(error) => {
                derived_errors.push(error.to_string());
                return;
            }
        }
        if !crate::type_services::visit::type_any(ty, |ty| matches!(ty, Type::Object(_))) {
            return;
        }
        match crate::traits::objects::admit_type_objects(ty, &traits, &environment) {
            Ok(admitted) => *ty = admitted,
            Err(error) => {
                derived_errors.push(format!("invalid derived object declaration: {error}"))
            }
        }
    };
    for declaration in context.functions.values_mut() {
        function(declaration, &mut admit);
    }
    for declaration in context.function_sigs.values_mut() {
        signature(declaration, &mut admit);
    }
    for declaration in context.structs.values_mut() {
        for field in &mut declaration.fields {
            admit(&mut field.ty);
        }
    }
    for declaration in context.enums.values_mut() {
        for variant in &mut declaration.variants {
            match &mut variant.fields {
                HirVariantFields::Named(fields) => {
                    for field in fields {
                        admit(&mut field.ty);
                    }
                }
                HirVariantFields::Positional(fields) => {
                    for ty in fields {
                        admit(ty);
                    }
                }
                HirVariantFields::Unit => {}
            }
        }
    }
    for declaration in context.traits.values_mut() {
        predicates(&mut declaration.predicates, &mut admit);
        for method in declaration.methods.values_mut() {
            function(method, &mut admit);
        }
        for method in declaration.signatures.values_mut() {
            signature(method, &mut admit);
        }
    }
    for declaration in &mut context.impls {
        match &mut declaration.receiver_pattern {
            HirImplReceiverPattern::Exact(ty) | HirImplReceiverPattern::Constructor(ty) => {
                admit(ty)
            }
            HirImplReceiverPattern::SliceFamily { element } => admit(element),
        }
        for arg in &mut declaration.trait_arg_types {
            admit(arg);
        }
        for binding in &mut declaration.associated_types {
            admit(&mut binding.ty);
        }
        bounds(&mut declaration.bounds, &mut admit);
        for method in declaration.methods.values_mut() {
            function(method, &mut admit);
        }
    }
    for declaration in &mut context.externs {
        for param in &mut declaration.params {
            admit(param);
        }
        admit(&mut declaration.ret);
    }
    for declaration in context.type_aliases.values_mut() {
        admit(&mut declaration.ty);
    }
    for error in derived_errors {
        context.push_error(error);
    }
}

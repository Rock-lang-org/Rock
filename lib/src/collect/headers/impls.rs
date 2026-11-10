use super::functions::{
    build_function_header_with_id, build_function_header_with_sig, build_function_sig_with_id,
};
use super::generic_names::{
    collect_declared_generic_kinds, collect_generic_names_from_parse_type, collect_usize_argument_names,
    is_known_nominal_type_name,
};
use super::where_clauses::lower_where_clauses;
use crate::ast;
use crate::collect::context::CollectContext;
use crate::hir::*;
use crate::ids::{AssocTypeId, DefId};
use crate::type_lowering::TypeLoweringContext;
use crate::types::{GenericParamDecl, GenericParamId, TraitBound, Type};
use std::collections::{HashMap, HashSet};

fn specialize_trait_signature_for_impl(
    context: &mut CollectContext,
    signature: &HirFunctionSig,
    trait_def: &HirTrait,
    trait_args: &[Type],
    target: &Type,
) -> HirFunctionSig {
    struct Substituter<'a> {
        substitutions: &'a HashMap<GenericParamId, Type>,
    }

    impl crate::type_services::visit::TypeFolder for Substituter<'_> {
        fn fold_type(&mut self, ty: Type) -> Type {
            match ty {
                Type::Generic(param) => self
                    .substitutions
                    .get(&param)
                    .cloned()
                    .unwrap_or(Type::Generic(param)),
                other => crate::type_services::visit::fold_type_children(other, self),
            }
        }
    }

    let mut substitutions = trait_def
        .generic_params
        .iter()
        .zip(trait_args)
        .map(|(param, arg)| (param.id, arg.clone()))
        .collect::<HashMap<_, _>>();
    if let Some(trait_target) = &trait_def.target {
        substitutions.insert(trait_target.id, target.clone());
    }
    let substitute = |ty: &Type| {
        crate::type_services::visit::fold_type(
            ty.clone(),
            &mut Substituter {
                substitutions: &substitutions,
            },
        )
    };

    let mut specialized = signature.clone();
    specialized.params = signature.params.iter().map(substitute).collect();
    specialized.ret = substitute(&signature.ret);
    for bounds in specialized.generic_bounds.values_mut() {
        for bound in bounds {
            for arg in &mut bound.type_args {
                *arg = substitute(arg);
            }
        }
    }
    for predicate in &mut specialized.generic_bounds.predicates {
        let crate::types::Predicate::Trait { subject, args, .. } = predicate;
        *subject = substitute(subject);
        for arg in args {
            *arg = substitute(arg);
        }
    }
    let mut env = crate::type_services::normalize::TypeNormalizationEnv::new();
    context.populate_type_normalization_env(&mut env);
    let normalize = |ty: &Type| {
        crate::type_services::normalize::TypeNormalizer::new(&env)
            .normalize(ty)
            .unwrap_or_else(|_| ty.clone())
    };
    specialized.params = specialized.params.iter().map(normalize).collect();
    specialized.ret = normalize(&specialized.ret);
    for bounds in specialized.generic_bounds.values_mut() {
        for bound in bounds {
            for arg in &mut bound.type_args {
                *arg = normalize(arg);
            }
        }
    }
    for predicate in &mut specialized.generic_bounds.predicates {
        let crate::types::Predicate::Trait { subject, args, .. } = predicate;
        *subject = normalize(subject);
        for arg in args {
            *arg = normalize(arg);
        }
    }
    specialized
}

pub(crate) fn build_impl_with_id(
    context: &mut CollectContext,
    imp: &ast::Impl,
    id: DefId,
    method_ids: &HashMap<String, DefId>,
) -> HirImpl {
    let mut impl_generic_params: Vec<String> = Vec::new();
    if let Some(for_type) = imp.for_.as_ref() {
        for generic in for_type.generics() {
            collect_generic_names_from_parse_type(generic, &mut impl_generic_params, &|name| {
                is_known_nominal_type_name(context, name)
            });
        }
        if !matches!(for_type, ast::ParseType::Type(_)) {
            collect_generic_names_from_parse_type(for_type, &mut impl_generic_params, &|name| {
                is_known_nominal_type_name(context, name)
            });
        }
    } else {
        collect_generic_names_from_parse_type(&imp.name, &mut impl_generic_params, &|name| {
            is_known_nominal_type_name(context, name)
        });
    }
    for generic in imp.name.generics() {
        collect_generic_names_from_parse_type(generic, &mut impl_generic_params, &|name| {
            is_known_nominal_type_name(context, name)
        });
    }
    for clause in &imp.where_clauses {
        collect_generic_names_from_parse_type(&clause.subject, &mut impl_generic_params, &|name| {
            is_known_nominal_type_name(context, name)
        });
    }
    let mut impl_generic_kinds = HashMap::new();
    let mut impl_generic_kind_conflicts = Vec::new();
    if let Some(receiver) = imp.for_.as_ref() {
        collect_declared_generic_kinds(
            receiver,
            &mut impl_generic_kinds,
            &mut impl_generic_kind_conflicts,
            &|name| is_known_nominal_type_name(context, name),
        );
    }
    if imp.for_.is_none() {
        collect_declared_generic_kinds(
            &imp.name,
            &mut impl_generic_kinds,
            &mut impl_generic_kind_conflicts,
            &|name| is_known_nominal_type_name(context, name),
        );
    }
    for generic in imp.name.generics() {
        collect_declared_generic_kinds(
            generic,
            &mut impl_generic_kinds,
            &mut impl_generic_kind_conflicts,
            &|name| is_known_nominal_type_name(context, name),
        );
    }
    for clause in &imp.where_clauses {
        collect_declared_generic_kinds(
            &clause.subject,
            &mut impl_generic_kinds,
            &mut impl_generic_kind_conflicts,
            &|name| is_known_nominal_type_name(context, name),
        );
    }
    let mut usize_arguments = HashSet::new();
    if let Some(receiver) = imp.for_.as_ref() {
        collect_usize_argument_names(context, receiver, &mut usize_arguments);
    }
    collect_usize_argument_names(context, &imp.name, &mut usize_arguments);
    for name in &usize_arguments {
        impl_generic_kinds.insert(name.clone(), crate::type_services::kind::Kind::Usize);
    }
    // A name used both as a nominal's `usize` argument and as an array length is consistent.
    impl_generic_kind_conflicts.retain(|(name, _, _)| !usize_arguments.contains(name));
    for (name, declared, required) in impl_generic_kind_conflicts {
        context.push_error_with_span(format!(
            "generic parameter '{name}' was declared with kind {declared}, but a later binder requires {required}",
        ), imp.name.span());
    }
    let generic_kinds = impl_generic_params
        .iter()
        .map(|name| {
            impl_generic_kinds
                .get(name)
                .cloned()
                .unwrap_or(crate::type_services::kind::Kind::Type)
        })
        .collect();
    let generic_spans = vec![imp.name.span(); impl_generic_params.len()];
    let previous_generics = context.push_generic_context_with_kinds_at(
        id,
        impl_generic_params,
        generic_kinds,
        &generic_spans,
    );

    let (type_name, type_generics, mut receiver_pattern, trait_name) = if imp.for_.is_some() {
        let (type_name, type_generics, receiver_pattern) = context.impl_type_info(imp);
        (
            type_name,
            type_generics,
            receiver_pattern,
            Some(imp.name.type_name()),
        )
    } else {
        let (type_name, type_generics, receiver_pattern) = context.impl_type_info(imp);
        (type_name, type_generics, receiver_pattern, None)
    };

    let trait_generic_names: Vec<String> = imp
        .name
        .generics()
        .iter()
        .filter_map(|g| {
            if let ast::ParseType::Type(inner) = g {
                Some(inner.name.clone())
            } else {
                None
            }
        })
        .collect();
    let trait_def = trait_name
        .as_deref()
        .and_then(|name| context.trait_by_name(name))
        .cloned();
    let mut trait_arg_types: Vec<Type> = imp
        .name
        .generics()
        .iter()
        .enumerate()
        .map(|(index, generic)| {
            let expected = trait_def
                .as_ref()
                .and_then(|item| item.generic_params.get(index))
                .map(|param| param.kind.clone())
                .unwrap_or(crate::type_services::kind::Kind::Type);
            let ty = crate::type_lowering::TypeLowerer::lower_type_argument(
                context, generic, &expected,
            );
            // A trait argument must have the kind its parameter declares.
            if trait_def.is_some() && !matches!(ty, Type::Error) {
                if let Ok(actual) = crate::type_lowering::TypeLowerer::kind_of(context, &ty) {
                    if actual != expected {
                        context.push_error_with_span(
                            crate::type_services::normalize::NormalizeError::KindMismatch {
                                expected,
                                actual,
                            }
                            .to_string(),
                            generic.span(),
                        );
                    }
                }
            }
            ty
        })
        .collect();
    let trait_id = trait_def.as_ref().map(|trait_def| trait_def.id);
    let impl_target_ty = match &receiver_pattern {
        crate::hir::HirImplReceiverPattern::Exact(ty)
        | crate::hir::HirImplReceiverPattern::Constructor(ty) => ty.clone(),
        crate::hir::HirImplReceiverPattern::SliceFamily { element } => {
            Type::Slice(Box::new(element.clone()))
        }
    };
    if let Some(trait_def) = trait_def.as_ref() {
        match crate::type_lowering::complete_generic_args(
            &trait_def.generic_params,
            trait_arg_types.clone(),
            Some(&impl_target_ty),
        ) {
            Ok(completed) => trait_arg_types = completed,
            Err(message) => context.push_error_with_span(message, imp.name.span()),
        }
        let target_kind = trait_def
            .target
            .as_ref()
            .map(|target| target.kind.clone())
            .unwrap_or(crate::type_services::kind::Kind::Type);
        match crate::type_lowering::TypeLowerer::kind_of(context, &impl_target_ty) {
            Ok(actual_kind) if actual_kind != target_kind => context.push_error_with_span(
                format!(
                    "impl target has kind {actual_kind}, but trait '{}' requires {target_kind}",
                    trait_def.name
                ),
                imp.for_
                    .as_ref()
                    .map(|ty| ty.span())
                    .unwrap_or_else(|| imp.name.span()),
            ),
            Err(error) => context.push_error_with_span(
                error,
                imp.for_
                    .as_ref()
                    .map(|ty| ty.span())
                    .unwrap_or_else(|| imp.name.span()),
            ),
            _ => {}
        }
        if !matches!(target_kind, crate::type_services::kind::Kind::Type) {
            receiver_pattern =
                crate::hir::HirImplReceiverPattern::Constructor(impl_target_ty.clone());
        }
    }
    let trait_associated_types = trait_def
        .as_ref()
        .map(|trait_def| trait_def.associated_types.clone())
        .unwrap_or_default();

    let mut bounds = lower_where_clauses(context, &imp.where_clauses);
    if let Some(sized_trait_id) = context
        .language_items
        .sized
        .as_ref()
        .map(|items| items.trait_id)
    {
        let mut receiver_generic_ids = HashSet::new();
        match &receiver_pattern {
            crate::hir::HirImplReceiverPattern::Exact(ty) => {
                ty.collect_generic_params(&mut receiver_generic_ids)
            }
            crate::hir::HirImplReceiverPattern::SliceFamily { element } => {
                element.collect_generic_params(&mut receiver_generic_ids)
            }
            crate::hir::HirImplReceiverPattern::Constructor(ty) => {
                ty.collect_generic_params(&mut receiver_generic_ids)
            }
        }
        for (index, _) in type_generics.iter().enumerate() {
            if context.generics.kind_at(index)
                != Some(&crate::type_services::kind::Kind::Type)
            {
                continue;
            }
            let type_param_id = GenericParamId {
                owner: id,
                index: index as u32,
            };
            if !receiver_generic_ids.contains(&type_param_id) {
                continue;
            }
            let param_bounds = bounds.entry(type_param_id).or_default();
            if !param_bounds
                .iter()
                .any(|bound| bound.trait_id == sized_trait_id && bound.type_args.is_empty())
            {
                param_bounds.push(TraitBound {
                    trait_id: sized_trait_id,
                    type_args: Vec::new(),
                });
            }
        }
    }

    let mut methods = HashMap::new();
    for (ident, fd) in &imp.methods {
        let method_id = *method_ids
            .get(&ident.name)
            .expect("impl method IDs must be allocated before header building");
        let trait_signature = trait_def
            .as_ref()
            .and_then(|trait_def| trait_def.signatures.get(&ident.name))
            .filter(|signature| {
                !signature.generic_bounds.is_empty()
                    || !signature.generic_bounds.predicates.is_empty()
            })
            .map(|signature| {
                specialize_trait_signature_for_impl(
                    context,
                    signature,
                    trait_def.as_ref().expect("trait signature requires trait"),
                    &trait_arg_types,
                    &impl_target_ty,
                )
            });
        let explicit_signature = imp
            .signatures
            .iter()
            .find(|(name, _)| name.name == ident.name)
            .map(|(_, signature)| build_function_sig_with_id(context, signature, method_id));
        let mut func = if let Some(signature) = explicit_signature.or(trait_signature) {
            build_function_header_with_sig(context, fd, &signature, method_id)
        } else {
            build_function_header_with_id(context, fd, method_id)
        };
        // Receiver identity is already known at collection time, even when
        // this method's body is lowered after its callers in another module.
        if imp.for_.is_none() && imp.signatures.contains_key(ident) {
            if let (Some(receiver_mode), Some(receiver)) =
                (func.self_receiver, func.params.first_mut())
            {
                receiver.ty = match (receiver_mode, impl_target_ty.clone()) {
                    (
                        crate::types::ReceiverMode::Mut,
                        target @ Type::Reference { mutable: true, .. },
                    ) => target,
                    (crate::types::ReceiverMode::Shared, target) => Type::Reference {
                        mutable: false,
                        inner: Box::new(target),
                    },
                    (crate::types::ReceiverMode::Mut, target) => Type::Reference {
                        mutable: true,
                        inner: Box::new(target),
                    },
                    (crate::types::ReceiverMode::Move, target) => target,
                };
            }
        }
        let method_name = ident.name.clone();
        methods.insert(method_name, func);
    }

    let all_type_generic_names = context.generics.params().to_vec();
    let all_type_generic_kinds = context.generics.kinds().to_vec();

    let mut associated_types = Vec::new();
    for (index, assoc) in imp.associated_types.iter().enumerate() {
        let declaration = trait_associated_types
            .iter()
            .find(|decl| decl.name == assoc.name.name);
        let id = declaration.map(|decl| decl.id).unwrap_or_else(|| {
            if trait_def.is_some() {
                context.push_error_with_span(
                    format!(
                        "unknown associated type '{}' for trait '{}'",
                        assoc.name.name,
                        trait_name.as_deref().unwrap_or("<unknown>")
                    ),
                    assoc.name.span.clone(),
                );
            }
            AssocTypeId(index as u32)
        });
        let kind = crate::type_lowering::lower_associated_type_kind(assoc.kind.as_ref());
        if let Some(declaration) = declaration {
            if declaration.kind != kind {
                context.push_error_with_span(
                    format!(
                        "associated type '{}' has kind {}, but trait '{}' requires {}",
                        assoc.name.name,
                        kind,
                        trait_name.as_deref().unwrap_or("<unknown>"),
                        declaration.kind,
                    ),
                    assoc.name.span.clone(),
                );
            }
        }
        let ty = crate::type_lowering::TypeLowerer::lower_unsized_type(context, &assoc.ty);
        if !matches!(ty, Type::Error) {
            match crate::type_lowering::TypeLowerer::kind_of(context, &ty) {
                Ok(actual) if actual != kind => context.push_error_with_span(
                    format!(
                        "associated type '{}' defines kind {}, but its body has kind {}",
                        assoc.name.name, kind, actual,
                    ),
                    assoc.ty.span(),
                ),
                Err(error) => context.push_error_with_span(error, assoc.ty.span()),
                _ => {}
            }
        }
        associated_types.push(HirAssociatedTypeDef {
            id,
            name: assoc.name.name.clone(),
            kind,
            ty,
        });
    }
    context.pop_generic_context_with_kinds(previous_generics);
    let type_generics = all_type_generic_names
        .iter()
        .zip(all_type_generic_kinds)
        .enumerate()
        .map(|(index, (name, kind))| {
            GenericParamDecl::new(
                GenericParamId {
                    owner: id,
                    index: index as u32,
                },
                name.clone(),
                kind,
            )
        })
        .collect::<Vec<_>>();
    let trait_generics = trait_generic_names
        .into_iter()
        .filter_map(|name| {
            all_type_generic_names
                .iter()
                .position(|candidate| candidate == &name)
                .map(|index| type_generics[index].clone())
        })
        .collect();

    HirImpl {
        id,
        owner: match imp.for_.as_ref() {
            Some(ast::ParseType::Slice(_)) if !type_name.contains(';') => {
                HirImplOwner::BuiltinSlice
            }
            _ => HirImplOwner::Named(type_name.clone()),
        },
        type_name,
        type_generics,
        receiver_pattern,
        trait_name,
        trait_id,
        trait_generics,
        trait_arg_types,
        associated_types,
        bounds,
        methods,
    }
}

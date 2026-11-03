use std::collections::{HashMap, HashSet};

use crate::ast;
use crate::collect::context::CollectContext;
use crate::collect::CollectedTraitMemberIds;
use crate::hir::*;
use crate::ids::{AssocTypeId, DefId, FieldId, TypeVarId, VariantId};
use crate::type_lowering::TypeLoweringContext;
use crate::types::{GenericParamDecl, GenericParamId, TraitBound, Type};

fn collect_generic_ids_from_type(ty: &Type, generic_params: &mut Vec<GenericParamId>) {
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

fn collect_generic_names_from_parse_type<F>(
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

fn collect_constructor_generic_kinds<F>(
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

fn parse_type_name_span(ty: &ast::ParseType, name: &str) -> Option<crate::lexer::Span> {
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

fn collect_declared_generic_kinds<F>(
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

fn is_known_nominal_type_name(context: &CollectContext, name: &str) -> bool {
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

fn lower_where_clauses(
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
        let argument_kinds = context.traits.values().find(|item| item.id == trait_id)
            .map(|item| item.generic_params.iter().map(|param| param.kind.clone()).collect::<Vec<_>>()).unwrap_or_default();
        let mut type_args = type_args
            .iter()
            .enumerate()
            .map(|(index, generic)| crate::type_lowering::TypeLowerer::lower_type_argument(context, generic,
                argument_kinds.get(index).unwrap_or(&crate::type_services::kind::Kind::Type)))
            .collect::<Vec<_>>();
        if let Some(trait_def) = context.traits.values().find(|item| item.id == trait_id) {
            match crate::type_lowering::complete_generic_args(&trait_def.generic_params, type_args.clone(), Some(&subject)) {
                Ok(completed) => type_args = completed,
                Err(message) => context.push_error_with_span(message, clause.trait_bound.as_ref().unwrap().span()),
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

fn remap_generic_param_ids_in_type(
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

fn remap_generic_bounds_owner(
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

fn build_function_type(param_types: &[Type], ret_type: Type, is_curried: bool) -> Type {
    if is_curried {
        param_types.iter().rev().fold(ret_type, |acc, param| {
            Type::function(vec![param.clone()], acc)
        })
    } else {
        Type::function(param_types.to_vec(), ret_type)
    }
}

fn collect_type_var_ids(ty: &Type, type_vars: &mut HashSet<TypeVarId>) {
    crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
        if let Type::TypeVar(id) = nested {
            type_vars.insert(*id);
        }
    });
}

pub(crate) fn build_nominal_generic_params(
    context: &mut CollectContext,
    syntax: &[ast::GenericParamDecl],
    id: DefId,
) -> Vec<GenericParamDecl> {
    let mut generic_params = crate::type_lowering::lower_generic_param_decls(id, syntax);
    let spans = syntax.iter().map(|param| param.span.clone()).collect::<Vec<_>>();
    let previous = context.push_generic_context_with_kinds_at(
        id,
        GenericParamDecl::names(&generic_params).map(str::to_string).collect(),
        generic_params.iter().map(|param| param.kind.clone()).collect(),
        &spans,
    );
    crate::type_lowering::lower_param_defaults(context, &mut generic_params, syntax, None);
    context.pop_generic_context_with_kinds(previous.0, previous.1, previous.2);
    // Publish through every qualified/imported alias before any fields are lowered.
    for declaration in context.structs.values_mut().filter(|item| item.id == id) {
        declaration.generic_params.clone_from(&generic_params);
    }
    for declaration in context.enums.values_mut().filter(|item| item.id == id) {
        declaration.generic_params.clone_from(&generic_params);
    }
    generic_params
}

pub(crate) fn build_struct_with_id(
    context: &mut CollectContext,
    sd: &ast::StructDecl,
    id: DefId,
    generic_params: Vec<GenericParamDecl>,
) -> HirStruct {
    let generic_param_names = GenericParamDecl::names(&generic_params)
        .map(str::to_string)
        .collect();
    let generic_param_kinds = generic_params
        .iter()
        .map(|param| param.kind.clone())
        .collect();
    let generic_spans = sd
        .generic_params
        .iter()
        .map(|param| param.span.clone())
        .collect::<Vec<_>>();
    let (prev_owner, prev_params, prev_kinds) = context.push_generic_context_with_kinds_at(
        id,
        generic_param_names,
        generic_param_kinds,
        &generic_spans,
    );
    let fields = sd
        .fields
        .iter()
        .enumerate()
        .map(|(index, f)| HirField {
            id: FieldId(index as u32),
            name: f.name.name.clone(),
            ty: context.lower_parse_type(&f.ty),
            public: f.public,
        })
        .collect();
    context.pop_generic_context_with_kinds(prev_owner, prev_params, prev_kinds);

    HirStruct {
        id,
        name: sd.name.name.clone(),
        generic_params,
        fields,
    }
}

pub(crate) fn build_type_alias_with_id(
    context: &mut CollectContext,
    name: &ast::ParseTypeInner,
    params: &[ast::GenericParamDecl],
    target: &ast::ParseType,
    id: DefId,
) -> crate::hir::HirTypeAlias {
    let mut generic_params = crate::type_lowering::lower_generic_param_decls(id, params);
    let generic_names = GenericParamDecl::names(&generic_params)
        .map(str::to_string)
        .collect();
    let generic_kinds = generic_params
        .iter()
        .map(|param| param.kind.clone())
        .collect();
    let generic_spans = params
        .iter()
        .map(|param| param.span.clone())
        .collect::<Vec<_>>();
    let (previous_owner, previous_params, previous_kinds) = context
        .push_generic_context_with_kinds_at(id, generic_names, generic_kinds, &generic_spans);
    crate::type_lowering::lower_param_defaults(context, &mut generic_params, params, None);
    let mut ty = crate::type_lowering::TypeLowerer::lower_parse_type_term(context, target);
    context.pop_generic_context_with_kinds(previous_owner, previous_params, previous_kinds);
    if !generic_params.is_empty() {
        struct AliasBinder<'a> {
            owner: DefId,
            params: &'a [GenericParamDecl],
            depth: u32,
        }
        impl crate::type_services::visit::TypeFolder for AliasBinder<'_> {
            fn enter_binders(&mut self, _params: &[crate::type_services::kind::Kind]) {
                self.depth += 1;
            }

            fn exit_binders(&mut self) {
                self.depth -= 1;
            }

            fn fold_type(&mut self, ty: Type) -> Type {
                match ty {
                    Type::Generic(param) if param.owner == self.owner => Type::BoundVar {
                        depth: self.depth,
                        index: param.index,
                        kind: self.params[param.index as usize].kind.clone(),
                    },
                    other => crate::type_services::visit::fold_type_children(other, self),
                }
            }
        }
        ty = crate::type_services::visit::fold_type(
            ty,
            &mut AliasBinder {
                owner: id,
                params: &generic_params,
                depth: 0,
            },
        );
        ty = Type::Lambda {
            params: generic_params
                .iter()
                .map(|param| param.kind.clone())
                .collect(),
            body: Box::new(ty),
        };
    }
    crate::hir::HirTypeAlias {
        id,
        name: name.name.clone(),
        generic_params,
        ty,
    }
}

pub(crate) fn build_enum_with_id(
    context: &mut CollectContext,
    ed: &ast::EnumDecl,
    id: DefId,
    generic_params: Vec<GenericParamDecl>,
) -> HirEnum {
    let generic_param_names = GenericParamDecl::names(&generic_params)
        .map(str::to_string)
        .collect();
    let generic_param_kinds = generic_params
        .iter()
        .map(|param| param.kind.clone())
        .collect();
    let generic_spans = ed
        .generic_params
        .iter()
        .map(|param| param.span.clone())
        .collect::<Vec<_>>();
    let (prev_owner, prev_params, prev_kinds) = context.push_generic_context_with_kinds_at(
        id,
        generic_param_names,
        generic_param_kinds,
        &generic_spans,
    );
    let mut next_field_id = 0;
    let variants = ed
        .variants
        .iter()
        .enumerate()
        .map(|(variant_index, v)| {
            let fields = match &v.fields {
                ast::NamedFieldsOrTypesList::NamedFields(fields) => HirVariantFields::Named(
                    fields
                        .iter()
                        .map(|f| {
                            let id = FieldId(next_field_id);
                            next_field_id += 1;
                            HirField {
                                id,
                                name: f.name.name.clone(),
                                ty: context.lower_parse_type(&f.ty),
                                public: f.public,
                            }
                        })
                        .collect(),
                ),
                ast::NamedFieldsOrTypesList::TypesList(types) => {
                    if types.is_empty() {
                        HirVariantFields::Unit
                    } else {
                        HirVariantFields::Positional(
                            types.iter().map(|t| context.lower_parse_type(t)).collect(),
                        )
                    }
                }
            };
            HirVariant {
                id: VariantId(variant_index as u32),
                name: v.name.name.clone(),
                fields,
            }
        })
        .collect();
    context.pop_generic_context_with_kinds(prev_owner, prev_params, prev_kinds);

    HirEnum {
        id,
        name: ed.name.name.clone(),
        generic_params,
        variants,
    }
}

pub(crate) fn build_function_sig_with_id(
    context: &mut CollectContext,
    sig: &ast::FunctionSig,
    id: DefId,
) -> HirFunctionSig {
    let fallback_generic_context = context.current_generic_owner.is_none();
    let inherited_generic_param_count = if fallback_generic_context {
        0
    } else {
        context.current_generic_params.len()
    };
    let mut signature_id = id;
    let mut constructor_kinds = HashMap::new();
    for clause in &sig.where_clauses {
        collect_constructor_generic_kinds(&clause.subject, &mut constructor_kinds, &|name| {
            is_known_nominal_type_name(context, name)
        });
    }
    let fallback_context = if fallback_generic_context {
        let owner = id;
        signature_id = owner;
        let mut params = constructor_kinds.keys().cloned().collect::<Vec<_>>();
        params.sort();
        let mut kinds = params
            .iter()
            .map(|name| constructor_kinds[name].clone())
            .collect::<Vec<_>>();
        if sig.self_receiver.is_some() {
            params.push("Self".to_string());
            kinds.push(crate::type_services::kind::Kind::Type);
        }
        let spans = vec![sig.sig.span(); params.len()];
        Some(context.push_generic_context_with_kinds_at(owner, params, kinds, &spans))
    } else {
        None
    };
    let nested_context = if fallback_generic_context {
        None
    } else {
        let previous = (
            context.current_generic_params.clone(),
            context.current_generic_kinds.clone(),
        );
        let mut constructor_params = constructor_kinds.into_iter().collect::<Vec<_>>();
        constructor_params.sort_by(|left, right| left.0.cmp(&right.0));
        for (name, kind) in constructor_params {
            if let Some(index) = context
                .current_generic_params
                .iter()
                .position(|param| param == &name)
            {
                let declared_kind = context.current_generic_kinds[index].clone();
                if declared_kind != kind {
                    let span = sig
                        .where_clauses
                        .iter()
                        .find_map(|clause| parse_type_name_span(&clause.subject, &name))
                        .unwrap_or_else(|| sig.sig.span());
                    context.push_error_with_span(
                        format!(
                            "generic parameter '{name}' was declared with kind {declared_kind}, but its constructor binder requires {kind}",
                        ),
                        span,
                    );
                }
            } else {
                context.current_generic_params.push(name);
                context.current_generic_kinds.push(kind);
            }
        }
        Some(previous)
    };
    fn flatten_curried_type(ty: &Type) -> (Vec<Type>, Type) {
        match ty {
            Type::Function { params, ret, .. } => {
                // Unparenthesized arrows are already parsed into one Function node with
                // multiple params. A nested Function here is an explicit higher-order type.
                (params.clone(), ret.as_ref().clone())
            }
            _ => (vec![], ty.clone()),
        }
    }

    let mut lowered_type = context.lower_parse_type(&sig.sig);
    let mut receiver_ty = sig
        .self_receiver
        .map(|self_receiver| context.receiver_ty_for_self(self_receiver, sig.name.span.clone()));
    let lowered_generic_bounds = lower_where_clauses(context, &sig.where_clauses);
    let mut generic_param_ids = Vec::new();
    collect_generic_ids_from_type(&lowered_type, &mut generic_param_ids);
    if let Some(receiver_ty) = &receiver_ty {
        collect_generic_ids_from_type(receiver_ty, &mut generic_param_ids);
    }
    for predicate in &lowered_generic_bounds.predicates {
        let crate::types::Predicate::Trait { subject, args, .. } = predicate;
        collect_generic_ids_from_type(subject, &mut generic_param_ids);
        for arg in args {
            collect_generic_ids_from_type(arg, &mut generic_param_ids);
        }
    }
    let signature_owned_generic_param_ids = generic_param_ids
        .iter()
        .copied()
        .filter(|param| {
            context.current_generic_owner == Some(param.owner)
                && param.index >= inherited_generic_param_count as u32
        })
        .collect::<Vec<_>>();
    remap_generic_param_ids_in_type(
        &mut lowered_type,
        &signature_owned_generic_param_ids,
        signature_id,
    );
    if let Some(receiver_ty) = &mut receiver_ty {
        remap_generic_param_ids_in_type(
            receiver_ty,
            &signature_owned_generic_param_ids,
            signature_id,
        );
    }
    let mut public_signature_generic_params = Vec::new();
    let mut hidden_signature_generic_params = Vec::new();
    for (index, param) in signature_owned_generic_param_ids.iter().enumerate() {
        let remapped = GenericParamId {
            owner: signature_id,
            index: index as u32,
        };
        if let Some(name) = context.current_generic_params.get(param.index as usize) {
            if name != "Self" {
                public_signature_generic_params.push(GenericParamDecl::new(
                    remapped,
                    name.clone(),
                    context
                        .current_generic_kinds
                        .get(param.index as usize)
                        .cloned()
                        .unwrap_or(crate::type_services::kind::Kind::Type),
                ));
            } else {
                hidden_signature_generic_params.push(GenericParamDecl::new(
                    remapped,
                    name.clone(),
                    context
                        .current_generic_kinds
                        .get(param.index as usize)
                        .cloned()
                        .unwrap_or(crate::type_services::kind::Kind::Type),
                ));
            }
        }
    }
    let mut generic_params = public_signature_generic_params;
    generic_params.extend(hidden_signature_generic_params);
    let (mut params, ret) = flatten_curried_type(&lowered_type);
    if let Some(receiver_ty) = receiver_ty {
        params.insert(0, receiver_ty);
    }
    let generic_bounds = remap_generic_bounds_owner(
        &lowered_generic_bounds,
        &signature_owned_generic_param_ids,
        signature_id,
    );

    let hir_sig = HirFunctionSig {
        id: signature_id,
        name: sig.name.name.clone(),
        generic_params,
        params,
        ret,
        generic_bounds,
        self_receiver: sig.self_receiver.map(receiver_mode),
        is_unsafe: sig.is_unsafe,
    };
    if let Some((prev_owner, prev_params, prev_kinds)) = fallback_context {
        context.pop_generic_context_with_kinds(prev_owner, prev_params, prev_kinds);
    }
    if let Some((previous_params, previous_kinds)) = nested_context {
        context.current_generic_params = previous_params;
        context.current_generic_kinds = previous_kinds;
    }

    hir_sig
}

pub(crate) fn build_function_header_with_id(
    context: &mut CollectContext,
    fd: &ast::FunctionDecl,
    function_id: DefId,
) -> HirFunction {
    let lambda = &fd.lambda;
    let func_name = fd.name.name.clone();
    let is_curried = matches!(lambda.arrow_kind, ast::LambdaArrowKind::Curried);

    context.current_function = Some(func_name.clone());
    let mut func_type_vars = HashSet::new();

    let mut all_params = Vec::new();
    let mut all_param_types = Vec::new();

    if let Some(self_receiver) = fd.self_receiver {
        let self_param = context.build_self_param_at(self_receiver, lambda.span.clone());
        collect_type_var_ids(&self_param.ty, &mut func_type_vars);
        all_param_types.push(self_param.ty.clone());
        all_params.push(self_param);
    }

    for param in &lambda.parameters {
        let (name, ty, mutable, is_ref) =
            context.lower_param_pattern_at(param, lambda.span.clone());
        collect_type_var_ids(&ty, &mut func_type_vars);
        all_param_types.push(ty.clone());
        all_params.push(HirParam {
            name,
            local_id: crate::ids::HirLocalId(all_params.len() as u32),
            ty,
            mutable,
            is_ref,
        });
    }

    let ret_type = if matches!(lambda.arrow_kind, ast::LambdaArrowKind::Unit) {
        Type::Unit
    } else {
        context.type_vars.fresh_type_var_at(lambda.span.clone())
    };
    collect_type_var_ids(&ret_type, &mut func_type_vars);

    let outer_param_count = if is_curried && !lambda.parameters.is_empty() {
        if fd.self_receiver.is_some() {
            2
        } else {
            1
        }
    } else {
        all_params.len()
    };
    let params = all_params[..outer_param_count].to_vec();
    let remaining_param_types = all_param_types[outer_param_count..].to_vec();
    let ret_type = if is_curried {
        build_function_type(&remaining_param_types, ret_type, true)
    } else {
        ret_type
    };

    collect_type_var_ids(&ret_type, &mut func_type_vars);
    context
        .function_type_vars
        .insert(function_id, func_type_vars);
    context.current_function = None;

    HirFunction {
        id: function_id,
        name: func_name,
        generic_params: vec![],
        generic_bounds: crate::hir::HirGenericBounds::new(),
        params,
        ret_type,
        body: HirBlock {
            stmts: vec![],
            ty: Type::Unit,
        },
        is_curried,
        is_method: fd.self_receiver.is_some(),
        self_receiver: fd.self_receiver.map(receiver_mode),
        is_unsafe: fd.is_unsafe,
    }
}

pub(crate) fn build_function_header_with_sig(
    context: &mut CollectContext,
    fd: &ast::FunctionDecl,
    sig: &HirFunctionSig,
    function_id: DefId,
) -> HirFunction {
    let lambda = &fd.lambda;
    let func_name = fd.name.name.clone();
    let is_curried = matches!(lambda.arrow_kind, ast::LambdaArrowKind::Curried);

    context.current_function = Some(func_name.clone());
    let mut func_type_vars = HashSet::new();

    let mut sig_params = sig.params.clone();
    let mut sig_ret = sig.ret.clone();
    let mut hidden_receiver_generic_ids = Vec::new();
    if fd.self_receiver.is_some() {
        if let Some(receiver_ty) = sig.params.first() {
            collect_generic_ids_from_type(receiver_ty, &mut hidden_receiver_generic_ids);
        }
    }
    let mut public_signature_owned_generic_param_ids = Vec::new();
    let mut hidden_signature_owned_generic_param_ids = Vec::new();
    for generic_id in sig.generic_params.iter().map(|param| param.id) {
        if hidden_receiver_generic_ids.contains(&generic_id) {
            hidden_signature_owned_generic_param_ids.push(generic_id);
        } else {
            public_signature_owned_generic_param_ids.push(generic_id);
        }
    }
    let mut signature_owned_generic_param_ids = public_signature_owned_generic_param_ids;
    signature_owned_generic_param_ids.extend(hidden_signature_owned_generic_param_ids);
    for param in &mut sig_params {
        remap_generic_param_ids_in_type(param, &signature_owned_generic_param_ids, function_id);
    }
    remap_generic_param_ids_in_type(
        &mut sig_ret,
        &signature_owned_generic_param_ids,
        function_id,
    );
    let generic_bounds = remap_generic_bounds_owner(
        &sig.generic_bounds,
        &signature_owned_generic_param_ids,
        function_id,
    );

    let mut used_generic_param_ids = Vec::new();
    for ty in &sig_params {
        collect_generic_ids_from_type(ty, &mut used_generic_param_ids);
    }
    collect_generic_ids_from_type(&sig_ret, &mut used_generic_param_ids);
    for (param, bounds) in &generic_bounds {
        if !used_generic_param_ids.contains(param) {
            used_generic_param_ids.push(*param);
        }
        for bound in bounds {
            for arg in &bound.type_args {
                collect_generic_ids_from_type(arg, &mut used_generic_param_ids);
            }
        }
    }
    for predicate in &generic_bounds.predicates {
        let crate::types::Predicate::Trait { subject, args, .. } = predicate;
        collect_generic_ids_from_type(subject, &mut used_generic_param_ids);
        for arg in args {
            collect_generic_ids_from_type(arg, &mut used_generic_param_ids);
        }
    }

    let mut generic_params = Vec::new();
    let mut public_entries = Vec::new();
    let mut hidden_entries = Vec::new();
    for generic in &sig.generic_params {
        if hidden_receiver_generic_ids.contains(&generic.id) {
            hidden_entries.push(generic.clone());
        } else {
            public_entries.push(generic.clone());
        }
    }
    public_entries.extend(hidden_entries);
    for generic in public_entries {
        let generic_id = generic.id;
        let remapped = if let Some(index) = signature_owned_generic_param_ids
            .iter()
            .position(|id| id == &generic_id)
        {
            GenericParamId {
                owner: function_id,
                index: index as u32,
            }
        } else {
            generic_id
        };
        if used_generic_param_ids.contains(&remapped) {
            generic_params.push(GenericParamDecl::new(remapped, generic.name, generic.kind));
        }
    }

    let mut all_params = Vec::new();

    if let Some(self_receiver) = fd.self_receiver {
        let mut self_param = context.build_self_param_at(self_receiver, lambda.span.clone());
        if let Some(sig_self_ty) = sig_params.first() {
            self_param.ty = sig_self_ty.clone();
        }
        collect_type_var_ids(&self_param.ty, &mut func_type_vars);
        all_params.push(self_param);
    }

    let param_start = if fd.self_receiver.is_some() { 1 } else { 0 };
    for (i, param) in lambda.parameters.iter().enumerate() {
        let (name, _decl_ty, mutable, is_ref) =
            context.lower_param_pattern_at(param, fd.lambda.span.clone());
        let ty = if param_start + i < sig.params.len() {
            sig_params[param_start + i].clone()
        } else {
            context.type_vars.fresh_type_var_at(
                crate::lower::Lowerer::pattern_binding_span(param)
                    .unwrap_or_else(|| fd.lambda.span.clone()),
            )
        };
        collect_type_var_ids(&ty, &mut func_type_vars);
        all_params.push(HirParam {
            name,
            local_id: crate::ids::HirLocalId(all_params.len() as u32),
            ty,
            mutable,
            is_ref,
        });
    }

    let outer_param_count = if is_curried && !lambda.parameters.is_empty() {
        if fd.self_receiver.is_some() {
            2
        } else {
            1
        }
    } else {
        all_params.len()
    };
    let params = all_params[..outer_param_count].to_vec();
    let remaining_param_types: Vec<Type> = all_params[outer_param_count..]
        .iter()
        .map(|param| param.ty.clone())
        .collect();
    let ret_type = if is_curried {
        build_function_type(&remaining_param_types, sig_ret, true)
    } else {
        sig_ret
    };

    collect_type_var_ids(&ret_type, &mut func_type_vars);
    context
        .function_type_vars
        .insert(function_id, func_type_vars);
    context.current_function = None;

    HirFunction {
        id: function_id,
        name: func_name,
        generic_params,
        generic_bounds,
        params,
        ret_type,
        body: HirBlock {
            stmts: vec![],
            ty: Type::Unit,
        },
        is_curried,
        is_method: fd.self_receiver.is_some(),
        self_receiver: fd.self_receiver.map(receiver_mode),
        is_unsafe: fd.is_unsafe,
    }
}

fn receiver_mode(mode: ast::SelfReceiverMode) -> crate::types::ReceiverMode {
    match mode {
        ast::SelfReceiverMode::Shared => crate::types::ReceiverMode::Shared,
        ast::SelfReceiverMode::Mut => crate::types::ReceiverMode::Mut,
        ast::SelfReceiverMode::Move => crate::types::ReceiverMode::Move,
    }
}

pub(crate) fn build_trait_with_id(
    context: &mut CollectContext,
    td: &ast::TraitDecl,
    id: DefId,
    member_ids: &CollectedTraitMemberIds,
) -> HirTrait {
    let mut generic_params = crate::type_lowering::lower_generic_param_decls(id, &td.generic_params);
    let generic_param_names = GenericParamDecl::names(&generic_params)
        .map(str::to_string)
        .collect::<Vec<_>>();

    let prev_trait = context.current_trait.clone();
    let prev_trait_generics = context.current_trait_generics.clone();
    context.current_trait = Some(td.name.name.clone());
    context.current_trait_generics = generic_param_names.clone();
    let mut generic_context_params = generic_param_names.clone();
    generic_context_params.push(
        td.for_
            .as_ref()
            .map(|target| target.name.name.clone())
            .unwrap_or_else(|| "Self".to_string()),
    );
    let mut generic_context_kinds = generic_params
        .iter()
        .map(|param| param.kind.clone())
        .collect::<Vec<_>>();
    generic_context_kinds.push(
        td.for_
            .as_ref()
            .map(|target| crate::type_lowering::lower_generic_param_kind(target.kind.as_ref()))
            .unwrap_or(crate::type_services::kind::Kind::Type),
    );
    let mut generic_context_spans = td
        .generic_params
        .iter()
        .map(|param| param.span.clone())
        .collect::<Vec<_>>();
    generic_context_spans.push(
        td.for_
            .as_ref()
            .map(|target| target.name.span.clone())
            .unwrap_or_else(|| td.name.span.clone()),
    );
    let (prev_owner, prev_params, prev_kinds) = context.push_generic_context_with_kinds_at(
        id,
        generic_context_params,
        generic_context_kinds,
        &generic_context_spans,
    );

    let subject_name = td.for_.as_ref().map(|param| param.name.name.clone()).unwrap_or_else(|| "Self".into());
    let subject = GenericParamDecl::new(
        GenericParamId { owner: id, index: generic_params.len() as u32 },
        subject_name.clone(),
        td.for_.as_ref().map(|param| crate::type_lowering::lower_generic_param_kind(param.kind.as_ref()))
            .unwrap_or(crate::type_services::kind::Kind::Type),
    );
    crate::type_lowering::lower_param_defaults(context, &mut generic_params, &td.generic_params, Some((subject_name, subject)));
    let associated_types: Vec<HirAssociatedTypeDecl> = td
        .associated_types
        .iter()
        .enumerate()
        .map(|(index, assoc)| HirAssociatedTypeDecl {
            id: AssocTypeId(index as u32),
            name: assoc.name.name.clone(),
            kind: crate::type_lowering::lower_associated_type_kind(assoc.kind.as_ref()),
        })
        .collect();
    let target = td.for_.as_ref().map(|target| {
        GenericParamDecl::new(
            GenericParamId {
                owner: id,
                index: generic_params.len() as u32,
            },
            target.name.name.clone(),
            crate::type_lowering::lower_generic_param_kind(target.kind.as_ref()),
        )
    });
    let predicates = lower_where_clauses(context, &td.where_clauses).predicates;
    let previous_trait = context.traits.insert(
        td.name.name.clone(),
        HirTrait {
            id,
            name: td.name.name.clone(),
            generic_params: generic_params.clone(),
            target: target.clone(),
            predicates: predicates.clone(),
            associated_types: associated_types.clone(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );

    let mut methods = HashMap::new();
    for (ident, fd) in &td.methods {
        let method_id = *member_ids
            .methods
            .get(&ident.name)
            .expect("trait method IDs must be allocated before header building");
        let func = build_function_header_with_id(context, fd, method_id);
        methods.insert(ident.name.clone(), func);
    }

    let mut signatures = HashMap::new();
    for (ident, sig) in &td.signatures {
        let signature_id = *member_ids
            .signatures
            .get(&ident.name)
            .expect("trait signature IDs must be allocated before header building");
        let hir_sig = build_function_sig_with_id(context, sig, signature_id);
        signatures.insert(ident.name.clone(), hir_sig);
    }

    if let Some(previous_trait) = previous_trait {
        context.traits.insert(td.name.name.clone(), previous_trait);
    } else {
        context.traits.remove(&td.name.name);
    }

    context.current_trait = prev_trait;
    context.current_trait_generics = prev_trait_generics;
    context.pop_generic_context_with_kinds(prev_owner, prev_params, prev_kinds);

    HirTrait {
        id,
        name: td.name.name.clone(),
        generic_params,
        target,
        predicates,
        associated_types,
        methods,
        signatures,
    }
}

pub(crate) fn build_extern_with_id(
    context: &mut CollectContext,
    sig: &ast::FunctionSig,
    name: String,
    id: DefId,
) -> HirExtern {
    let hir_sig = build_function_sig_with_id(context, sig, id);
    let params = hir_sig.params.clone();
    let ret = hir_sig.ret.clone();

    let func_type = Type::function_with_safety(
        params.clone(),
        ret.clone(),
        crate::types::FunctionSafety::from_is_unsafe(sig.is_unsafe),
    );
    context.scope.define(name.clone(), func_type, false);

    HirExtern {
        id,
        name,
        params,
        ret,
        variadic: false,
        is_unsafe: sig.is_unsafe,
    }
}

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
    let (prev_owner, prev_params, prev_kinds) = context.push_generic_context_with_kinds_at(
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
        .map(|(index, generic)| crate::type_lowering::TypeLowerer::lower_type_argument(context, generic,
            trait_def.as_ref().and_then(|item| item.generic_params.get(index)).map(|param| &param.kind)
                .unwrap_or(&crate::type_services::kind::Kind::Type)))
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
        match crate::type_lowering::complete_generic_args(&trait_def.generic_params, trait_arg_types.clone(), Some(&impl_target_ty)) {
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
            if context.current_generic_kinds.get(index)
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
        let explicit_signature = imp.signatures.iter()
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
            if let (Some(receiver_mode), Some(receiver)) = (func.self_receiver, func.params.first_mut()) {
                receiver.ty = match (receiver_mode, impl_target_ty.clone()) {
                    (crate::types::ReceiverMode::Mut, target @ Type::Reference { mutable: true, .. }) => target,
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

    let all_type_generic_names = context.current_generic_params.clone();
    let all_type_generic_kinds = context.current_generic_kinds.clone();

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
    context.pop_generic_context_with_kinds(prev_owner, prev_params, prev_kinds);
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

#[cfg(test)]
mod tests;

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
        let type_args = type_args
            .iter()
            .map(|generic| context.lower_parse_type(generic))
            .collect::<Vec<_>>();
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

pub(crate) fn build_struct_with_id(
    context: &mut CollectContext,
    sd: &ast::StructDecl,
    id: DefId,
) -> HirStruct {
    let generic_params = crate::type_lowering::lower_generic_param_decls(id, &sd.generic_params);
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
    target: &ast::ParseType,
    id: DefId,
) -> crate::hir::HirTypeAlias {
    let generic_params = crate::type_lowering::lower_parse_generic_param_decls(id, &name.generics);
    let generic_names = GenericParamDecl::names(&generic_params)
        .map(str::to_string)
        .collect();
    let generic_kinds = generic_params
        .iter()
        .map(|param| param.kind.clone())
        .collect();
    let generic_spans = name
        .generics
        .iter()
        .map(|param| param.span())
        .collect::<Vec<_>>();
    let (previous_owner, previous_params, previous_kinds) = context
        .push_generic_context_with_kinds_at(id, generic_names, generic_kinds, &generic_spans);
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
) -> HirEnum {
    let generic_params =
        crate::type_lowering::lower_parse_generic_param_decls(id, &ed.name.generics);
    let generic_param_names = GenericParamDecl::names(&generic_params)
        .map(str::to_string)
        .collect();
    let generic_param_kinds = generic_params
        .iter()
        .map(|param| param.kind.clone())
        .collect();
    let generic_spans = ed
        .name
        .generics
        .iter()
        .map(|param| param.span())
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
    let generic_params = crate::type_lowering::lower_generic_param_decls(id, &td.generic_params);
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
        for generic in &imp.name.generics {
            collect_generic_names_from_parse_type(generic, &mut impl_generic_params, &|name| {
                is_known_nominal_type_name(context, name)
            });
        }
    }
    for generic in &imp.name.generics {
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
    for generic in &imp.name.generics {
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
        ), imp.name.span.clone());
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
    let generic_spans = vec![imp.name.span.clone(); impl_generic_params.len()];
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
            Some(imp.name.name.clone()),
        )
    } else {
        let (type_name, type_generics, receiver_pattern) = context.impl_type_info(imp);
        (type_name, type_generics, receiver_pattern, None)
    };

    let trait_generic_names: Vec<String> = imp
        .name
        .generics
        .iter()
        .filter_map(|g| {
            if let ast::ParseType::Type(inner) = g {
                Some(inner.name.clone())
            } else {
                None
            }
        })
        .collect();
    let trait_arg_types: Vec<Type> = imp
        .name
        .generics
        .iter()
        .map(|g| context.lower_parse_type(g))
        .collect();
    let trait_def = trait_name
        .as_deref()
        .and_then(|name| context.trait_by_name(name))
        .cloned();
    let trait_id = trait_def.as_ref().map(|trait_def| trait_def.id);
    let impl_target_ty = match &receiver_pattern {
        crate::hir::HirImplReceiverPattern::Exact(ty)
        | crate::hir::HirImplReceiverPattern::Constructor(ty) => ty.clone(),
        crate::hir::HirImplReceiverPattern::SliceFamily { element } => {
            Type::Slice(Box::new(element.clone()))
        }
    };
    if let Some(trait_def) = trait_def.as_ref() {
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
                    .unwrap_or_else(|| imp.name.span.clone()),
            ),
            Err(error) => context.push_error_with_span(
                error,
                imp.for_
                    .as_ref()
                    .map(|ty| ty.span())
                    .unwrap_or_else(|| imp.name.span.clone()),
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
        let func = if let Some(signature) = trait_signature {
            build_function_header_with_sig(context, fd, &signature, method_id)
        } else {
            build_function_header_with_id(context, fd, method_id)
        };
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
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;

    use crate::ast::{
        self, AssociatedTypeDecl, AssociatedTypeDef, EnumDecl, EnumVariant, FunctionDecl,
        FunctionSig, Ident, Impl, LambdaArrowKind, LambdaDecl, NamedFieldsOrTypesList, ParseType,
        ParseTypeInner, Pattern, PatternKind, SelfReceiverMode, StructDecl, StructDeclField,
        TraitDecl, WhereClause,
    };
    use crate::collect::context::CollectContext;
    use crate::hir::{HirFunctionSig, HirVariantFields};
    use crate::ids::{CrateId, LocalDefId};
    use crate::language_items::SizedLanguageItems;
    use crate::lexer::Span;
    use crate::types::{AssociatedTypeKey, GenericParamId, Type};

    fn ident(name: &str) -> Ident {
        Ident {
            name: name.to_string(),
            span: Span::test(),
        }
    }

    fn type_inner(name: &str) -> ParseTypeInner {
        ParseTypeInner {
            name: name.to_string(),
            generics: vec![],
            span: Span::test(),
        }
    }

    fn named_type(name: &str) -> ParseType {
        ParseType::Type(type_inner(name))
    }

    fn generic_type(name: &str) -> ParseType {
        ParseType::Type(type_inner(name))
    }

    fn generic_param(name: &str) -> ast::GenericParamDecl {
        ast::GenericParamDecl {
            name: ident(name),
            kind: None,
            span: Span::test(),
        }
    }

    fn constructor_kind_syntax(name: &str, arity: usize) -> ast::TypeApplication {
        ast::TypeApplication {
            constructor: Box::new(named_type(name)),
            args: (0..arity)
                .map(|_| ast::ParseType::Hole(ast::TypeHole { span: Span::test() }))
                .collect(),
            span: Span::test(),
        }
    }

    fn generic_type_inner(name: &str) -> ParseTypeInner {
        ParseTypeInner {
            name: name.to_string(),
            generics: vec![],
            span: Span::test(),
        }
    }

    fn def_id(local: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(local))
    }

    fn trait_member_ids(
        methods: &[(&str, DefId)],
        signatures: &[(&str, DefId)],
    ) -> CollectedTraitMemberIds {
        CollectedTraitMemberIds {
            methods: methods
                .iter()
                .map(|(name, id)| ((*name).to_string(), *id))
                .collect(),
            signatures: signatures
                .iter()
                .map(|(name, id)| ((*name).to_string(), *id))
                .collect(),
        }
    }

    fn impl_method_ids(methods: &[(&str, DefId)]) -> HashMap<String, DefId> {
        methods
            .iter()
            .map(|(name, id)| ((*name).to_string(), *id))
            .collect()
    }

    fn field(name: &str, ty: ParseType, public: bool) -> StructDeclField {
        StructDeclField {
            name: ident(name),
            ty,
            public,
            default: None,
        }
    }

    fn ident_pattern(name: &str) -> Pattern {
        Pattern {
            binding: None,
            kind: PatternKind::Ident(ast::IdentPattern {
                name: ident(name),
                mut_: false,
            }),
        }
    }

    fn function_decl(
        name: &str,
        params: &[&str],
        self_receiver: Option<SelfReceiverMode>,
        arrow_kind: LambdaArrowKind,
    ) -> FunctionDecl {
        FunctionDecl {
            name: ident(name),
            lambda: LambdaDecl {
                parameters: params.iter().map(|name| ident_pattern(name)).collect(),
                body: ast::Block { statements: vec![] },
                arrow_kind,
                span: crate::lexer::Span::test(),
            },
            self_receiver,
            is_unsafe: false,
            exported: false,
        }
    }

    fn function_sig(
        name: &str,
        sig: ParseType,
        self_receiver: Option<SelfReceiverMode>,
        where_clauses: Vec<WhereClause>,
    ) -> FunctionSig {
        FunctionSig {
            name: ident(name),
            sig,
            where_clauses,
            self_receiver,
            is_unsafe: false,
            exported: false,
        }
    }

    #[test]
    fn collect_type_var_ids_visits_projection_base_and_trait_args() {
        let base_var = TypeVarId(10);
        let arg_var = TypeVarId(11);
        let ty = Type::Projection {
            ty: Box::new(Type::TypeVar(base_var)),
            trait_id: DefId::new(CrateId(0), LocalDefId(20)),
            assoc_type: AssociatedTypeKey {
                owner: DefId::new(CrateId(0), LocalDefId(20)),
                assoc_type_id: AssocTypeId(0),
            },
            trait_args: vec![Type::TypeVar(arg_var)],
        };
        let mut type_vars = HashSet::new();

        collect_type_var_ids(&ty, &mut type_vars);

        assert!(type_vars.contains(&base_var));
        assert!(type_vars.contains(&arg_var));
    }

    #[test]
    fn test_build_struct_lowers_struct_field_types() {
        let mut context = CollectContext::new();
        let decl = StructDecl {
            name: ParseTypeInner {
                name: "Buffer".to_string(),
                generics: vec![],
                span: Span::test(),
            },
            generic_params: vec![generic_param("T")],
            fields: vec![
                field(
                    "bytes",
                    ParseType::Reference {
                        is_mut: false,
                        pointee: Box::new(ParseType::Slice(Box::new(named_type("U8")))),
                    },
                    true,
                ),
                field(
                    "cursor",
                    ParseType::Reference {
                        is_mut: true,
                        pointee: Box::new(named_type("I64")),
                    },
                    false,
                ),
            ],
            exported: false,
        };

        let hir_struct = build_struct_with_id(&mut context, &decl, def_id(1));

        assert_eq!(hir_struct.name, "Buffer");
        assert_eq!(
            hir_struct.generic_params,
            vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: def_id(1),
                    index: 0,
                },
                "T",
            )]
        );
        assert_eq!(hir_struct.fields.len(), 2);
        assert_eq!(hir_struct.fields[0].name, "bytes");
        assert_eq!(
            hir_struct.fields[0].ty,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::Slice(Box::new(Type::U8))),
            }
        );
        assert!(hir_struct.fields[0].public);
        assert_eq!(hir_struct.fields[1].name, "cursor");
        assert_eq!(
            hir_struct.fields[1].ty,
            Type::Reference {
                mutable: true,
                inner: Box::new(Type::I64),
            }
        );
        assert!(!hir_struct.fields[1].public);
    }

    #[test]
    fn build_struct_preserves_constructor_generic_kind_and_application() {
        let owner = def_id(90);
        let constructor_param = ast::GenericParamDecl {
            name: ident("F"),
            kind: Some(ast::TypeApplication {
                constructor: Box::new(named_type("F")),
                args: vec![ast::ParseType::Hole(ast::TypeHole { span: Span::test() })],
                span: Span::test(),
            }),
            span: Span::test(),
        };
        let decl = StructDecl {
            name: type_inner("Wrapper"),
            generic_params: vec![constructor_param, generic_param("A")],
            fields: vec![field(
                "value",
                ast::ParseType::Application(ast::TypeApplication {
                    constructor: Box::new(named_type("F")),
                    args: vec![named_type("A")],
                    span: Span::test(),
                }),
                true,
            )],
            exported: false,
        };

        let strukt = build_struct_with_id(&mut CollectContext::new(), &decl, owner);

        assert_eq!(
            strukt.generic_params[0].kind,
            crate::type_services::kind::Kind::arrow(
                crate::type_services::kind::Kind::Type,
                crate::type_services::kind::Kind::Type,
            )
        );
        assert_eq!(
            strukt.fields[0].ty,
            Type::Apply {
                constructor: Box::new(Type::Generic(GenericParamId { owner, index: 0 })),
                args: vec![Type::Generic(GenericParamId { owner, index: 1 })],
            }
        );
    }

    #[test]
    fn build_struct_assigns_source_order_field_ids_per_owner() {
        let mut context = CollectContext::new();
        let left = StructDecl {
            name: type_inner("Left"),
            generic_params: vec![],
            fields: vec![field("value", named_type("I32"), true)],
            exported: false,
        };
        let right = StructDecl {
            name: type_inner("Right"),
            generic_params: vec![],
            fields: vec![
                field("value", named_type("I32"), true),
                field("other", named_type("I64"), true),
            ],
            exported: false,
        };

        let left = build_struct_with_id(&mut context, &left, def_id(1));
        let right = build_struct_with_id(&mut context, &right, def_id(2));

        assert_eq!(left.fields[0].id, FieldId(0));
        assert_eq!(right.fields[0].id, FieldId(0));
        assert_eq!(right.fields[1].id, FieldId(1));
    }

    #[test]
    fn test_build_struct_rejects_bare_str_field_type() {
        let mut context = CollectContext::new();
        let decl = StructDecl {
            name: ParseTypeInner {
                name: "TextHolder".to_string(),
                generics: vec![],
                span: Span::test(),
            },
            generic_params: vec![],
            fields: vec![field("text", named_type("Str"), true)],
            exported: false,
        };

        let hir_struct = build_struct_with_id(&mut context, &decl, def_id(1));

        assert_eq!(hir_struct.fields[0].ty, Type::Error);
        assert!(context.errors.iter().any(|err| err
            .message
            .contains("bare string slice type Str must be written behind a reference")));
    }

    #[test]
    fn test_build_struct_accepts_borrowed_str_field_type() {
        let mut context = CollectContext::new();
        let decl = StructDecl {
            name: ParseTypeInner {
                name: "TextHolder".to_string(),
                generics: vec![],
                span: Span::test(),
            },
            generic_params: vec![],
            fields: vec![field(
                "text",
                ParseType::Reference {
                    is_mut: false,
                    pointee: Box::new(named_type("Str")),
                },
                true,
            )],
            exported: false,
        };

        let hir_struct = build_struct_with_id(&mut context, &decl, def_id(1));

        assert_eq!(
            hir_struct.fields[0].ty,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::Str),
            }
        );
        assert!(context.errors.is_empty());
    }

    #[test]
    fn test_build_enum_mirrors_variant_header_construction() {
        let mut lowerer = CollectContext::new();
        let decl = EnumDecl {
            name: ParseTypeInner {
                name: "Message".to_string(),
                generics: vec![generic_type("T")],
                span: Span::test(),
            },
            variants: vec![
                EnumVariant {
                    name: generic_type_inner("Data"),
                    fields: NamedFieldsOrTypesList::NamedFields(vec![field(
                        "payload",
                        ParseType::Pointer(Box::new(named_type("U8"))),
                        true,
                    )]),
                },
                EnumVariant {
                    name: generic_type_inner("Pair"),
                    fields: NamedFieldsOrTypesList::TypesList(vec![
                        named_type("I32"),
                        ParseType::Array {
                            inner: Box::new(named_type("Bool")),
                            len: 2,
                        },
                    ]),
                },
                EnumVariant {
                    name: generic_type_inner("Empty"),
                    fields: NamedFieldsOrTypesList::TypesList(vec![]),
                },
            ],
            exported: false,
            language_items: Default::default(),
        };

        let hir_enum = build_enum_with_id(&mut lowerer, &decl, def_id(1));

        assert_eq!(hir_enum.name, "Message");
        assert_eq!(
            hir_enum.generic_params,
            vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: def_id(1),
                    index: 0,
                },
                "T",
            )]
        );
        assert_eq!(hir_enum.variants.len(), 3);
        match &hir_enum.variants[0].fields {
            HirVariantFields::Named(fields) => {
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].name, "payload");
                assert_eq!(fields[0].ty, Type::Pointer(Box::new(Type::U8)));
                assert!(fields[0].public);
            }
            other => panic!("expected named fields, got {other:?}"),
        }
        match &hir_enum.variants[1].fields {
            HirVariantFields::Positional(types) => {
                assert_eq!(
                    types,
                    &vec![Type::I32, Type::Array(Box::new(Type::Bool), 2)]
                );
            }
            other => panic!("expected positional fields, got {other:?}"),
        }
        assert!(matches!(
            hir_enum.variants[2].fields,
            HirVariantFields::Unit
        ));
    }

    #[test]
    fn build_enum_assigns_variant_ids_and_owner_unique_named_field_ids() {
        let mut context = CollectContext::new();
        let decl = EnumDecl {
            name: type_inner("Message"),
            variants: vec![
                EnumVariant {
                    name: generic_type_inner("First"),
                    fields: NamedFieldsOrTypesList::NamedFields(vec![
                        field("value", named_type("I32"), true),
                        field("shared", named_type("I64"), true),
                    ]),
                },
                EnumVariant {
                    name: generic_type_inner("Second"),
                    fields: NamedFieldsOrTypesList::NamedFields(vec![field(
                        "value",
                        named_type("Bool"),
                        true,
                    )]),
                },
            ],
            exported: false,
            language_items: Default::default(),
        };

        let hir_enum = build_enum_with_id(&mut context, &decl, def_id(1));

        assert_eq!(hir_enum.variants[0].id, VariantId(0));
        assert_eq!(hir_enum.variants[1].id, VariantId(1));
        let HirVariantFields::Named(first_fields) = &hir_enum.variants[0].fields else {
            panic!("expected first variant to have named fields");
        };
        let HirVariantFields::Named(second_fields) = &hir_enum.variants[1].fields else {
            panic!("expected second variant to have named fields");
        };
        assert_eq!(first_fields[0].id, FieldId(0));
        assert_eq!(first_fields[1].id, FieldId(1));
        assert_eq!(second_fields[0].id, FieldId(2));
    }

    #[test]
    fn test_build_function_sig_handles_curried_types_and_where_bounds() {
        let mut lowerer = CollectContext::new();
        let show_id = DefId::new(CrateId(0), LocalDefId(11));
        let eq_id = DefId::new(CrateId(0), LocalDefId(12));
        lowerer.traits.insert(
            "Show".to_string(),
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: show_id,
                name: "Show".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        );
        lowerer.traits.insert(
            "Eq".to_string(),
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: eq_id,
                name: "Eq".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        );
        let sig = function_sig(
            "compose",
            ParseType::Function(vec![
                generic_type("T"),
                named_type("I32"),
                named_type("Bool"),
                named_type("U8"),
            ]),
            Some(SelfReceiverMode::Shared),
            vec![
                WhereClause {
                    subject: generic_type("T"),
                    trait_bound: Some(named_type("Show")),
                },
                WhereClause {
                    subject: generic_type("T"),
                    trait_bound: Some(named_type("Eq")),
                },
            ],
        );

        let hir_sig = build_function_sig_with_id(&mut lowerer, &sig, def_id(20));

        assert_eq!(hir_sig.name, "compose");
        assert_eq!(
            hir_sig.generic_params,
            vec![
                GenericParamDecl::type_param(
                    GenericParamId {
                        owner: def_id(20),
                        index: 0,
                    },
                    "T",
                ),
                GenericParamDecl::type_param(
                    GenericParamId {
                        owner: def_id(20),
                        index: 1,
                    },
                    "Self",
                ),
            ]
        );
        let t_id = hir_sig.generic_params[0].id;
        let self_id = hir_sig.generic_params[1].id;
        assert_eq!(
            hir_sig.params,
            vec![
                Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::Generic(self_id)),
                },
                Type::Generic(t_id),
                Type::I32,
                Type::Bool
            ]
        );
        assert_eq!(hir_sig.ret, Type::U8);
        assert_eq!(
            hir_sig.self_receiver,
            Some(crate::types::ReceiverMode::Shared)
        );
        assert_eq!(
            hir_sig.generic_bounds.get(&t_id),
            Some(&vec![
                TraitBound {
                    trait_id: show_id,
                    type_args: Vec::new(),
                },
                TraitBound {
                    trait_id: eq_id,
                    type_args: Vec::new(),
                },
            ])
        );
    }

    #[test]
    fn test_build_function_sig_preserves_returned_function_types() {
        let mut lowerer = CollectContext::new();
        let sig = function_sig(
            "make",
            ParseType::Function(vec![
                named_type("I64"),
                ParseType::Function(vec![
                    ParseType::Unit(crate::lexer::Span::test()),
                    named_type("I64"),
                ]),
            ]),
            None,
            Vec::new(),
        );

        let hir_sig = build_function_sig_with_id(&mut lowerer, &sig, def_id(21));

        assert_eq!(hir_sig.params, vec![Type::I64]);
        assert_eq!(hir_sig.ret, Type::function(Vec::new(), Type::I64));
    }

    #[test]
    fn test_build_function_sig_omits_unknown_where_clause_trait_bounds() {
        let mut lowerer = CollectContext::new();
        let sig = function_sig(
            "identity",
            ParseType::Function(vec![generic_type("T"), generic_type("T")]),
            None,
            vec![WhereClause {
                subject: generic_type("T"),
                trait_bound: Some(named_type("Missing")),
            }],
        );

        let hir_sig = build_function_sig_with_id(&mut lowerer, &sig, def_id(22));

        assert!(hir_sig.generic_bounds.is_empty());
        assert!(lowerer
            .errors
            .iter()
            .any(|error| error.message == "unknown trait 'Missing' in where clause"));
        assert!(lowerer.errors.iter().any(|error| {
            error.message == "unknown trait 'Missing' in where clause"
                && error.span() == Some(Span::test())
        }));
    }

    #[test]
    fn test_build_function_sig_preserves_self_receiver_for_signature_backed_method() {
        let mut lowerer = CollectContext::new();
        let sig = function_sig(
            "render",
            ParseType::Function(vec![named_type("I64"), named_type("Bool")]),
            Some(SelfReceiverMode::Mut),
            vec![],
        );

        let hir_sig = build_function_sig_with_id(&mut lowerer, &sig, def_id(23));

        assert_eq!(hir_sig.self_receiver, Some(crate::types::ReceiverMode::Mut));
        assert!(matches!(
            hir_sig.params[0],
            Type::Reference { mutable: true, .. }
        ));
        assert_eq!(
            hir_sig.generic_params,
            vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: def_id(23),
                    index: 0,
                },
                "Self",
            )]
        );
        assert_eq!(hir_sig.params[1], Type::I64);
        assert_eq!(hir_sig.ret, Type::Bool);
    }

    #[test]
    fn test_build_function_sig_preserves_explicit_generic_names() {
        let mut lowerer = CollectContext::new();
        let sig = function_sig(
            "identity",
            ParseType::Function(vec![named_type("A"), named_type("A")]),
            None,
            vec![],
        );

        let hir_sig = build_function_sig_with_id(&mut lowerer, &sig, def_id(24));

        assert_eq!(
            hir_sig.generic_params,
            vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: def_id(24),
                    index: 0,
                },
                "A",
            )]
        );
    }

    #[test]
    fn test_build_function_header_tracks_function_type_vars() {
        let mut lowerer = CollectContext::new();
        let decl = function_decl("map", &["f", "x"], None, LambdaArrowKind::Curried);

        let function = build_function_header_with_id(&mut lowerer, &decl, def_id(40));

        assert_eq!(function.name, "map");
        assert!(function.is_curried);
        assert_eq!(function.params.len(), 1);
        let mut expected = HashSet::new();
        collect_type_var_ids(&function.params[0].ty, &mut expected);
        collect_type_var_ids(&function.ret_type, &mut expected);
        assert_eq!(
            lowerer.function_type_vars.get(&function.id),
            Some(&expected)
        );
    }

    #[test]
    fn test_build_function_header_uses_provided_function_id() {
        let mut lowerer = CollectContext::new();
        let decl = function_decl("map", &["f", "x"], None, LambdaArrowKind::Curried);
        let function_id = def_id(44);

        let function = build_function_header_with_id(&mut lowerer, &decl, function_id);

        assert_eq!(function.id, function_id);
    }

    #[test]
    fn test_build_function_header_with_sig_uses_provided_function_id_for_generic_owner() {
        let mut lowerer = CollectContext::new();
        let decl = function_decl("id", &["value"], None, LambdaArrowKind::Normal);
        let signature_id = def_id(11);
        let function_id = def_id(45);
        let sig_generic = GenericParamId {
            owner: signature_id,
            index: 0,
        };
        let sig = HirFunctionSig {
            id: signature_id,
            name: "id".to_string(),
            generic_params: vec![GenericParamDecl::type_param(sig_generic, "T")],
            params: vec![Type::Generic(sig_generic)],
            ret: Type::Generic(sig_generic),
            generic_bounds: HashMap::new().into(),
            self_receiver: None,
            is_unsafe: false,
        };

        let function = build_function_header_with_sig(&mut lowerer, &decl, &sig, function_id);

        assert_eq!(function.id, function_id);
        assert_eq!(
            function.generic_params,
            vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: function_id,
                    index: 0,
                },
                "T",
            )]
        );
        assert_eq!(
            function.params[0].ty,
            Type::Generic(GenericParamId {
                owner: function_id,
                index: 0,
            })
        );
        assert_eq!(
            function.ret_type,
            Type::Generic(GenericParamId {
                owner: function_id,
                index: 0,
            })
        );
    }

    #[test]
    fn test_build_function_header_with_sig_uses_signature_types_and_tracks_type_vars() {
        let mut lowerer = CollectContext::new();
        let decl = function_decl("id", &["value"], None, LambdaArrowKind::Normal);
        let sig = HirFunctionSig {
            id: DefId::new(CrateId(0), LocalDefId(1)),
            name: "id".to_string(),
            generic_params: Vec::new(),
            params: vec![Type::TypeVar(TypeVarId(41))],
            ret: Type::TypeVar(TypeVarId(99)),
            generic_bounds: HashMap::new().into(),
            self_receiver: None,
            is_unsafe: false,
        };

        let function = build_function_header_with_sig(&mut lowerer, &decl, &sig, def_id(41));

        assert_eq!(function.params[0].ty, Type::TypeVar(TypeVarId(41)));
        assert_eq!(function.ret_type, Type::TypeVar(TypeVarId(99)));
        assert_eq!(
            lowerer.function_type_vars.get(&function.id),
            Some(&HashSet::from([TypeVarId(41), TypeVarId(99)]))
        );
    }

    #[test]
    fn test_build_function_header_with_sig_keeps_self_receiver_for_signature_backed_method() {
        let mut lowerer = CollectContext::new();
        let decl = function_decl(
            "render",
            &["value"],
            Some(SelfReceiverMode::Shared),
            LambdaArrowKind::Normal,
        );
        let sig = HirFunctionSig {
            id: DefId::new(CrateId(0), LocalDefId(2)),
            name: "render".to_string(),
            generic_params: Vec::new(),
            params: vec![Type::TypeVar(TypeVarId(7)), Type::I64],
            ret: Type::Bool,
            generic_bounds: HashMap::new().into(),
            self_receiver: Some(crate::types::ReceiverMode::Shared),
            is_unsafe: false,
        };

        let function = build_function_header_with_sig(&mut lowerer, &decl, &sig, def_id(42));

        assert!(function.is_method);
        assert_eq!(
            function.self_receiver,
            Some(crate::types::ReceiverMode::Shared)
        );
        assert_eq!(function.params.len(), 2);
        assert_eq!(function.params[0].ty, Type::TypeVar(TypeVarId(7)));
        assert_eq!(function.params[1].name, "value");
        assert_eq!(function.params[1].ty, Type::I64);
        assert_eq!(function.ret_type, Type::Bool);
        assert_eq!(
            lowerer.function_type_vars.get(&function.id),
            Some(&HashSet::from([TypeVarId(7)]))
        );
    }

    #[test]
    fn test_build_function_header_with_sig_remaps_hidden_mut_self_receiver_generic() {
        let mut lowerer = CollectContext::new();
        let signature_id = def_id(46);
        let function_id = def_id(47);
        let signature_self = GenericParamId {
            owner: signature_id,
            index: 0,
        };
        let decl = function_decl(
            "set",
            &["value"],
            Some(SelfReceiverMode::Mut),
            LambdaArrowKind::Normal,
        );
        let sig = HirFunctionSig {
            id: signature_id,
            name: "set".to_string(),
            generic_params: vec![GenericParamDecl::type_param(signature_self, "Self")],
            params: vec![
                Type::Reference {
                    mutable: true,
                    inner: Box::new(Type::Generic(signature_self)),
                },
                Type::I64,
            ],
            ret: Type::Bool,
            generic_bounds: HashMap::new().into(),
            self_receiver: Some(crate::types::ReceiverMode::Mut),
            is_unsafe: false,
        };

        let function = build_function_header_with_sig(&mut lowerer, &decl, &sig, function_id);

        let expected_self = GenericParamId {
            owner: function_id,
            index: 0,
        };
        assert_eq!(
            function.generic_params,
            vec![GenericParamDecl::type_param(expected_self, "Self")]
        );
        assert_eq!(function.params[0].local_id, crate::ids::HirLocalId(0));
        assert!(function.params[0].mutable);
        assert_eq!(
            function.params[0].ty,
            Type::Reference {
                mutable: true,
                inner: Box::new(Type::Generic(expected_self)),
            }
        );
        assert_eq!(function.params[1].ty, Type::I64);
    }

    #[test]
    fn test_build_function_header_with_sig_keeps_public_generic_name_paired_after_hidden_self() {
        let mut lowerer = CollectContext::new();
        let signature_id = def_id(48);
        let function_id = def_id(49);
        let signature_self = GenericParamId {
            owner: signature_id,
            index: 0,
        };
        let signature_t = GenericParamId {
            owner: signature_id,
            index: 1,
        };
        let decl = function_decl(
            "set",
            &["value"],
            Some(SelfReceiverMode::Mut),
            LambdaArrowKind::Normal,
        );
        let sig = HirFunctionSig {
            id: signature_id,
            name: "set".to_string(),
            generic_params: vec![
                GenericParamDecl::type_param(signature_self, "Self"),
                GenericParamDecl::type_param(signature_t, "T"),
            ],
            params: vec![
                Type::Reference {
                    mutable: true,
                    inner: Box::new(Type::Generic(signature_self)),
                },
                Type::Generic(signature_t),
            ],
            ret: Type::Generic(signature_t),
            generic_bounds: HashMap::new().into(),
            self_receiver: Some(crate::types::ReceiverMode::Mut),
            is_unsafe: false,
        };

        let function = build_function_header_with_sig(&mut lowerer, &decl, &sig, function_id);

        let expected_t = GenericParamId {
            owner: function_id,
            index: 0,
        };
        let expected_self = GenericParamId {
            owner: function_id,
            index: 1,
        };
        assert_eq!(
            function.generic_params,
            vec![
                GenericParamDecl::type_param(expected_t, "T"),
                GenericParamDecl::type_param(expected_self, "Self"),
            ]
        );
        assert_eq!(
            function.params[0].ty,
            Type::Reference {
                mutable: true,
                inner: Box::new(Type::Generic(expected_self)),
            }
        );
        assert_eq!(function.params[1].ty, Type::Generic(expected_t));
        assert_eq!(function.ret_type, Type::Generic(expected_t));
    }

    #[test]
    fn test_build_function_header_with_sig_keeps_curried_signature_shape() {
        let mut lowerer = CollectContext::new();
        let decl = function_decl("compose", &["f", "x"], None, LambdaArrowKind::Curried);
        let sig = HirFunctionSig {
            id: DefId::new(CrateId(0), LocalDefId(3)),
            name: "compose".to_string(),
            generic_params: Vec::new(),
            params: vec![Type::I32, Type::Bool],
            ret: Type::U8,
            generic_bounds: HashMap::new().into(),
            self_receiver: None,
            is_unsafe: false,
        };

        let function = build_function_header_with_sig(&mut lowerer, &decl, &sig, def_id(43));

        assert!(function.is_curried);
        assert_eq!(function.params.len(), 1);
        assert_eq!(function.params[0].name, "f");
        assert_eq!(function.params[0].ty, Type::I32);
        assert_eq!(
            function.ret_type,
            Type::function(vec![Type::Bool], Type::U8)
        );
    }

    #[test]
    fn test_build_trait_uses_temporary_trait_context_and_restores_it() {
        let mut lowerer = CollectContext::new();
        lowerer.current_trait = Some("Outer".to_string());
        lowerer.current_trait_generics = vec!["X".to_string()];

        let decl = TraitDecl {
            where_clauses: Vec::new(),
            name: ParseTypeInner {
                name: "Iterable".to_string(),
                generics: vec![],
                span: Span::test(),
            },
            generic_params: vec![generic_param("T")],
            for_: None,
            associated_types: vec![AssociatedTypeDecl {
                name: ident("Item"),
                kind: None,
            }],
            methods: HashMap::from([(
                ident("next"),
                function_decl(
                    "next",
                    &[],
                    Some(SelfReceiverMode::Shared),
                    LambdaArrowKind::Normal,
                ),
            )]),
            signatures: HashMap::from([(
                ident("collect"),
                function_sig(
                    "collect",
                    ParseType::Associated {
                        base: type_inner("Self"),
                        member: ident("Item"),
                    },
                    None,
                    vec![],
                ),
            )]),
            exported: false,
            language_items: Default::default(),
        };

        let member_ids = trait_member_ids(&[("next", def_id(2))], &[("collect", def_id(3))]);
        let hir_trait = build_trait_with_id(&mut lowerer, &decl, def_id(1), &member_ids);

        assert_eq!(hir_trait.name, "Iterable");
        assert!(hir_trait.methods.contains_key("next"));
        assert!(lowerer
            .function_type_vars
            .contains_key(&hir_trait.methods["next"].id));
        assert_eq!(lowerer.current_trait.as_deref(), Some("Outer"));
        assert_eq!(lowerer.current_trait_generics, vec!["X".to_string()]);
        assert_eq!(
            hir_trait.signatures["collect"].ret,
            Type::Projection {
                ty: Box::new(Type::Generic(crate::types::GenericParamId {
                    owner: def_id(1),
                    index: 1
                })),
                trait_id: def_id(1),
                assoc_type: crate::types::AssociatedTypeKey {
                    owner: def_id(1),
                    assoc_type_id: crate::ids::AssocTypeId(0),
                },
                trait_args: vec![Type::Generic(crate::types::GenericParamId {
                    owner: def_id(1),
                    index: 0
                })],
            }
        );
    }

    #[test]
    fn build_trait_lowers_constructor_target_and_typed_supertrait_predicate() {
        let functor_id = def_id(70);
        let applicative_id = def_id(71);
        let mut context = CollectContext::new();
        context.traits.insert(
            "Functor".to_string(),
            HirTrait {
                id: functor_id,
                name: "Functor".to_string(),
                generic_params: Vec::new(),
                target: Some(GenericParamDecl::new(
                    GenericParamId {
                        owner: functor_id,
                        index: 0,
                    },
                    "F",
                    crate::type_services::kind::Kind::arrow(
                        crate::type_services::kind::Kind::Type,
                        crate::type_services::kind::Kind::Type,
                    ),
                )),
                predicates: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        );
        let target = ast::GenericParamDecl {
            name: ident("F"),
            kind: Some(ast::TypeApplication {
                constructor: Box::new(named_type("F")),
                args: vec![ast::ParseType::Hole(ast::TypeHole { span: Span::test() })],
                span: Span::test(),
            }),
            span: Span::test(),
        };
        let declaration = TraitDecl {
            name: type_inner("Applicative"),
            generic_params: Vec::new(),
            for_: Some(target),
            where_clauses: vec![ast::WhereClause {
                subject: named_type("F"),
                trait_bound: Some(named_type("Functor")),
            }],
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
            exported: false,
            language_items: Default::default(),
        };

        let trait_def = build_trait_with_id(
            &mut context,
            &declaration,
            applicative_id,
            &trait_member_ids(&[], &[]),
        );

        let target = trait_def.target.expect("constructor target");
        assert_eq!(
            target.kind,
            crate::type_services::kind::Kind::arrow(
                crate::type_services::kind::Kind::Type,
                crate::type_services::kind::Kind::Type,
            )
        );
        assert_eq!(
            trait_def.predicates,
            vec![crate::types::Predicate::Trait {
                subject: Type::Generic(target.id),
                trait_id: functor_id,
                args: Vec::new(),
            }]
        );
    }

    #[test]
    fn typed_predicate_preserves_arbitrary_subjects() {
        let owner = def_id(72);
        let trait_id = def_id(73);
        let mut context = CollectContext::new();
        context.traits.insert(
            "Show".to_string(),
            HirTrait {
                id: trait_id,
                name: "Show".to_string(),
                generic_params: Vec::new(),
                target: None,
                predicates: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        );
        let previous = context.push_generic_context(owner, vec!["T".to_string()]);
        let clauses = vec![ast::WhereClause {
            subject: ast::ParseType::Tuple(vec![named_type("T"), named_type("I64")]),
            trait_bound: Some(named_type("Show")),
        }];

        let bounds = lower_where_clauses(&mut context, &clauses);
        context.pop_generic_context(previous.0, previous.1);

        assert!(bounds.is_empty());
        assert_eq!(
            bounds.predicates,
            vec![crate::types::Predicate::Trait {
                subject: Type::Tuple(vec![
                    Type::Generic(GenericParamId { owner, index: 0 }),
                    Type::I64,
                ]),
                trait_id,
                args: Vec::new(),
            }]
        );
    }

    #[test]
    fn build_trait_method_reuses_trait_generic_without_redeclaring_it() {
        let mut context = CollectContext::new();
        let decl = TraitDecl {
            where_clauses: Vec::new(),
            name: ParseTypeInner {
                name: "Mapper".to_string(),
                generics: vec![],
                span: Span::test(),
            },
            generic_params: vec![generic_param("T")],
            for_: None,
            associated_types: vec![],
            methods: HashMap::new(),
            signatures: HashMap::from([(
                ident("map"),
                function_sig(
                    "map",
                    ParseType::Function(vec![generic_type("T"), generic_type("T")]),
                    None,
                    vec![],
                ),
            )]),
            exported: false,
            language_items: Default::default(),
        };

        let member_ids = trait_member_ids(&[], &[("map", def_id(2))]);
        let hir_trait = build_trait_with_id(&mut context, &decl, def_id(1), &member_ids);
        let expected_generic = crate::types::GenericParamId {
            owner: hir_trait.id,
            index: 0,
        };
        let sig = &hir_trait.signatures["map"];

        assert!(sig.generic_params.is_empty());
        assert_eq!(sig.params[0], Type::Generic(expected_generic));
        assert_eq!(sig.ret, Type::Generic(expected_generic));
    }

    #[test]
    fn trait_member_owned_generics_do_not_collide_across_members() {
        let mut context = CollectContext::new();
        let trait_id = def_id(1);
        let first_id = def_id(2);
        let second_id = def_id(3);
        let signature = |name| {
            function_sig(
                name,
                ParseType::Function(vec![generic_type("T"), generic_type("U")]),
                None,
                vec![],
            )
        };
        let decl = TraitDecl {
            where_clauses: Vec::new(),
            name: ParseTypeInner {
                name: "Mapper".to_string(),
                generics: vec![],
                span: Span::test(),
            },
            generic_params: vec![generic_param("T")],
            for_: None,
            associated_types: vec![],
            methods: HashMap::new(),
            signatures: HashMap::from([
                (ident("first"), signature("first")),
                (ident("second"), signature("second")),
            ]),
            exported: false,
            language_items: Default::default(),
        };
        let member_ids = trait_member_ids(&[], &[("first", first_id), ("second", second_id)]);

        let hir_trait = build_trait_with_id(&mut context, &decl, trait_id, &member_ids);
        let inherited = Type::Generic(GenericParamId {
            owner: trait_id,
            index: 0,
        });
        for (name, member_id) in [("first", first_id), ("second", second_id)] {
            let signature = &hir_trait.signatures[name];
            let member_generic = GenericParamId {
                owner: member_id,
                index: 0,
            };
            assert_eq!(
                signature.generic_params,
                vec![GenericParamDecl::type_param(member_generic, "U")]
            );
            assert_eq!(signature.params, vec![inherited.clone()]);
            assert_eq!(signature.ret, Type::Generic(member_generic));
        }
    }

    #[test]
    fn trait_member_bound_only_generic_uses_member_owner() {
        let mut context = CollectContext::new();
        let trait_id = def_id(1);
        let member_id = def_id(2);
        let marker_id = def_id(3);
        context.traits.insert(
            "Marker".to_string(),
            HirTrait {
                id: marker_id,
                name: "Marker".to_string(),
                generic_params: vec![],
                target: None,
                predicates: vec![],
                associated_types: vec![],
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        );
        let decl = TraitDecl {
            where_clauses: vec![],
            name: type_inner("Bounded"),
            generic_params: vec![generic_param("T")],
            for_: None,
            associated_types: vec![],
            methods: HashMap::new(),
            signatures: HashMap::from([(
                ident("check"),
                function_sig(
                    "check",
                    ParseType::Function(vec![generic_type("T"), generic_type("T")]),
                    None,
                    vec![WhereClause {
                        subject: generic_type("U"),
                        trait_bound: Some(named_type("Marker")),
                    }],
                ),
            )]),
            exported: false,
            language_items: Default::default(),
        };
        let member_ids = trait_member_ids(&[], &[("check", member_id)]);

        let hir_trait = build_trait_with_id(&mut context, &decl, trait_id, &member_ids);
        let signature = &hir_trait.signatures["check"];
        let member_generic = GenericParamId {
            owner: member_id,
            index: 0,
        };
        assert_eq!(
            signature.generic_params,
            vec![GenericParamDecl::type_param(member_generic, "U")]
        );
        assert_eq!(
            signature.generic_bounds[&member_generic][0].trait_id,
            marker_id
        );
    }

    #[test]
    fn constructor_kind_discovery_requires_explicit_holes() {
        let application = |arg| {
            ParseType::Application(ast::TypeApplication {
                constructor: Box::new(generic_type("F")),
                args: vec![arg],
                span: Span::test(),
            })
        };
        let mut kinds = HashMap::new();
        collect_constructor_generic_kinds(&application(generic_type("A")), &mut kinds, &|_| false);
        assert!(!kinds.contains_key("F"));

        collect_constructor_generic_kinds(
            &application(ParseType::Hole(ast::TypeHole { span: Span::test() })),
            &mut kinds,
            &|_| false,
        );
        assert_eq!(
            kinds["F"],
            crate::type_services::kind::Kind::arrow(
                crate::type_services::kind::Kind::Type,
                crate::type_services::kind::Kind::Type,
            )
        );
    }

    #[test]
    fn constructor_binder_does_not_rekind_existing_ordinary_generic() {
        let mut context = CollectContext::new();
        let marker_id = def_id(3);
        context.traits.insert(
            "Marker".to_string(),
            HirTrait {
                id: marker_id,
                name: "Marker".to_string(),
                generic_params: vec![],
                target: None,
                predicates: vec![],
                associated_types: vec![],
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        );
        let constructor_subject = ParseType::Application(ast::TypeApplication {
            constructor: Box::new(generic_type("F")),
            args: vec![ParseType::Hole(ast::TypeHole { span: Span::test() })],
            span: Span::test(),
        });
        let decl = TraitDecl {
            where_clauses: vec![],
            name: type_inner("Ordinary"),
            generic_params: vec![generic_param("F")],
            for_: None,
            associated_types: vec![],
            methods: HashMap::new(),
            signatures: HashMap::from([(
                ident("check"),
                function_sig(
                    "check",
                    ParseType::Function(vec![generic_type("T"), generic_type("T")]),
                    None,
                    vec![WhereClause {
                        subject: constructor_subject,
                        trait_bound: Some(named_type("Marker")),
                    }],
                ),
            )]),
            exported: false,
            language_items: Default::default(),
        };

        let hir_trait = build_trait_with_id(
            &mut context,
            &decl,
            def_id(1),
            &trait_member_ids(&[], &[("check", def_id(2))]),
        );

        assert_eq!(
            hir_trait.generic_params[0].kind,
            crate::type_services::kind::Kind::Type
        );
        assert!(context.errors.iter().any(|error| error.message.contains(
            "generic parameter 'F' was declared with kind Type, but its constructor binder requires Type -> Type"
        )));
    }

    #[test]
    fn build_impl_resolves_trait_id_through_import_alias() {
        let mut context = CollectContext::new();
        let trait_id = crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(10));
        let assoc_id = crate::ids::AssocTypeId(7);
        context
            .import_aliases
            .insert("Deref".to_string(), "stdlib::deref::Deref".to_string());
        context.traits.insert(
            "stdlib::deref::Deref".to_string(),
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Deref".to_string(),
                generic_params: Vec::new(),
                associated_types: vec![HirAssociatedTypeDecl {
                    id: assoc_id,
                    name: "Target".to_string(),
                    kind: crate::type_services::kind::Kind::Type,
                }],
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        );

        let imp = Impl {
            name: type_inner("Deref"),
            for_: Some(ParseType::Type(ParseTypeInner {
                name: "Vec".to_string(),
                generics: vec![generic_type("T")],
                span: Span::test(),
            })),
            associated_types: vec![AssociatedTypeDef {
                name: ident("Target"),
                kind: None,
                ty: ParseType::Reference {
                    is_mut: false,
                    pointee: Box::new(ParseType::Slice(Box::new(generic_type("T")))),
                },
            }],
            methods: HashMap::new(),
            signatures: HashMap::new(),
            where_clauses: Vec::new(),
        };

        let method_ids = impl_method_ids(&[]);
        let hir_impl = build_impl_with_id(&mut context, &imp, def_id(1), &method_ids);

        assert_eq!(hir_impl.trait_id, Some(trait_id));
        assert_eq!(hir_impl.associated_types[0].id, assoc_id);
    }

    #[test]
    fn build_impl_resolves_trait_id_through_canonical_import_alias() {
        let mut context = CollectContext::new();
        let trait_id = crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(10));
        context
            .canonical_import_aliases
            .insert("Clone".to_string(), trait_id);
        context.traits.insert(
            "stdlib::clone::Clone".to_string(),
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Clone".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        );

        let imp = Impl {
            name: type_inner("Clone"),
            for_: Some(ParseType::Type(type_inner("String"))),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
            where_clauses: Vec::new(),
        };

        let method_ids = impl_method_ids(&[]);
        let hir_impl = build_impl_with_id(&mut context, &imp, def_id(1), &method_ids);

        assert_eq!(hir_impl.trait_id, Some(trait_id));
    }

    #[test]
    fn build_trait_assigns_source_order_associated_type_ids_per_owner() {
        let mut context = CollectContext::new();
        let left = TraitDecl {
            where_clauses: Vec::new(),
            name: type_inner("Left"),
            generic_params: vec![],
            for_: None,
            associated_types: vec![AssociatedTypeDecl {
                name: ident("Item"),
                kind: None,
            }],
            methods: HashMap::new(),
            signatures: HashMap::new(),
            exported: false,
            language_items: Default::default(),
        };
        let right = TraitDecl {
            where_clauses: Vec::new(),
            name: type_inner("Right"),
            generic_params: vec![],
            for_: None,
            associated_types: vec![
                AssociatedTypeDecl {
                    name: ident("Item"),
                    kind: None,
                },
                AssociatedTypeDecl {
                    name: ident("Error"),
                    kind: None,
                },
            ],
            methods: HashMap::new(),
            signatures: HashMap::new(),
            exported: false,
            language_items: Default::default(),
        };

        let left_member_ids = trait_member_ids(&[], &[]);
        let right_member_ids = trait_member_ids(&[], &[]);
        let left = build_trait_with_id(&mut context, &left, def_id(1), &left_member_ids);
        let right = build_trait_with_id(&mut context, &right, def_id(2), &right_member_ids);

        assert_eq!(left.associated_types[0].id, AssocTypeId(0));
        assert_eq!(right.associated_types[0].id, AssocTypeId(0));
        assert_eq!(right.associated_types[1].id, AssocTypeId(1));
    }

    #[test]
    fn test_build_extern_registers_scope_entry() {
        let mut lowerer = CollectContext::new();
        let sig = function_sig(
            "puts",
            ParseType::Function(vec![
                ParseType::Pointer(Box::new(named_type("U8"))),
                named_type("I32"),
            ]),
            None,
            vec![],
        );

        let hir_extern = build_extern_with_id(&mut lowerer, &sig, sig.name.name.clone(), def_id(1));

        assert_eq!(hir_extern.name, "puts");
        assert_eq!(hir_extern.params, vec![Type::Pointer(Box::new(Type::U8))]);
        assert_eq!(hir_extern.ret, Type::I32);
        assert!(!hir_extern.variadic);
        let binding = lowerer.scope.lookup("puts").expect("extern is registered");
        assert_eq!(
            binding.ty,
            Type::function(vec![Type::Pointer(Box::new(Type::U8))], Type::I32)
        );
        assert!(!binding.mutable);
    }

    #[test]
    fn test_build_impl_registers_methods_and_tracks_function_type_vars_by_id() {
        let mut lowerer = CollectContext::new();
        let imp = Impl {
            name: ParseTypeInner {
                name: "Show".to_string(),
                generics: vec![],
                span: Span::test(),
            },
            for_: Some(named_type("Point")),
            associated_types: vec![],
            methods: HashMap::from([(
                ident("render"),
                function_decl(
                    "render",
                    &[],
                    Some(SelfReceiverMode::Shared),
                    LambdaArrowKind::Normal,
                ),
            )]),
            signatures: HashMap::new(),
            where_clauses: vec![],
        };

        let method_ids = impl_method_ids(&[("render", def_id(2))]);
        let hir_impl = build_impl_with_id(&mut lowerer, &imp, def_id(1), &method_ids);

        assert_eq!(hir_impl.type_name, "Point");
        assert_eq!(hir_impl.trait_name.as_deref(), Some("Show"));
        assert!(hir_impl.methods.contains_key("render"));
        assert!(lowerer
            .function_type_vars
            .contains_key(&hir_impl.methods["render"].id));
    }

    #[test]
    fn build_impl_assigns_source_order_associated_type_def_ids_per_owner() {
        let mut context = CollectContext::new();
        let left = Impl {
            name: type_inner("Iterable"),
            for_: Some(named_type("List")),
            associated_types: vec![AssociatedTypeDef {
                name: ident("Item"),
                kind: None,
                ty: named_type("I32"),
            }],
            methods: HashMap::new(),
            signatures: HashMap::new(),
            where_clauses: vec![],
        };
        let right = Impl {
            name: type_inner("Iterable"),
            for_: Some(named_type("Tree")),
            associated_types: vec![
                AssociatedTypeDef {
                    name: ident("Item"),
                    kind: None,
                    ty: named_type("I32"),
                },
                AssociatedTypeDef {
                    name: ident("Error"),
                    kind: None,
                    ty: named_type("Bool"),
                },
            ],
            methods: HashMap::new(),
            signatures: HashMap::new(),
            where_clauses: vec![],
        };

        let left_method_ids = impl_method_ids(&[]);
        let right_method_ids = impl_method_ids(&[]);
        let left = build_impl_with_id(&mut context, &left, def_id(1), &left_method_ids);
        let right = build_impl_with_id(&mut context, &right, def_id(2), &right_method_ids);

        assert_eq!(left.associated_types[0].id, AssocTypeId(0));
        assert_eq!(right.associated_types[0].id, AssocTypeId(0));
        assert_eq!(right.associated_types[1].id, AssocTypeId(1));
    }

    #[test]
    fn build_impl_preserves_constructor_valued_associated_type_kind() {
        let mut context = CollectContext::new();
        let box_id = def_id(210);
        let trait_id = def_id(211);
        context.structs.insert(
            "Box".to_string(),
            HirStruct {
                id: box_id,
                name: "Box".to_string(),
                generic_params: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: box_id,
                        index: 0,
                    },
                    "T",
                )],
                fields: Vec::new(),
            },
        );
        context.traits.insert(
            "Families".to_string(),
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Families".to_string(),
                generic_params: Vec::new(),
                associated_types: vec![HirAssociatedTypeDecl {
                    id: AssocTypeId(0),
                    name: "Family".to_string(),
                    kind: crate::type_services::kind::Kind::arrow(
                        crate::type_services::kind::Kind::Type,
                        crate::type_services::kind::Kind::Type,
                    ),
                }],
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        );
        let imp = Impl {
            name: type_inner("Families"),
            for_: Some(ParseType::Type(ParseTypeInner {
                name: "Box".to_string(),
                generics: vec![named_type("I64")],
                span: Span::test(),
            })),
            associated_types: vec![AssociatedTypeDef {
                name: ident("Family"),
                kind: Some(constructor_kind_syntax("Family", 1)),
                ty: named_type("Box"),
            }],
            methods: HashMap::new(),
            signatures: HashMap::new(),
            where_clauses: Vec::new(),
        };

        let hir_impl = build_impl_with_id(&mut context, &imp, def_id(212), &HashMap::new());

        assert!(context.errors.is_empty(), "{:?}", context.errors);
        assert_eq!(hir_impl.associated_types[0].id, AssocTypeId(0));
        assert_eq!(
            hir_impl.associated_types[0].kind,
            crate::type_services::kind::Kind::arrow(
                crate::type_services::kind::Kind::Type,
                crate::type_services::kind::Kind::Type,
            )
        );
        assert_eq!(
            hir_impl.associated_types[0].ty,
            Type::Constructor {
                id: box_id,
                flavor: crate::types::NominalTypeKind::Struct,
            }
        );
    }

    #[test]
    fn build_impl_accepts_constructor_generic_in_receiver_pattern() {
        let impl_id = def_id(213);
        let receiver = ParseType::Application(ast::TypeApplication {
            constructor: Box::new(named_type("F")),
            args: vec![named_type("A")],
            span: Span::test(),
        });
        let imp = Impl {
            name: type_inner("Functor"),
            for_: Some(receiver),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
            where_clauses: vec![ast::WhereClause {
                subject: ParseType::Application(constructor_kind_syntax("F", 1)),
                trait_bound: None,
            }],
        };
        let mut context = CollectContext::new();

        let hir_impl = build_impl_with_id(&mut context, &imp, impl_id, &HashMap::new());

        assert!(context.errors.is_empty(), "{:?}", context.errors);
        assert_eq!(
            hir_impl
                .type_generics
                .iter()
                .find(|param| param.name == "F")
                .unwrap()
                .kind,
            crate::type_services::kind::Kind::arrow(
                crate::type_services::kind::Kind::Type,
                crate::type_services::kind::Kind::Type,
            )
        );
        assert!(matches!(
            hir_impl.receiver_pattern,
            HirImplReceiverPattern::Exact(Type::Apply { .. })
        ));
    }

    #[test]
    fn build_impl_does_not_rekind_existing_ordinary_generic() {
        let trait_id = def_id(214);
        let impl_id = def_id(215);
        let mut context = CollectContext::new();
        context.traits.insert(
            "Holder".to_string(),
            HirTrait {
                id: trait_id,
                name: "Holder".to_string(),
                generic_params: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: trait_id,
                        index: 0,
                    },
                    "F",
                )],
                target: None,
                predicates: vec![],
                associated_types: vec![],
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        );
        let imp = Impl {
            name: ParseTypeInner {
                name: "Holder".to_string(),
                generics: vec![generic_type("F")],
                span: Span::test(),
            },
            for_: Some(named_type("I64")),
            associated_types: vec![],
            methods: HashMap::new(),
            signatures: HashMap::new(),
            where_clauses: vec![WhereClause {
                subject: ParseType::Application(constructor_kind_syntax("F", 1)),
                trait_bound: None,
            }],
        };

        let hir_impl = build_impl_with_id(&mut context, &imp, impl_id, &HashMap::new());

        assert_eq!(
            hir_impl
                .type_generics
                .iter()
                .find(|param| param.name == "F")
                .expect("impl F generic")
                .kind,
            crate::type_services::kind::Kind::Type
        );
        assert!(context.errors.iter().any(|error| error.message.contains(
            "generic parameter 'F' was declared with kind Type, but a later binder requires Type -> Type"
        )));
    }

    #[test]
    fn build_impl_records_constructor_trait_target_explicitly() {
        let trait_id = def_id(214);
        let option_id = def_id(215);
        let impl_id = def_id(216);
        let mut context = CollectContext::new();
        context.enums.insert(
            "Option".to_string(),
            HirEnum {
                id: option_id,
                name: "Option".to_string(),
                generic_params: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: option_id,
                        index: 0,
                    },
                    "T",
                )],
                variants: Vec::new(),
            },
        );
        context.traits.insert(
            "Functor".to_string(),
            HirTrait {
                id: trait_id,
                name: "Functor".to_string(),
                generic_params: Vec::new(),
                target: Some(GenericParamDecl::new(
                    GenericParamId {
                        owner: trait_id,
                        index: 0,
                    },
                    "F",
                    crate::type_services::kind::Kind::arrow(
                        crate::type_services::kind::Kind::Type,
                        crate::type_services::kind::Kind::Type,
                    ),
                )),
                predicates: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        );
        let imp = Impl {
            name: type_inner("Functor"),
            for_: Some(named_type("Option")),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
            where_clauses: Vec::new(),
        };

        let hir_impl = build_impl_with_id(&mut context, &imp, impl_id, &HashMap::new());

        assert!(context.errors.is_empty(), "{:?}", context.errors);
        assert_eq!(
            hir_impl.receiver_pattern,
            HirImplReceiverPattern::Constructor(Type::Constructor {
                id: option_id,
                flavor: crate::types::NominalTypeKind::Enum,
            })
        );
    }

    #[test]
    fn test_build_impl_tracks_receiver_and_trait_generics() {
        let mut lowerer = CollectContext::new();
        let vec_id = def_id(3);
        lowerer.structs.insert(
            "Vec".to_string(),
            HirStruct {
                id: vec_id,
                name: "Vec".to_string(),
                generic_params: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: vec_id,
                        index: 0,
                    },
                    "T",
                )],
                fields: Vec::new(),
            },
        );
        let imp = Impl {
            name: ParseTypeInner {
                name: "Show".to_string(),
                generics: vec![named_type("U8")],
                span: Span::test(),
            },
            for_: Some(ParseType::Type(ParseTypeInner {
                name: "Vec".to_string(),
                generics: vec![generic_type("T")],
                span: Span::test(),
            })),
            associated_types: vec![],
            methods: HashMap::from([(
                ident("render"),
                function_decl(
                    "render",
                    &[],
                    Some(SelfReceiverMode::Shared),
                    LambdaArrowKind::Normal,
                ),
            )]),
            signatures: HashMap::new(),
            where_clauses: vec![],
        };

        let method_ids = impl_method_ids(&[("render", def_id(2))]);
        let hir_impl = build_impl_with_id(&mut lowerer, &imp, def_id(1), &method_ids);

        assert_eq!(hir_impl.type_name, "Vec");
        assert_eq!(
            hir_impl.receiver_pattern,
            HirImplReceiverPattern::Exact(Type::Struct {
                id: vec_id,
                args: vec![Type::Generic(crate::types::GenericParamId {
                    owner: def_id(1),
                    index: 0
                })],
            })
        );
        assert_eq!(hir_impl.trait_arg_types, vec![Type::U8]);
        assert!(lowerer
            .function_type_vars
            .contains_key(&hir_impl.methods["render"].id));
    }

    #[test]
    fn build_impl_uses_marked_renamed_sized_trait_id() {
        let mut context = CollectContext::new();
        let sized_id = def_id(90);
        let vec_id = def_id(91);
        context.language_items.sized = Some(SizedLanguageItems { trait_id: sized_id });
        context.structs.insert(
            "Vec".to_string(),
            HirStruct {
                id: vec_id,
                name: "Vec".to_string(),
                generic_params: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: vec_id,
                        index: 0,
                    },
                    "T",
                )],
                fields: Vec::new(),
            },
        );
        context.traits.insert(
            "StaticLayout".to_string(),
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: sized_id,
                name: "StaticLayout".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        );
        let imp = Impl {
            name: type_inner("Vec"),
            for_: Some(ParseType::Type(ParseTypeInner {
                name: "Vec".to_string(),
                generics: vec![generic_type("T")],
                span: Span::test(),
            })),
            associated_types: vec![],
            methods: HashMap::new(),
            signatures: HashMap::new(),
            where_clauses: vec![],
        };

        let hir_impl = build_impl_with_id(&mut context, &imp, def_id(92), &impl_method_ids(&[]));

        assert_eq!(
            hir_impl.bounds[&GenericParamId {
                owner: hir_impl.id,
                index: 0,
            }],
            vec![TraitBound {
                trait_id: sized_id,
                type_args: Vec::new(),
            }]
        );
    }

    #[test]
    fn build_impl_does_not_inject_sized_bound_without_marked_bundle() {
        let mut context = CollectContext::new();
        let vec_id = def_id(93);
        context.structs.insert(
            "Vec".to_string(),
            HirStruct {
                id: vec_id,
                name: "Vec".to_string(),
                generic_params: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: vec_id,
                        index: 0,
                    },
                    "T",
                )],
                fields: Vec::new(),
            },
        );
        let imp = Impl {
            name: type_inner("Vec"),
            for_: Some(ParseType::Type(ParseTypeInner {
                name: "Vec".to_string(),
                generics: vec![generic_type("T")],
                span: Span::test(),
            })),
            associated_types: vec![],
            methods: HashMap::new(),
            signatures: HashMap::new(),
            where_clauses: vec![],
        };

        let hir_impl = build_impl_with_id(&mut context, &imp, def_id(94), &impl_method_ids(&[]));

        assert!(hir_impl.bounds.is_empty());
    }
}

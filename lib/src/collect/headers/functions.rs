use super::generic_names::{
    collect_constructor_generic_kinds, collect_generic_ids_from_type, is_known_nominal_type_name,
    parse_type_name_span,
};
use super::where_clauses::{
    lower_where_clauses, remap_generic_bounds_owner, remap_generic_param_ids_in_type,
};
use crate::ast;
use crate::collect::context::CollectContext;
use crate::hir::*;
use crate::ids::{DefId, TypeVarId};
use crate::types::{GenericParamDecl, GenericParamId, Type};
use std::collections::{HashMap, HashSet};

fn build_function_type(param_types: &[Type], ret_type: Type, is_curried: bool) -> Type {
    if is_curried {
        param_types.iter().rev().fold(ret_type, |acc, param| {
            Type::function(vec![param.clone()], acc)
        })
    } else {
        Type::function(param_types.to_vec(), ret_type)
    }
}

pub(super) fn collect_type_var_ids(ty: &Type, type_vars: &mut HashSet<TypeVarId>) {
    crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
        if let Type::TypeVar(id) = nested {
            type_vars.insert(*id);
        }
    });
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

pub(super) fn receiver_mode(mode: ast::SelfReceiverMode) -> crate::types::ReceiverMode {
    match mode {
        ast::SelfReceiverMode::Shared => crate::types::ReceiverMode::Shared,
        ast::SelfReceiverMode::Mut => crate::types::ReceiverMode::Mut,
        ast::SelfReceiverMode::Move => crate::types::ReceiverMode::Move,
    }
}

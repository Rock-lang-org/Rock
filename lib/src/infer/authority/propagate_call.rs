use std::collections::{HashMap, HashSet};

use crate::hir::{HirExpr, HirExprKind};
use crate::ids::DefId;
use crate::infer::constraints::Constraint;
use crate::infer::PartialHir;
use crate::lower::ResolveError;
use crate::types::{FunctionSafety, Type};

use super::instance_relation::{
    find_method, propagate_generic_scheme_type, propagate_instance_relation,
};
use super::type_predicates::contains_recovery_type;

pub(super) fn propagate_call_instance(
    hir: &mut PartialHir,
    result_ty: &Type,
    callee: &HirExpr,
    args: &mut [HirExpr],
    constrained_vars: &HashMap<DefId, HashSet<crate::ids::TypeVarId>>,
    errors: &mut Vec<ResolveError>,
) {
    let HirExprKind::ResolvedVar(reference) = &callee.kind else {
        return;
    };
    let crate::hir::HirVarTarget::Function(function_id) = reference.target else {
        return;
    };
    let Some(function) = hir
        .functions
        .get(&function_id)
        .cloned()
        .or_else(|| find_method(hir, Some(function_id)))
    else {
        return;
    };
    if !function.generic_params.is_empty() {
        let source_ret = hir.engine.resolve(&function.ret_type);
        if !matches!(source_ret, Type::Generic(_)) {
            let resolved_callee = hir.engine.resolve(&callee.ty);
            if let Type::Function { params, ret, .. } = resolved_callee {
                for (arg, expected) in args.iter().zip(params.iter()) {
                    propagate_argument_type(&mut hir.engine, &arg.ty, expected, errors, &arg.span);
                }
                propagate_generic_scheme_type(&mut hir.engine, &source_ret, ret.as_ref());
                let resolved_ret = hir.engine.resolve(ret.as_ref());
                let call_result = if args.len() < params.len() {
                    Type::function_with_safety(
                        params[args.len()..].to_vec(),
                        resolved_ret,
                        FunctionSafety::from_is_unsafe(function.is_unsafe),
                    )
                } else {
                    resolved_ret
                };
                let _ = hir.engine.unify(result_ty, &call_result);
            }
        }
        return;
    }
    if function.params.iter().any(|param| param.is_ref) {
        return;
    }
    let shared_vars = constrained_vars
        .get(&function_id)
        .cloned()
        .unwrap_or_default();
    let source_type = Type::function_with_safety(
        function
            .params
            .iter()
            .map(|param| hir.engine.resolve(&param.ty))
            .collect(),
        hir.engine.resolve(&function.ret_type),
        crate::types::FunctionSafety::from_is_unsafe(function.is_unsafe),
    );
    if !contains_recovery_type(&source_type) {
        let _ = hir.engine.unify(&callee.ty, &source_type);
    }
    let resolved_callee = hir.engine.resolve(&callee.ty);
    let resolved_result = hir.engine.resolve(result_ty);
    if let Type::Function { params, ret, .. } = resolved_callee {
        let mut source_partial_result = None;
        if let Type::Function {
            params: source_params,
            ret: source_ret,
            ..
        } = source_type
        {
            let mut instance_bindings = HashMap::new();
            for (instance, source) in params.iter().zip(source_params.iter()) {
                if contains_recovery_type(source) {
                    propagate_instance_relation(
                        &mut hir.engine,
                        source,
                        instance,
                        &mut instance_bindings,
                        &shared_vars,
                    );
                } else {
                    let _ = hir.engine.unify(instance, source);
                }
            }
            if contains_recovery_type(source_ret.as_ref()) {
                propagate_instance_relation(
                    &mut hir.engine,
                    source_ret.as_ref(),
                    ret.as_ref(),
                    &mut instance_bindings,
                    &shared_vars,
                );
            } else {
                let _ = hir.engine.unify(ret.as_ref(), source_ret.as_ref());
            }
            // A caller can be lowered before this body's callable predicates
            // exist. Refresh them using this instance's substitutions, never
            // by specializing the declaration's callback to one caller.
            let callback_instances = instance_bindings.clone();
            for (source_id, instance) in callback_instances {
                let native = hir.engine.resolve(&instance);
                let mut native_type = &native;
                while let Type::Reference { inner, .. } = native_type {
                    native_type = inner;
                }
                let Type::Function { params, ret, .. } = native_type else {
                    continue;
                };
                let packed = match params.as_slice() {
                    [] => Type::Unit,
                    [param] => param.clone(),
                    params => Type::Tuple(params.to_vec()),
                };
                for bound in hir.engine.get_bounds(source_id) {
                    if bound.type_args.len() != 2
                        || !hir
                            .language_items
                            .callable_protocols()
                            .any(|(_, id, _)| id == bound.trait_id)
                    {
                        continue;
                    }
                    for (source, actual) in bound.type_args.iter().zip([&packed, ret.as_ref()]) {
                        propagate_instance_relation(
                            &mut hir.engine,
                            source,
                            actual,
                            &mut instance_bindings,
                            &shared_vars,
                        );
                    }
                    let instantiated = crate::types::TraitBound {
                        trait_id: bound.trait_id,
                        type_args: bound
                            .type_args
                            .iter()
                            .map(|ty| hir.engine.resolve(ty).substitute(&instance_bindings))
                            .collect(),
                    };
                    let already_recorded = hir.constraint_store.iter().any(|(_, constraint)| {
                        matches!(constraint, Constraint::Trait { ty, bound, span, .. }
                            if span == &callee.span
                                && hir.engine.resolve(ty) == hir.engine.resolve(&instance)
                                && bound.trait_id == instantiated.trait_id
                                && bound.type_args.iter().map(|ty| hir.engine.resolve(ty)).collect::<Vec<_>>()
                                    == instantiated.type_args.iter().map(|ty| hir.engine.resolve(ty)).collect::<Vec<_>>())
                    });
                    if !already_recorded {
                        hir.constraint_store.add_trait(
                            instance.clone(),
                            instantiated,
                            callee.span.clone(),
                            "inferred callable argument",
                        );
                    }
                }
            }
            if args.len() < source_params.len() {
                source_partial_result = Some(Type::function_with_safety(
                    source_params[args.len()..].to_vec(),
                    *source_ret,
                    FunctionSafety::from_is_unsafe(function.is_unsafe),
                ));
            }
        }
        for (arg, expected) in args.iter().zip(params.iter()) {
            propagate_argument_type(&mut hir.engine, &arg.ty, expected, errors, &arg.span);
        }
        if let Some(source_result) = source_partial_result {
            let _ = hir.engine.unify(result_ty, &source_result);
        }
        let _ = hir.engine.unify(
            result_ty,
            &if args.len() < params.len() {
                Type::function_with_safety(
                    params[args.len()..].to_vec(),
                    *ret,
                    FunctionSafety::Safe,
                )
            } else {
                *ret
            },
        );
        return;
    }
    if !contains_recovery_type(&resolved_callee) && !contains_recovery_type(&resolved_result) {
        return;
    }
    let source_params = function
        .params
        .iter()
        .map(|param| hir.engine.resolve(&param.ty))
        .collect::<Vec<_>>();
    let source_ret = hir.engine.resolve(&function.ret_type);
    let instance_params = source_params;
    let instance_ret = source_ret;
    let instance_type = Type::function_with_safety(
        instance_params,
        instance_ret,
        crate::types::FunctionSafety::from_is_unsafe(function.is_unsafe),
    );
    let before = hir.engine.substitution_generation();
    let _ = hir.engine.unify(&callee.ty, &instance_type);
    let Type::Function { params, ret, .. } = instance_type else {
        return;
    };
    let instance_ret = *ret;
    for (arg, expected) in args.iter().zip(params.iter()) {
        propagate_argument_type(&mut hir.engine, &arg.ty, expected, errors, &arg.span);
    }
    let result_type = if args.len() < params.len() {
        Type::function_with_safety(
            params[args.len()..].to_vec(),
            instance_ret.clone(),
            crate::types::FunctionSafety::from_is_unsafe(function.is_unsafe),
        )
    } else {
        instance_ret
    };
    let _ = hir.engine.unify(result_ty, &result_type);
    if before == hir.engine.substitution_generation() {
        return;
    }
}

pub(super) fn propagate_argument_type(
    engine: &mut crate::infer::InferenceEngine,
    actual: &Type,
    expected: &Type,
    errors: &mut Vec<ResolveError>,
    span: &crate::lexer::Span,
) {
    if let (
        Type::Function {
            ret: actual_ret, ..
        },
        Type::Function {
            ret: expected_ret, ..
        },
    ) = (engine.resolve(actual), engine.resolve(expected))
    {
        unify_propagated_callable(
            engine,
            actual_ret.as_ref(),
            expected_ret.as_ref(),
            errors,
            "callable return",
            span,
        );
    }
    unify_propagated_callable(engine, actual, expected, errors, "callable argument", span);
}

pub(super) fn unify_propagated_callable(
    engine: &mut crate::infer::InferenceEngine,
    actual: &Type,
    expected: &Type,
    errors: &mut Vec<ResolveError>,
    context: &str,
    span: &crate::lexer::Span,
) -> bool {
    let mut probe = engine.clone_for_probe();
    match probe.unify(actual, expected) {
        Ok(()) => {
            engine.commit_probe(probe);
            true
        }
        Err(error)
            if !contains_recovery_type(&engine.resolve(actual))
                && !contains_recovery_type(&engine.resolve(expected)) =>
        {
            errors.push(ResolveError::with_span(
                format!("{context} type mismatch: {}", error.render(engine)),
                span.clone(),
            ));
            false
        }
        Err(_) => false,
    }
}

use std::collections::{HashMap, HashSet};

use crate::hir::{HirBlock, HirExpr, HirExprKind, HirStmt, HirVarRef, HirVarTarget};
use crate::ids::DefId;
use crate::infer::PartialHir;
use crate::lower::ResolveError;
use crate::types::{FunctionSafety, Type};

use super::instance_relation::{find_method, propagate_method_result};
use super::propagate_call::{
    propagate_argument_type, propagate_call_instance, unify_propagated_callable,
};

pub(super) fn propagate_block(
    hir: &mut PartialHir,
    block: &mut HirBlock,
    constrained_vars: &HashMap<DefId, HashSet<crate::ids::TypeVarId>>,
    errors: &mut Vec<ResolveError>,
) {
    for stmt in &mut block.stmts {
        match stmt {
            HirStmt::Let { value, .. }
            | HirStmt::Expr(value)
            | HirStmt::Return(Some(value))
            | HirStmt::Break(Some(value)) => propagate_expr(hir, value, constrained_vars, errors),
            HirStmt::Return(None) | HirStmt::Break(None) | HirStmt::Continue => {}
        }
    }
    block.ty = hir.engine.resolve(&block.ty);
}

fn propagate_expr(
    hir: &mut PartialHir,
    expr: &mut HirExpr,
    constrained_vars: &HashMap<DefId, HashSet<crate::ids::TypeVarId>>,
    errors: &mut Vec<ResolveError>,
) {
    if let HirExprKind::ResolvedVar(HirVarRef {
        target: HirVarTarget::Function(function_id),
        ..
    }) = &expr.kind
    {
        propagate_function_value_instance(
            hir,
            expr.ty.clone(),
            *function_id,
            constrained_vars,
            errors,
            &expr.span,
        );
    }
    match &mut expr.kind {
        HirExprKind::Call(callee, args, _) => {
            // Establish the call-site scheme before descending into deferred
            // arguments so their method authority can use the parameter type.
            propagate_call_instance(hir, &expr.ty, callee, args, constrained_vars, errors);
            let resolved = hir.engine.resolve(&callee.ty);
            let mut native = &resolved;
            while let Type::Reference { inner, .. } = native {
                native = inner;
            }
            let indirect = !matches!(&callee.kind, HirExprKind::ResolvedVar(reference) if !matches!(reference.target, HirVarTarget::Local(_)));
            if let Type::Function { params, ret, .. } = native {
                if indirect && params.len() == args.len() {
                    for (arg, expected) in args.iter().zip(params) {
                        propagate_argument_type(
                            &mut hir.engine,
                            &arg.ty,
                            expected,
                            errors,
                            &arg.span,
                        );
                    }
                    let _ = hir.engine.unify(&expr.ty, ret);
                }
            }
            if !matches!(
                callee.kind,
                HirExprKind::ResolvedVar(HirVarRef {
                    target: HirVarTarget::Function(_),
                    ..
                })
            ) {
                propagate_expr(hir, callee, constrained_vars, errors);
            }
            for arg in args.iter_mut() {
                propagate_expr(hir, arg, constrained_vars, errors);
            }
            propagate_call_instance(hir, &expr.ty, callee, args, constrained_vars, errors);
        }
        HirExprKind::MethodCall(receiver, _, args, _, target) => {
            if let Some(target) = target.as_ref() {
                propagate_method_arguments(hir, receiver, args, target);
            }
            propagate_expr(hir, receiver, constrained_vars, errors);
            for arg in args {
                propagate_expr(hir, arg, constrained_vars, errors);
            }
            if let Some(target) = target {
                propagate_method_result(hir, expr.ty.clone(), target, errors, &expr.span);
            }
        }
        HirExprKind::Try { expr, .. } => propagate_expr(hir, expr, constrained_vars, errors),
        HirExprKind::FieldAccess(receiver, _, _)
        | HirExprKind::Deref(receiver)
        | HirExprKind::Ref(_, receiver)
        | HirExprKind::Cast(receiver, _) => propagate_expr(hir, receiver, constrained_vars, errors),
        HirExprKind::TupleIndex(receiver, index) => {
            propagate_expr(hir, receiver, constrained_vars, errors);
            if let Type::Tuple(elements) = hir.engine.resolve(&receiver.ty) {
                if let Some(element) = elements.get(*index as usize) {
                    let _ = hir.engine.unify(&expr.ty, element);
                }
            }
        }
        HirExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            propagate_expr(hir, condition, constrained_vars, errors);
            propagate_block(hir, then_branch, constrained_vars, errors);
            if let Some(else_branch) = else_branch {
                propagate_block(hir, else_branch, constrained_vars, errors);
            }
        }
        HirExprKind::Match { scrutinee, arms } => {
            propagate_expr(hir, scrutinee, constrained_vars, errors);
            for arm in arms {
                if let Some(guard) = &mut arm.guard {
                    propagate_expr(hir, guard, constrained_vars, errors);
                }
                propagate_block(hir, &mut arm.body, constrained_vars, errors);
            }
        }
        HirExprKind::While { condition, body } => {
            propagate_expr(hir, condition, constrained_vars, errors);
            propagate_block(hir, body, constrained_vars, errors);
        }
        HirExprKind::For { iter, body, .. } => {
            propagate_expr(hir, iter, constrained_vars, errors);
            propagate_block(hir, body, constrained_vars, errors);
        }
        HirExprKind::Block(body) | HirExprKind::Loop(body) | HirExprKind::UnsafeBlock(body) => {
            propagate_block(hir, body, constrained_vars, errors)
        }
        HirExprKind::Lambda { params, body, .. } => {
            if let Type::Function {
                params: expected_params,
                ret,
                ..
            } = hir.engine.resolve(&expr.ty)
            {
                for (param, expected) in params.iter_mut().zip(expected_params) {
                    let _ = hir.engine.unify(&param.ty, &expected);
                    param.ty = hir.engine.resolve(&param.ty);
                }
                let _ = hir.engine.unify(&body.ty, ret.as_ref());
            }
            propagate_block(hir, body, constrained_vars, errors);
        }
        HirExprKind::Assign(left, right) | HirExprKind::BinOp(_, left, right) => {
            propagate_expr(hir, left, constrained_vars, errors);
            propagate_expr(hir, right, constrained_vars, errors);
        }
        HirExprKind::UnaryOp(_, inner) | HirExprKind::ArrayRepeat(inner, _) => {
            propagate_expr(hir, inner, constrained_vars, errors)
        }
        HirExprKind::Intrinsic { args, .. }
        | HirExprKind::TupleLiteral(args)
        | HirExprKind::ArrayLiteral(args) => {
            for arg in args {
                propagate_expr(hir, arg, constrained_vars, errors);
            }
        }
        HirExprKind::StructLiteral(_, _, fields) => {
            for field in fields {
                propagate_expr(hir, &mut field.value, constrained_vars, errors);
            }
        }
        HirExprKind::EnumVariant(_, _, args, _) => {
            for arg in args {
                propagate_expr(hir, arg, constrained_vars, errors);
            }
        }
        HirExprKind::Var(_)
        | HirExprKind::ResolvedVar(_)
        | HirExprKind::IntLiteral(_)
        | HirExprKind::FloatLiteral(_)
        | HirExprKind::BoolLiteral(_)
        | HirExprKind::StringLiteral(_)
        | HirExprKind::CharLiteral(_)
        | HirExprKind::Unit => {}
    }
    expr.ty = hir.engine.resolve(&expr.ty);
}

fn propagate_function_value_instance(
    hir: &mut PartialHir,
    value_ty: Type,
    function_id: DefId,
    _constrained_vars: &HashMap<DefId, HashSet<crate::ids::TypeVarId>>,
    errors: &mut Vec<ResolveError>,
    span: &crate::lexer::Span,
) {
    let Some(function) = hir
        .functions
        .get(&function_id)
        .cloned()
        .or_else(|| find_method(hir, Some(function_id)))
    else {
        return;
    };
    if !function.generic_params.is_empty() {
        return;
    }
    let source = Type::function_with_safety(
        function
            .params
            .iter()
            .map(|param| param.ty.clone())
            .collect(),
        function.ret_type.clone(),
        FunctionSafety::from_is_unsafe(function.is_unsafe),
    );
    unify_propagated_callable(
        &mut hir.engine,
        &value_ty,
        &source,
        errors,
        &format!("named callable '{}'", function.name),
        span,
    );
}

fn propagate_method_arguments(
    hir: &mut PartialHir,
    receiver: &HirExpr,
    args: &[HirExpr],
    target: &crate::hir::HirMethodCallTarget,
) {
    let Some(method) = find_method(hir, target.method_id()) else {
        return;
    };
    let substitutions = target
        .owner_substitution
        .iter()
        .chain(&target.method_substitution)
        .map(|binding| (binding.param, binding.ty.clone()))
        .collect::<HashMap<_, _>>();
    let (receiver_param, params) = if method.self_receiver.is_some() {
        (method.params.first(), &method.params[1..])
    } else {
        (None, &method.params[..])
    };
    if let Some(param) = receiver_param {
        let mut expected = param.ty.substitute_generics(&substitutions);
        if matches!(hir.engine.resolve(&receiver.ty), Type::TypeVar(_))
            && !method.ret_type.contains_reference()
        {
            if let Type::Reference { inner, .. } = expected {
                expected = *inner;
            }
        }
        let _ = hir.engine.unify(&receiver.ty, &expected);
    }
    for (arg, param) in args.iter().zip(params) {
        if let Type::TypeVar(id) = hir.engine.resolve(&param.ty) {
            if hir.engine.get_bounds(id).iter().any(|bound| {
                hir.language_items
                    .callable_protocols()
                    .any(|(_, trait_id, _)| trait_id == bound.trait_id)
            }) {
                // This callable parameter has an independently instantiated
                // bound at the call site; do not monomorphize its declaration.
                continue;
            }
        }
        let expected = param.ty.substitute_generics(&substitutions);
        let _ = hir.engine.unify(&arg.ty, &expected);
    }
}

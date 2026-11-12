//! Caller agreement for inferred (signatureless) function headers.
//!
//! Instance propagation seeds a signature variable that the body constrains
//! from its callers. That is only sound while every direct caller agrees on the
//! type: with `f "hi"` and `f 'c'` the header must stay generic instead of
//! being committed to the first caller.

use std::collections::{HashMap, HashSet};

use crate::hir::{HirBlock, HirExpr, HirExprKind, HirStmt, HirVarRef, HirVarTarget};
use crate::ids::{DefId, TypeVarId};
use crate::infer::{InferenceEngine, PartialHir};
use crate::types::Type;

/// Argument types of every direct call, keyed by callee.
fn collect_call_arguments(hir: &PartialHir) -> HashMap<DefId, Vec<Vec<Type>>> {
    let mut calls = HashMap::new();
    let bodies = hir
        .functions
        .values()
        .chain(hir.impls.values().flat_map(|imp| imp.methods.values()))
        .chain(hir.traits.values().flat_map(|tr| tr.methods.values()))
        .map(|function| &function.body);
    for body in bodies {
        block(body, &mut calls);
    }
    calls
}

fn block(node: &HirBlock, calls: &mut HashMap<DefId, Vec<Vec<Type>>>) {
    for stmt in &node.stmts {
        match stmt {
            HirStmt::Let { value, .. }
            | HirStmt::Expr(value)
            | HirStmt::Return(Some(value))
            | HirStmt::Break(Some(value)) => expr(value, calls),
            HirStmt::Return(None) | HirStmt::Break(None) | HirStmt::Continue => {}
        }
    }
}

fn expr(node: &HirExpr, calls: &mut HashMap<DefId, Vec<Vec<Type>>>) {
    match &node.kind {
        HirExprKind::Call(callee, args, _) => {
            if let HirExprKind::ResolvedVar(HirVarRef {
                target: HirVarTarget::Function(function_id),
                ..
            }) = &callee.kind
            {
                calls
                    .entry(*function_id)
                    .or_default()
                    .push(args.iter().map(|arg| arg.ty.clone()).collect());
            } else {
                expr(callee, calls);
            }
            for arg in args {
                expr(arg, calls);
            }
        }
        HirExprKind::MethodCall(receiver, _, args, _, _) => {
            expr(receiver, calls);
            for arg in args {
                expr(arg, calls);
            }
        }
        HirExprKind::Try { expr: inner, .. }
        | HirExprKind::FieldAccess(inner, _, _)
        | HirExprKind::Deref(inner)
        | HirExprKind::Ref(_, inner)
        | HirExprKind::Cast(inner, _)
        | HirExprKind::TupleIndex(inner, _)
        | HirExprKind::UnaryOp(_, inner)
        | HirExprKind::ArrayRepeat(inner, _) => expr(inner, calls),
        HirExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            expr(condition, calls);
            block(then_branch, calls);
            if let Some(else_branch) = else_branch {
                block(else_branch, calls);
            }
        }
        HirExprKind::Match { scrutinee, arms } => {
            expr(scrutinee, calls);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    expr(guard, calls);
                }
                block(&arm.body, calls);
            }
        }
        HirExprKind::While { condition, body } => {
            expr(condition, calls);
            block(body, calls);
        }
        HirExprKind::For { iter, body, .. } => {
            expr(iter, calls);
            block(body, calls);
        }
        HirExprKind::Block(body)
        | HirExprKind::Loop(body)
        | HirExprKind::UnsafeBlock(body)
        | HirExprKind::Lambda { body, .. } => block(body, calls),
        HirExprKind::Assign(left, right) | HirExprKind::BinOp(_, left, right) => {
            expr(left, calls);
            expr(right, calls);
        }
        HirExprKind::Intrinsic { args, .. }
        | HirExprKind::TupleLiteral(args)
        | HirExprKind::ArrayLiteral(args)
        | HirExprKind::EnumVariant(_, _, args, _) => {
            for arg in args {
                expr(arg, calls);
            }
        }
        HirExprKind::StructLiteral(_, _, fields) => {
            for field in fields {
                expr(&field.value, calls);
            }
        }
        HirExprKind::Var(_)
        | HirExprKind::ResolvedVar(_)
        | HirExprKind::IntLiteral(_)
        | HirExprKind::ConstParam(_)
        | HirExprKind::FloatLiteral(_)
        | HirExprKind::BoolLiteral(_)
        | HirExprKind::StringLiteral(_)
        | HirExprKind::CharLiteral(_)
        | HirExprKind::Unit => {}
    }
}

/// Record the instance type each source variable takes in one caller argument.
fn match_instance(
    engine: &InferenceEngine,
    source: &Type,
    instance: &Type,
    out: &mut HashMap<TypeVarId, Vec<Type>>,
) {
    let source = engine.resolve(source);
    let instance = engine.resolve(instance);
    match (&source, &instance) {
        (Type::TypeVar(id), _) => out.entry(*id).or_default().push(instance),
        (Type::Reference { inner: source, .. }, Type::Reference { inner: instance, .. })
        | (Type::Pointer(source), Type::Pointer(instance))
        | (Type::Slice(source), Type::Slice(instance))
        | (Type::Array(source, _), Type::Array(instance, _)) => {
            match_instance(engine, source, instance, out)
        }
        (Type::Tuple(sources), Type::Tuple(instances))
        | (
            Type::Struct { args: sources, .. },
            Type::Struct { args: instances, .. },
        )
        | (Type::Enum { args: sources, .. }, Type::Enum { args: instances, .. }) => {
            for (source, instance) in sources.iter().zip(instances) {
                match_instance(engine, source, instance, out);
            }
        }
        (
            Type::Function {
                params: sources,
                ret: source_ret,
                ..
            },
            Type::Function {
                params: instances,
                ret: instance_ret,
                ..
            },
        ) => {
            for (source, instance) in sources.iter().zip(instances) {
                match_instance(engine, source, instance, out);
            }
            match_instance(engine, source_ret, instance_ret, out);
        }
        _ => {}
    }
}

/// Signature variables of each inferred function whose direct callers supply
/// mutually incompatible types. Such variables must not be seeded from callers.
pub(super) fn conflicting_caller_vars(hir: &PartialHir) -> HashSet<TypeVarId> {
    let calls = collect_call_arguments(hir);
    let mut conflicting = HashSet::new();
    for (function_id, call_args) in calls {
        if call_args.len() < 2 {
            continue;
        }
        let Some(function) = hir.functions.get(&function_id) else {
            continue;
        };
        if !function.generic_params.is_empty() {
            continue;
        }
        let mut instances = HashMap::<TypeVarId, Vec<Type>>::new();
        for args in &call_args {
            for (param, arg) in function.params.iter().zip(args) {
                match_instance(&hir.engine, &param.ty, arg, &mut instances);
            }
        }
        for (var, types) in instances {
            let mut probe = hir.engine.clone_for_probe();
            let mut iter = types.iter();
            let Some(first) = iter.next() else {
                continue;
            };
            if iter.any(|other| probe.unify(first, other).is_err()) {
                conflicting.insert(var);
            }
        }
    }
    conflicting
}

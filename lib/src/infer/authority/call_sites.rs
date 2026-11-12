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
fn collect_call_arguments(hir: &PartialHir) -> HashMap<DefId, Vec<Vec<(Type, crate::lexer::Span)>>> {
    let mut calls = HashMap::<DefId, Vec<Vec<(Type, crate::lexer::Span)>>>::new();
    for function in all_functions(hir) {
        block(&function.body, &mut |node| {
            if let HirExprKind::Call(callee, args, _) = &node.kind {
                if let HirExprKind::ResolvedVar(HirVarRef {
                    target: HirVarTarget::Function(function_id),
                    ..
                }) = &callee.kind
                {
                    calls
                        .entry(*function_id)
                        .or_default()
                        .push(args.iter().map(|arg| (arg.ty.clone(), arg.span.clone())).collect());
                }
            }
        });
    }
    calls
}

fn all_functions(hir: &PartialHir) -> impl Iterator<Item = &crate::hir::HirFunction> {
    hir.functions
        .values()
        .chain(hir.impls.values().flat_map(|imp| imp.methods.values()))
        .chain(hir.traits.values().flat_map(|tr| tr.methods.values()))
}

fn block(node: &HirBlock, calls: &mut dyn FnMut(&HirExpr)) {
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

fn expr(node: &HirExpr, calls: &mut dyn FnMut(&HirExpr)) {
    calls(node);
    match &node.kind {
        HirExprKind::Call(callee, args, _) => {
            expr(callee, calls);
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

/// Variables the body uses as a field receiver. A field needs one concrete
/// struct, so these stay caller-seeded: a disagreeing caller is then a type
/// error at its call instead of a generic body that cannot be instantiated.
pub(super) fn field_receiver_vars(
    hir: &PartialHir,
    function: &crate::hir::HirFunction,
) -> HashSet<TypeVarId> {
    let mut vars = HashSet::new();
    // `x.name args` is still `Call(FieldAccess ..)` before materialization:
    // a method call, not a field use. The walk is pre-order, so a call's
    // callee span is known before the callee itself is visited.
    let mut method_callees = HashSet::new();
    block(&function.body, &mut |node| {
        if let HirExprKind::Call(callee, _, _) = &node.kind {
            if matches!(callee.kind, HirExprKind::FieldAccess(_, _, None)) {
                method_callees.insert((callee.span.start, callee.span.end));
            }
        }
        if method_callees.contains(&(node.span.start, node.span.end)) {
            return;
        }
        if let HirExprKind::FieldAccess(receiver, _, _) = &node.kind {
            let mut ty = hir.engine.resolve(&receiver.ty);
            while let Type::Reference { inner, .. } = ty {
                ty = *inner;
            }
            if let Type::TypeVar(id) = ty {
                vars.insert(id);
            }
        }
    });
    vars
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
        let struct_bound = field_receiver_vars(hir, function);
        let mut instances = HashMap::<TypeVarId, Vec<Type>>::new();
        for args in &call_args {
            for (param, (arg, _)) in function.params.iter().zip(args) {
                match_instance(&hir.engine, &param.ty, arg, &mut instances);
            }
        }
        for (var, types) in instances {
            if struct_bound.contains(&var) {
                continue;
            }
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

/// After inference, every direct call to a non-generic function must agree with
/// its final parameter types. Caller seeding is unchecked, so a disagreement
/// that inference could not generalize away is reported here, not in codegen.
pub(in crate::infer) fn check_monomorphic_call_arguments(
    hir: &PartialHir,
) -> Vec<crate::lower::ResolveError> {
    let mut errors = Vec::new();
    let mut calls = collect_call_arguments(hir).into_iter().collect::<Vec<_>>();
    calls.sort_by_key(|(id, _)| *id);
    for (function_id, call_args) in calls {
        let Some(function) = hir.functions.get(&function_id) else {
            continue;
        };
        if !function.generic_params.is_empty() {
            continue;
        }
        for args in call_args {
            for (param, (arg, span)) in function.params.iter().zip(args) {
                let expected = hir.engine.resolve(&param.ty);
                let actual = hir.engine.resolve(&arg);
                let unresolved = |ty: &Type| {
                    crate::type_services::visit::type_any(ty, |ty| {
                        matches!(ty, Type::TypeVar(_) | Type::Generic(_))
                    })
                };
                if unresolved(&expected) || unresolved(&actual) {
                    continue;
                }
                let mut probe = hir.engine.clone_for_probe();
                if let Err(error) = probe.unify(&expected, &actual) {
                    errors.push(crate::lower::ResolveError::with_span_code(
                        format!(
                            "argument to '{}' does not match its inferred parameter type: {}",
                            function.name,
                            error.render(&hir.engine)
                        ),
                        span,
                        crate::diagnostic::DiagnosticCode::Type,
                    ));
                }
            }
        }
    }
    errors
}


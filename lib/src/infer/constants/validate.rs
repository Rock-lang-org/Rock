use std::collections::{BTreeSet, HashMap};

use crate::diagnostic::DiagnosticCode;
use crate::hir::*;
use crate::ids::DefId;
use crate::lower::ResolveError;

use super::evaluate::evaluate;
use super::summary::ArithmeticSummary;
use super::Environment;

pub(super) struct Check<'a> {
    program: &'a HirProgram,
    pub(super) summaries: &'a HashMap<DefId, ArithmeticSummary>,
}

impl std::ops::Deref for Check<'_> {
    type Target = HirProgram;
    fn deref(&self) -> &Self::Target {
        self.program
    }
}

pub(in crate::infer) fn validate(
    program: &HirProgram,
    current: &BTreeSet<DefId>,
    summaries: &HashMap<DefId, ArithmeticSummary>,
) -> Vec<ResolveError> {
    let program = &Check { program, summaries };
    let mut errors = Vec::new();
    let mut functions = program
        .functions
        .values()
        .chain(program.impls.values().flat_map(|imp| imp.methods.values()))
        .chain(program.traits.values().flat_map(|trt| trt.methods.values()))
        .filter(|function| current.contains(&function.id))
        .collect::<Vec<_>>();
    functions.sort_by_key(|function| function.id);
    for function in functions {
        scan_block(program, &function.body, &mut errors);
    }
    errors
}

fn scan_block(program: &Check<'_>, block: &HirBlock, errors: &mut Vec<ResolveError>) {
    for stmt in &block.stmts {
        match stmt {
            HirStmt::Let { value, .. }
            | HirStmt::Expr(value)
            | HirStmt::Return(Some(value))
            | HirStmt::Break(Some(value)) => scan(program, value, errors),
            _ => {}
        }
    }
}

fn scan(program: &Check<'_>, expr: &HirExpr, errors: &mut Vec<ResolveError>) {
    let previous_errors = errors.len();
    match &expr.kind {
        HirExprKind::BinOp(_, lhs, rhs) | HirExprKind::Assign(lhs, rhs) => {
            scan(program, lhs, errors);
            scan(program, rhs, errors);
        }
        HirExprKind::Call(function, args, _) => {
            scan(program, function, errors);
            for arg in args {
                scan(program, arg, errors);
            }
        }
        HirExprKind::MethodCall(receiver, _, args, _, _) => {
            scan(program, receiver, errors);
            for arg in args {
                scan(program, arg, errors);
            }
        }
        HirExprKind::UnaryOp(_, inner)
        | HirExprKind::FieldAccess(inner, _, _)
        | HirExprKind::TupleIndex(inner, _)
        | HirExprKind::ArrayRepeat(inner, _)
        | HirExprKind::Ref(_, inner)
        | HirExprKind::Deref(inner)
        | HirExprKind::Cast(inner, _)
        | HirExprKind::Try { expr: inner, .. } => scan(program, inner, errors),
        HirExprKind::ArrayLiteral(args)
        | HirExprKind::TupleLiteral(args)
        | HirExprKind::EnumVariant(_, _, args, _)
        | HirExprKind::Intrinsic { args, .. } => {
            for arg in args {
                scan(program, arg, errors);
            }
        }
        HirExprKind::StructLiteral(_, _, fields) => {
            for field in fields {
                scan(program, &field.value, errors);
            }
        }
        HirExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            scan(program, condition, errors);
            scan_block(program, then_branch, errors);
            if let Some(block) = else_branch {
                scan_block(program, block, errors);
            }
        }
        HirExprKind::Match { scrutinee, arms } => {
            scan(program, scrutinee, errors);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    scan(program, guard, errors);
                }
                scan_block(program, &arm.body, errors);
            }
        }
        HirExprKind::While { condition, body } => {
            scan(program, condition, errors);
            scan_block(program, body, errors);
        }
        HirExprKind::For { iter, body, .. } => {
            scan(program, iter, errors);
            scan_block(program, body, errors);
        }
        HirExprKind::Loop(body)
        | HirExprKind::Block(body)
        | HirExprKind::UnsafeBlock(body)
        | HirExprKind::Lambda { body, .. } => scan_block(program, body, errors),
        _ => {}
    }
    // Diagnose the innermost failing application, not a surrounding print/call
    // or the dependency's implementation; valid operands can still fail here.
    if errors.len() == previous_errors
        && matches!(
            expr.kind,
            HirExprKind::MethodCall(..) | HirExprKind::Intrinsic { .. }
        )
    {
        if let Err(reason) = evaluate(program, expr, &Environment::new(), &mut 512, 0) {
            errors.push(ResolveError::with_span_code(
                format!("invalid constant integer arithmetic: {reason}"),
                arithmetic_span(expr),
                DiagnosticCode::Type,
            ));
        }
    }
}

fn arithmetic_span(expr: &HirExpr) -> crate::lexer::Span {
    let mut span = expr.span.clone();
    let mut include = |child: &HirExpr| {
        let child = arithmetic_span(child);
        if child.file_path == span.file_path {
            span.start = span.start.min(child.start);
            span.end = span.end.max(child.end);
        }
    };
    match &expr.kind {
        HirExprKind::MethodCall(receiver, _, args, _, _) => {
            include(receiver);
            for arg in args {
                include(arg);
            }
        }
        HirExprKind::Intrinsic { args, .. } => {
            for arg in args {
                include(arg);
            }
        }
        HirExprKind::Cast(inner, _)
        | HirExprKind::Ref(_, inner)
        | HirExprKind::Deref(inner)
        | HirExprKind::TupleIndex(inner, _) => include(inner),
        _ => {}
    }
    span
}

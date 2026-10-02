//! Type variable collection and replacement utilities (free functions).

use std::collections::HashMap;

use crate::hir::*;
use crate::ids::TypeVarId;
use crate::types::{GenericParamId, Type};

use super::InferenceEngine;

fn replace_type_vars_in_pattern_composite(
    engine: &InferenceEngine,
    pattern: &mut HirPattern,
    mapping: &HashMap<TypeVarId, GenericParamId>,
    composite_types: &HashMap<TypeVarId, Type>,
) {
    match pattern {
        HirPattern::Tuple(patterns) | HirPattern::Or(patterns) => {
            for pattern in patterns {
                replace_type_vars_in_pattern_composite(engine, pattern, mapping, composite_types);
            }
        }
        HirPattern::Struct(_, _, type_args, field_patterns) => {
            for ty in type_args {
                *ty =
                    replace_type_vars_with_generics_composite(engine, ty, mapping, composite_types);
            }
            for field in field_patterns {
                replace_type_vars_in_pattern_composite(
                    engine,
                    &mut field.pattern,
                    mapping,
                    composite_types,
                );
            }
        }
        HirPattern::Enum(_, _, _, patterns) => {
            for pattern in patterns {
                replace_type_vars_in_pattern_composite(engine, pattern, mapping, composite_types);
            }
        }
        HirPattern::Wildcard | HirPattern::Binding { .. } | HirPattern::Literal(_) => {}
    }
}

/// Replace TypeVar nodes with Generic nodes, handling composite types.
/// Delegates to `Type::replace_type_vars_to_generics` — the canonical implementation.
pub(super) fn replace_type_vars_with_generics_composite(
    engine: &InferenceEngine,
    ty: &Type,
    mapping: &HashMap<TypeVarId, GenericParamId>,
    composite_types: &HashMap<TypeVarId, Type>,
) -> Type {
    engine
        .resolve(ty)
        .replace_type_vars_to_generics(mapping, composite_types)
}

/// Replace TypeVar nodes with Generic nodes in a block (composite-aware)
pub(super) fn replace_type_vars_in_block_composite(
    engine: &InferenceEngine,
    block: &mut HirBlock,
    mapping: &HashMap<TypeVarId, GenericParamId>,
    composite_types: &HashMap<TypeVarId, Type>,
) {
    block.ty =
        replace_type_vars_with_generics_composite(engine, &block.ty, mapping, composite_types);
    for stmt in &mut block.stmts {
        replace_type_vars_in_stmt_composite(engine, stmt, mapping, composite_types);
    }
}

/// Replace TypeVar nodes with Generic nodes in a statement (composite-aware)
fn replace_type_vars_in_stmt_composite(
    engine: &InferenceEngine,
    stmt: &mut HirStmt,
    mapping: &HashMap<TypeVarId, GenericParamId>,
    composite_types: &HashMap<TypeVarId, Type>,
) {
    match stmt {
        HirStmt::Let { ty, value, .. } => {
            *ty = replace_type_vars_with_generics_composite(engine, ty, mapping, composite_types);
            replace_type_vars_in_expr_composite(engine, value, mapping, composite_types);
        }
        HirStmt::Expr(e) => {
            replace_type_vars_in_expr_composite(engine, e, mapping, composite_types)
        }
        HirStmt::Return(Some(e)) => {
            replace_type_vars_in_expr_composite(engine, e, mapping, composite_types)
        }
        HirStmt::Break(Some(e)) => {
            replace_type_vars_in_expr_composite(engine, e, mapping, composite_types)
        }
        _ => {}
    }
}

/// Replace TypeVar nodes with Generic nodes in an expression (composite-aware)
fn replace_type_vars_in_expr_composite(
    engine: &InferenceEngine,
    expr: &mut HirExpr,
    mapping: &HashMap<TypeVarId, GenericParamId>,
    composite_types: &HashMap<TypeVarId, Type>,
) {
    expr.ty = replace_type_vars_with_generics_composite(engine, &expr.ty, mapping, composite_types);
    match &mut expr.kind {
        HirExprKind::Open { source, binding, body } => {
            replace_type_vars_in_expr_composite(engine, source, mapping, composite_types);
            binding.visit_types_mut(&mut |ty| *ty = replace_type_vars_with_generics_composite(engine, ty, mapping, composite_types));
            replace_type_vars_in_block_composite(engine, body, mapping, composite_types);
        }
        HirExprKind::OwnedObjectCall { owner, args, call } => {
            replace_type_vars_in_expr_composite(engine, owner, mapping, composite_types);
            for arg in args {
                replace_type_vars_in_expr_composite(engine, arg, mapping, composite_types);
            }
            call.visit_types_mut(&mut |ty| {
                *ty =
                    replace_type_vars_with_generics_composite(engine, ty, mapping, composite_types)
            });
        }
        HirExprKind::BinOp(_, lhs, rhs) => {
            replace_type_vars_in_expr_composite(engine, lhs, mapping, composite_types);
            replace_type_vars_in_expr_composite(engine, rhs, mapping, composite_types);
        }
        HirExprKind::UnaryOp(_, inner) => {
            replace_type_vars_in_expr_composite(engine, inner, mapping, composite_types)
        }
        HirExprKind::Call(func, args, target) => {
            replace_type_vars_in_expr_composite(engine, func, mapping, composite_types);
            for arg in args {
                replace_type_vars_in_expr_composite(engine, arg, mapping, composite_types);
            }
            if let Some(HirCallTarget::StaticMethod(target)) = target {
                target.owner_ty = replace_type_vars_with_generics_composite(
                    engine,
                    &target.owner_ty,
                    mapping,
                    composite_types,
                );
                replace_method_target_types(engine, &mut target.method, mapping, composite_types);
            }
        }
        HirExprKind::MethodCall(recv, _, args, _, target) => {
            replace_type_vars_in_expr_composite(engine, recv, mapping, composite_types);
            for arg in args {
                replace_type_vars_in_expr_composite(engine, arg, mapping, composite_types);
            }
            if let Some(target) = target {
                replace_method_target_types(engine, target, mapping, composite_types);
            }
        }
        HirExprKind::FieldAccess(e, _, _) => {
            replace_type_vars_in_expr_composite(engine, e, mapping, composite_types)
        }
        HirExprKind::TupleIndex(e, _) => {
            replace_type_vars_in_expr_composite(engine, e, mapping, composite_types)
        }
        HirExprKind::ArrayLiteral(elems) => {
            for e in elems {
                replace_type_vars_in_expr_composite(engine, e, mapping, composite_types);
            }
        }
        HirExprKind::ArrayRepeat(value, _) => {
            replace_type_vars_in_expr_composite(engine, value, mapping, composite_types);
        }
        HirExprKind::TupleLiteral(elems) => {
            for e in elems {
                replace_type_vars_in_expr_composite(engine, e, mapping, composite_types);
            }
        }
        HirExprKind::StructLiteral(_, _, fields) => {
            for field in fields {
                replace_type_vars_in_expr_composite(
                    engine,
                    &mut field.value,
                    mapping,
                    composite_types,
                );
            }
        }
        HirExprKind::EnumVariant(_, _, args, _) => {
            for e in args {
                replace_type_vars_in_expr_composite(engine, e, mapping, composite_types);
            }
        }
        HirExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            replace_type_vars_in_expr_composite(engine, condition, mapping, composite_types);
            replace_type_vars_in_block_composite(engine, then_branch, mapping, composite_types);
            if let Some(else_b) = else_branch {
                replace_type_vars_in_block_composite(engine, else_b, mapping, composite_types);
            }
        }
        HirExprKind::Match { scrutinee, arms } => {
            replace_type_vars_in_expr_composite(engine, scrutinee, mapping, composite_types);
            for arm in arms {
                replace_type_vars_in_pattern_composite(
                    engine,
                    &mut arm.pattern,
                    mapping,
                    composite_types,
                );
                replace_type_vars_in_block_composite(
                    engine,
                    &mut arm.body,
                    mapping,
                    composite_types,
                );
                if let Some(guard) = &mut arm.guard {
                    replace_type_vars_in_expr_composite(engine, guard, mapping, composite_types);
                }
            }
        }
        HirExprKind::While { condition, body } => {
            replace_type_vars_in_expr_composite(engine, condition, mapping, composite_types);
            replace_type_vars_in_block_composite(engine, body, mapping, composite_types);
        }
        HirExprKind::For { iter, body, .. } => {
            replace_type_vars_in_expr_composite(engine, iter, mapping, composite_types);
            replace_type_vars_in_block_composite(engine, body, mapping, composite_types);
        }
        HirExprKind::Loop(body) => {
            replace_type_vars_in_block_composite(engine, body, mapping, composite_types)
        }
        HirExprKind::Block(body) | HirExprKind::UnsafeBlock(body) => {
            replace_type_vars_in_block_composite(engine, body, mapping, composite_types)
        }
        HirExprKind::Lambda {
            params,
            body,
            captures,
        } => {
            for p in params {
                p.ty = replace_type_vars_with_generics_composite(
                    engine,
                    &p.ty,
                    mapping,
                    composite_types,
                );
            }
            for capture in captures {
                capture.ty = replace_type_vars_with_generics_composite(
                    engine,
                    &capture.ty,
                    mapping,
                    composite_types,
                );
            }
            replace_type_vars_in_block_composite(engine, body, mapping, composite_types);
        }
        HirExprKind::Ref(_, inner) | HirExprKind::Deref(inner) => {
            replace_type_vars_in_expr_composite(engine, inner, mapping, composite_types);
        }
        HirExprKind::Cast(inner, ty) => {
            replace_type_vars_in_expr_composite(engine, inner, mapping, composite_types);
            *ty = replace_type_vars_with_generics_composite(engine, ty, mapping, composite_types);
        }
        HirExprKind::ObjectCoercion(inner, coercion) => {
            replace_type_vars_in_expr_composite(engine, inner, mapping, composite_types);
            coercion.visit_types_mut(&mut |ty| {
                *ty =
                    replace_type_vars_with_generics_composite(engine, ty, mapping, composite_types)
            });
        }
        HirExprKind::Assign(lhs, rhs) => {
            replace_type_vars_in_expr_composite(engine, lhs, mapping, composite_types);
            replace_type_vars_in_expr_composite(engine, rhs, mapping, composite_types);
        }
        HirExprKind::Intrinsic { args, .. } => {
            for arg in args {
                replace_type_vars_in_expr_composite(engine, arg, mapping, composite_types);
            }
        }
        HirExprKind::Try {
            expr,
            branch_method,
            branch_target,
            from_residual_target,
            output_ty,
            residual_ty,
            return_ty,
            ..
        } => {
            replace_type_vars_in_expr_composite(engine, expr, mapping, composite_types);
            if let Some(target) = branch_method {
                replace_method_target_types(engine, target, mapping, composite_types);
            }
            for target in [branch_target, from_residual_target].into_iter().flatten() {
                if let HirCallTarget::StaticMethod(target) = target {
                    target.owner_ty = replace_type_vars_with_generics_composite(
                        engine,
                        &target.owner_ty,
                        mapping,
                        composite_types,
                    );
                    replace_method_target_types(
                        engine,
                        &mut target.method,
                        mapping,
                        composite_types,
                    );
                }
            }
            for ty in [output_ty, residual_ty, return_ty] {
                *ty =
                    replace_type_vars_with_generics_composite(engine, ty, mapping, composite_types);
            }
        }
        _ => {}
    }
}

fn replace_method_target_types(
    engine: &InferenceEngine,
    target: &mut HirMethodCallTarget,
    mapping: &HashMap<TypeVarId, GenericParamId>,
    composite_types: &HashMap<TypeVarId, Type>,
) {
    target.for_each_type_mut(|ty| {
        *ty = replace_type_vars_with_generics_composite(engine, ty, mapping, composite_types);
    });
}

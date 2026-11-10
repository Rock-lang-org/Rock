//! Generic owner remapping over HIR.

use crate::hir::*;
use crate::ids::DefId;
use crate::types::{GenericParamId, Type};

fn remap_type_generic_owner(ty: &mut Type, old_owner: DefId, new_owner: DefId) {
    ty.remap_def_ids(&mut |id| if id == old_owner { new_owner } else { id });
}

fn remap_generic_param_owner(param: &mut GenericParamId, old_owner: DefId, new_owner: DefId) {
    if param.owner == old_owner {
        param.owner = new_owner;
    }
}

fn remap_generic_bounds_owner(bounds: &mut HirGenericBounds, old_owner: DefId, new_owner: DefId) {
    let old_bounds = std::mem::take(bounds);
    for (mut param, mut trait_bounds) in old_bounds {
        remap_generic_param_owner(&mut param, old_owner, new_owner);
        for bound in &mut trait_bounds {
            for type_arg in &mut bound.type_args {
                remap_type_generic_owner(type_arg, old_owner, new_owner);
            }
        }
        bounds.entry(param).or_default().extend(trait_bounds);
    }
}

pub(super) fn remap_function_generic_owner(
    func: &mut HirFunction,
    old_owner: DefId,
    new_owner: DefId,
) {
    if old_owner == new_owner {
        return;
    }

    for param in &mut func.generic_params {
        remap_generic_param_owner(&mut param.id, old_owner, new_owner);
    }
    remap_generic_bounds_owner(&mut func.generic_bounds, old_owner, new_owner);
    for param in &mut func.params {
        remap_type_generic_owner(&mut param.ty, old_owner, new_owner);
    }
    remap_type_generic_owner(&mut func.ret_type, old_owner, new_owner);
    remap_block_generic_owner(&mut func.body, old_owner, new_owner);
}

fn remap_block_generic_owner(block: &mut HirBlock, old_owner: DefId, new_owner: DefId) {
    remap_type_generic_owner(&mut block.ty, old_owner, new_owner);
    for stmt in &mut block.stmts {
        remap_stmt_generic_owner(stmt, old_owner, new_owner);
    }
}

fn remap_stmt_generic_owner(stmt: &mut HirStmt, old_owner: DefId, new_owner: DefId) {
    match stmt {
        HirStmt::Let { ty, value, .. } => {
            remap_type_generic_owner(ty, old_owner, new_owner);
            remap_expr_generic_owner(value, old_owner, new_owner);
        }
        HirStmt::Expr(expr) => remap_expr_generic_owner(expr, old_owner, new_owner),
        HirStmt::Return(value) | HirStmt::Break(value) => {
            if let Some(value) = value {
                remap_expr_generic_owner(value, old_owner, new_owner);
            }
        }
        HirStmt::Continue => {}
    }
}

fn remap_pattern_generic_owner(pattern: &mut HirPattern, old_owner: DefId, new_owner: DefId) {
    match pattern {
        HirPattern::Struct(_, _, type_args, fields) => {
            for type_arg in type_args {
                remap_type_generic_owner(type_arg, old_owner, new_owner);
            }
            for field in fields {
                remap_pattern_generic_owner(&mut field.pattern, old_owner, new_owner);
            }
        }
        HirPattern::Tuple(items) | HirPattern::Or(items) => {
            for nested in items {
                remap_pattern_generic_owner(nested, old_owner, new_owner);
            }
        }
        HirPattern::Enum(_, _, _, items) => {
            for nested in items {
                remap_pattern_generic_owner(nested, old_owner, new_owner);
            }
        }
        HirPattern::Wildcard | HirPattern::Binding { .. } | HirPattern::Literal(_) => {}
    }
}

fn remap_expr_generic_owner(expr: &mut HirExpr, old_owner: DefId, new_owner: DefId) {
    remap_type_generic_owner(&mut expr.ty, old_owner, new_owner);

    match &mut expr.kind {
        HirExprKind::ArrayLiteral(elems) | HirExprKind::TupleLiteral(elems) => {
            for elem in elems {
                remap_expr_generic_owner(elem, old_owner, new_owner);
            }
        }
        HirExprKind::ArrayRepeat(value, _) => remap_expr_generic_owner(value, old_owner, new_owner),
        HirExprKind::FieldAccess(inner, _, _)
        | HirExprKind::TupleIndex(inner, _)
        | HirExprKind::UnaryOp(_, inner)
        | HirExprKind::Ref(_, inner)
        | HirExprKind::Deref(inner) => remap_expr_generic_owner(inner, old_owner, new_owner),
        HirExprKind::Cast(inner, ty) => {
            remap_expr_generic_owner(inner, old_owner, new_owner);
            remap_type_generic_owner(ty, old_owner, new_owner);
        }
        HirExprKind::BinOp(_, lhs, rhs) | HirExprKind::Assign(lhs, rhs) => {
            remap_expr_generic_owner(lhs, old_owner, new_owner);
            remap_expr_generic_owner(rhs, old_owner, new_owner);
        }
        HirExprKind::Call(func, args, _) => {
            remap_expr_generic_owner(func, old_owner, new_owner);
            for arg in args {
                remap_expr_generic_owner(arg, old_owner, new_owner);
            }
        }
        HirExprKind::MethodCall(func, _, args, _, target) => {
            remap_expr_generic_owner(func, old_owner, new_owner);
            for arg in args {
                remap_expr_generic_owner(arg, old_owner, new_owner);
            }
            if let Some(target) = target {
                target.for_each_type_mut(|ty| remap_type_generic_owner(ty, old_owner, new_owner));
            }
        }
        HirExprKind::Try {
            expr,
            branch_method,
            from_residual_target,
            output_ty,
            residual_ty,
            return_ty,
            ..
        } => {
            remap_expr_generic_owner(expr, old_owner, new_owner);
            if let Some(target) = branch_method {
                target.for_each_type_mut(|ty| remap_type_generic_owner(ty, old_owner, new_owner));
            }
            if let Some(HirCallTarget::StaticMethod(target)) = from_residual_target {
                remap_type_generic_owner(&mut target.owner_ty, old_owner, new_owner);
                target
                    .method
                    .for_each_type_mut(|ty| remap_type_generic_owner(ty, old_owner, new_owner));
            }
            remap_type_generic_owner(output_ty, old_owner, new_owner);
            remap_type_generic_owner(residual_ty, old_owner, new_owner);
            remap_type_generic_owner(return_ty, old_owner, new_owner);
        }
        HirExprKind::StructLiteral(_, _, fields) => {
            for field in fields {
                remap_expr_generic_owner(&mut field.value, old_owner, new_owner);
            }
        }
        HirExprKind::EnumVariant(_, _, args, _) | HirExprKind::Intrinsic { args, .. } => {
            for arg in args {
                remap_expr_generic_owner(arg, old_owner, new_owner);
            }
        }
        HirExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            remap_expr_generic_owner(condition, old_owner, new_owner);
            remap_block_generic_owner(then_branch, old_owner, new_owner);
            if let Some(else_branch) = else_branch {
                remap_block_generic_owner(else_branch, old_owner, new_owner);
            }
        }
        HirExprKind::Match { scrutinee, arms } => {
            remap_expr_generic_owner(scrutinee, old_owner, new_owner);
            for arm in arms {
                remap_pattern_generic_owner(&mut arm.pattern, old_owner, new_owner);
                if let Some(guard) = &mut arm.guard {
                    remap_expr_generic_owner(guard, old_owner, new_owner);
                }
                remap_block_generic_owner(&mut arm.body, old_owner, new_owner);
            }
        }
        HirExprKind::While { condition, body } => {
            remap_expr_generic_owner(condition, old_owner, new_owner);
            remap_block_generic_owner(body, old_owner, new_owner);
        }
        HirExprKind::For { iter, body, .. } => {
            remap_expr_generic_owner(iter, old_owner, new_owner);
            remap_block_generic_owner(body, old_owner, new_owner);
        }
        HirExprKind::Loop(body) | HirExprKind::Block(body) | HirExprKind::UnsafeBlock(body) => {
            remap_block_generic_owner(body, old_owner, new_owner);
        }
        HirExprKind::Lambda {
            params,
            body,
            captures,
        } => {
            for param in params {
                remap_type_generic_owner(&mut param.ty, old_owner, new_owner);
            }
            for capture in captures {
                remap_type_generic_owner(&mut capture.ty, old_owner, new_owner);
            }
            remap_block_generic_owner(body, old_owner, new_owner);
        }
        HirExprKind::IntLiteral(_) | HirExprKind::ConstParam(_)
        | HirExprKind::FloatLiteral(_)
        | HirExprKind::BoolLiteral(_)
        | HirExprKind::StringLiteral(_)
        | HirExprKind::CharLiteral(_)
        | HirExprKind::Unit
        | HirExprKind::Var(_)
        | HirExprKind::ResolvedVar(_) => {}
    }
}

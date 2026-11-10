//! Self-type application and type resolution for generated default methods.

use crate::hir::*;
use crate::infer::InferenceEngine;
use crate::types::Type;

pub(super) fn apply_generated_default_self_type(func: &mut HirFunction, self_type: &Type) {
    if func.self_receiver.is_none() {
        return;
    }
    let Some(existing_param_ty) = func.params.first().map(|param| param.ty.clone()) else {
        return;
    };

    let param_ty = match func.self_receiver {
        Some(crate::types::ReceiverMode::Move) => self_type.clone(),
        Some(crate::types::ReceiverMode::Shared) => Type::Reference {
            mutable: false,
            inner: Box::new(self_type.clone()),
        },
        Some(crate::types::ReceiverMode::Mut) => Type::Reference {
            mutable: true,
            inner: Box::new(self_type.clone()),
        },
        None => match existing_param_ty {
            Type::Reference { mutable, .. } => Type::Reference {
                mutable,
                inner: Box::new(self_type.clone()),
            },
            _ => self_type.clone(),
        },
    };

    if let Some(param) = func.params.first_mut() {
        if param.name == "self" {
            param.ty = param_ty.clone();
        }
    }
    apply_generated_default_self_type_in_block(&mut func.body, &param_ty, self_type);
}

fn apply_generated_default_self_type_in_block(
    block: &mut HirBlock,
    param_ty: &Type,
    self_type: &Type,
) {
    for stmt in &mut block.stmts {
        match stmt {
            HirStmt::Let { value, .. } | HirStmt::Expr(value) => {
                apply_generated_default_self_type_in_expr(value, param_ty, self_type);
            }
            HirStmt::Return(Some(value)) | HirStmt::Break(Some(value)) => {
                apply_generated_default_self_type_in_expr(value, param_ty, self_type);
            }
            HirStmt::Return(None) | HirStmt::Break(None) | HirStmt::Continue => {}
        }
    }
}

fn apply_generated_default_self_type_in_expr(
    expr: &mut HirExpr,
    param_ty: &Type,
    self_type: &Type,
) {
    match &mut expr.kind {
        HirExprKind::Var(name) if name == "self" => {
            expr.ty = param_ty.clone();
        }
        HirExprKind::Call(func, args, _) => {
            apply_generated_default_self_type_in_expr(func, param_ty, self_type);
            for arg in args {
                apply_generated_default_self_type_in_expr(arg, param_ty, self_type);
            }
        }
        HirExprKind::MethodCall(recv, _, args, _, _) => {
            apply_generated_default_self_type_in_expr(recv, param_ty, self_type);
            for arg in args {
                apply_generated_default_self_type_in_expr(arg, param_ty, self_type);
            }
        }
        HirExprKind::Try { expr, .. } => {
            apply_generated_default_self_type_in_expr(expr, param_ty, self_type);
        }
        HirExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            apply_generated_default_self_type_in_expr(condition, param_ty, self_type);
            apply_generated_default_self_type_in_block(then_branch, param_ty, self_type);
            if let Some(else_branch) = else_branch {
                apply_generated_default_self_type_in_block(else_branch, param_ty, self_type);
            }
        }
        HirExprKind::Block(block) | HirExprKind::Loop(block) | HirExprKind::UnsafeBlock(block) => {
            apply_generated_default_self_type_in_block(block, param_ty, self_type);
        }
        HirExprKind::Lambda { body, .. } => {
            apply_generated_default_self_type_in_block(body, param_ty, self_type);
        }
        HirExprKind::While { condition, body } => {
            apply_generated_default_self_type_in_expr(condition, param_ty, self_type);
            apply_generated_default_self_type_in_block(body, param_ty, self_type);
        }
        HirExprKind::For { iter, body, .. } => {
            apply_generated_default_self_type_in_expr(iter, param_ty, self_type);
            apply_generated_default_self_type_in_block(body, param_ty, self_type);
        }
        HirExprKind::Assign(lhs, rhs) | HirExprKind::BinOp(_, lhs, rhs) => {
            apply_generated_default_self_type_in_expr(lhs, param_ty, self_type);
            apply_generated_default_self_type_in_expr(rhs, param_ty, self_type);
        }
        HirExprKind::Deref(inner) => {
            apply_generated_default_self_type_in_expr(inner, param_ty, self_type);
            if inner.ty == *param_ty {
                expr.ty = self_type.clone();
            }
        }
        HirExprKind::UnaryOp(_, inner)
        | HirExprKind::FieldAccess(inner, _, _)
        | HirExprKind::TupleIndex(inner, _)
        | HirExprKind::Ref(_, inner)
        | HirExprKind::Cast(inner, _) => {
            apply_generated_default_self_type_in_expr(inner, param_ty, self_type);
        }
        HirExprKind::Intrinsic { args, .. }
        | HirExprKind::ArrayLiteral(args)
        | HirExprKind::TupleLiteral(args)
        | HirExprKind::EnumVariant(_, _, args, _) => {
            for arg in args {
                apply_generated_default_self_type_in_expr(arg, param_ty, self_type);
            }
        }
        HirExprKind::ArrayRepeat(value, _) => {
            apply_generated_default_self_type_in_expr(value, param_ty, self_type);
        }
        HirExprKind::StructLiteral(_, _, fields) => {
            for field in fields {
                apply_generated_default_self_type_in_expr(&mut field.value, param_ty, self_type);
            }
        }
        HirExprKind::Match { scrutinee, arms } => {
            apply_generated_default_self_type_in_expr(scrutinee, param_ty, self_type);
            for arm in arms {
                if let Some(guard) = &mut arm.guard {
                    apply_generated_default_self_type_in_expr(guard, param_ty, self_type);
                }
                apply_generated_default_self_type_in_block(&mut arm.body, param_ty, self_type);
            }
        }
        HirExprKind::Var(_)
        | HirExprKind::ResolvedVar(_)
        | HirExprKind::IntLiteral(_) | HirExprKind::ConstParam(_)
        | HirExprKind::FloatLiteral(_)
        | HirExprKind::BoolLiteral(_)
        | HirExprKind::StringLiteral(_)
        | HirExprKind::CharLiteral(_)
        | HirExprKind::Unit => {}
    }
}

pub(super) fn resolve_generated_default_types(
    engine: &mut InferenceEngine,
    func: &mut HirFunction,
) {
    for param in &mut func.params {
        param.ty = engine.resolve(&param.ty);
    }
    func.ret_type = engine.resolve(&func.ret_type);
    resolve_generated_default_block_types(engine, &mut func.body);
    if !generated_default_uses_generic_type(func) {
        func.generic_params.clear();
        func.generic_bounds.clear();
    }
}

fn generated_default_uses_generic_type(func: &HirFunction) -> bool {
    func.params.iter().any(|param| type_uses_generic(&param.ty))
        || type_uses_generic(&func.ret_type)
        || type_uses_generic(&func.body.ty)
        || bounds_use_generic_type(func)
}

fn bounds_use_generic_type(func: &HirFunction) -> bool {
    func.generic_bounds.iter().any(|(param, bounds)| {
        param.owner == func.id
            || bounds
                .iter()
                .any(|bound| bound.type_args.iter().any(type_uses_generic))
    })
}

fn type_uses_generic(ty: &Type) -> bool {
    crate::type_services::visit::type_any(ty, |nested| {
        matches!(nested, Type::Generic(_) | Type::Projection { .. })
    })
}

fn resolve_generated_default_block_types(engine: &mut InferenceEngine, block: &mut HirBlock) {
    block.ty = engine.resolve(&block.ty);
    for stmt in &mut block.stmts {
        resolve_generated_default_stmt_types(engine, stmt);
    }
    if let Some(HirStmt::Expr(expr) | HirStmt::Return(Some(expr)) | HirStmt::Break(Some(expr))) =
        block.stmts.last()
    {
        block.ty = expr.ty.clone();
    }
}

fn resolve_generated_default_stmt_types(engine: &mut InferenceEngine, stmt: &mut HirStmt) {
    match stmt {
        HirStmt::Let { ty, value, .. } => {
            *ty = engine.resolve(ty);
            resolve_generated_default_expr_types(engine, value);
        }
        HirStmt::Expr(expr) | HirStmt::Return(Some(expr)) | HirStmt::Break(Some(expr)) => {
            resolve_generated_default_expr_types(engine, expr);
        }
        HirStmt::Return(None) | HirStmt::Break(None) | HirStmt::Continue => {}
    }
}

fn resolve_generated_default_pattern_types(engine: &mut InferenceEngine, pattern: &mut HirPattern) {
    match pattern {
        HirPattern::Tuple(patterns) | HirPattern::Or(patterns) => {
            for pattern in patterns {
                resolve_generated_default_pattern_types(engine, pattern);
            }
        }
        HirPattern::Struct(_, _, type_args, field_patterns) => {
            for ty in type_args {
                *ty = engine.resolve(ty);
            }
            for field in field_patterns {
                resolve_generated_default_pattern_types(engine, &mut field.pattern);
            }
        }
        HirPattern::Enum(_, _, _, patterns) => {
            for pattern in patterns {
                resolve_generated_default_pattern_types(engine, pattern);
            }
        }
        HirPattern::Wildcard | HirPattern::Binding { .. } | HirPattern::Literal(_) => {}
    }
}

fn resolve_generated_default_expr_types(engine: &mut InferenceEngine, expr: &mut HirExpr) {
    expr.ty = engine.resolve(&expr.ty);
    match &mut expr.kind {
        HirExprKind::Call(func_expr, args, target) => {
            resolve_generated_default_expr_types(engine, func_expr);
            for arg in args {
                resolve_generated_default_expr_types(engine, arg);
            }
            if let Some(HirCallTarget::StaticMethod(target)) = target {
                target.owner_ty = engine.resolve(&target.owner_ty);
                target
                    .method
                    .for_each_type_mut(|ty| *ty = engine.resolve(ty));
            }
        }
        HirExprKind::MethodCall(recv, _, args, _, target) => {
            resolve_generated_default_expr_types(engine, recv);
            for arg in args {
                resolve_generated_default_expr_types(engine, arg);
            }
            if let Some(target) = target {
                target.for_each_type_mut(|ty| *ty = engine.resolve(ty));
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
            resolve_generated_default_expr_types(engine, expr);
            if let Some(target) = branch_method {
                target.for_each_type_mut(|ty| *ty = engine.resolve(ty));
            }
            if let Some(HirCallTarget::StaticMethod(target)) = from_residual_target {
                target.owner_ty = engine.resolve(&target.owner_ty);
                target
                    .method
                    .for_each_type_mut(|ty| *ty = engine.resolve(ty));
            }
            *output_ty = engine.resolve(output_ty);
            *residual_ty = engine.resolve(residual_ty);
            *return_ty = engine.resolve(return_ty);
        }
        HirExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            resolve_generated_default_expr_types(engine, condition);
            resolve_generated_default_block_types(engine, then_branch);
            if let Some(else_branch) = else_branch {
                resolve_generated_default_block_types(engine, else_branch);
            }
        }
        HirExprKind::Block(block) | HirExprKind::Loop(block) | HirExprKind::UnsafeBlock(block) => {
            resolve_generated_default_block_types(engine, block);
        }
        HirExprKind::Lambda {
            params,
            body,
            captures,
        } => {
            for param in params {
                param.ty = engine.resolve(&param.ty);
            }
            for capture in captures {
                capture.ty = engine.resolve(&capture.ty);
            }
            resolve_generated_default_block_types(engine, body);
        }
        HirExprKind::While { condition, body } => {
            resolve_generated_default_expr_types(engine, condition);
            resolve_generated_default_block_types(engine, body);
        }
        HirExprKind::For { iter, body, .. } => {
            resolve_generated_default_expr_types(engine, iter);
            resolve_generated_default_block_types(engine, body);
        }
        HirExprKind::Assign(lhs, rhs) | HirExprKind::BinOp(_, lhs, rhs) => {
            resolve_generated_default_expr_types(engine, lhs);
            resolve_generated_default_expr_types(engine, rhs);
        }
        HirExprKind::UnaryOp(_, inner)
        | HirExprKind::FieldAccess(inner, _, _)
        | HirExprKind::TupleIndex(inner, _)
        | HirExprKind::Ref(_, inner)
        | HirExprKind::Deref(inner) => {
            resolve_generated_default_expr_types(engine, inner);
        }
        HirExprKind::Cast(inner, ty) => {
            resolve_generated_default_expr_types(engine, inner);
            *ty = engine.resolve(ty);
        }
        HirExprKind::Intrinsic { args, .. }
        | HirExprKind::ArrayLiteral(args)
        | HirExprKind::TupleLiteral(args)
        | HirExprKind::EnumVariant(_, _, args, _) => {
            for arg in args {
                resolve_generated_default_expr_types(engine, arg);
            }
        }
        HirExprKind::ArrayRepeat(value, _) => {
            resolve_generated_default_expr_types(engine, value);
        }
        HirExprKind::StructLiteral(_, _, fields) => {
            for field in fields {
                resolve_generated_default_expr_types(engine, &mut field.value);
            }
        }
        HirExprKind::Match { scrutinee, arms } => {
            resolve_generated_default_expr_types(engine, scrutinee);
            for arm in arms {
                resolve_generated_default_pattern_types(engine, &mut arm.pattern);
                if let Some(guard) = &mut arm.guard {
                    resolve_generated_default_expr_types(engine, guard);
                }
                resolve_generated_default_block_types(engine, &mut arm.body);
            }
        }
        HirExprKind::Var(_)
        | HirExprKind::ResolvedVar(_)
        | HirExprKind::IntLiteral(_) | HirExprKind::ConstParam(_)
        | HirExprKind::FloatLiteral(_)
        | HirExprKind::BoolLiteral(_)
        | HirExprKind::StringLiteral(_)
        | HirExprKind::CharLiteral(_)
        | HirExprKind::Unit => {}
    }
}

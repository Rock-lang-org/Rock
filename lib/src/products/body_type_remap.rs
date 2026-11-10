use super::interface_types::ProductImplInterface;
use super::type_id_remap::remap_function_interface_signature_types;
use crate::hir::{
    HirCallTarget, HirFunctionFor, HirImplFor, HirMethodCallTarget, HirPattern, HirPhase,
};

pub(super) fn remap_function_signature_types<P: HirPhase, F>(
    function: &mut HirFunctionFor<P>,
    remap_type: &mut F,
) where
    F: FnMut(&mut crate::types::Type),
{
    for param in &mut function.params {
        remap_type(&mut param.ty);
    }
    remap_type(&mut function.ret_type);
    remap_type(&mut function.body.ty);
}

pub(super) fn remap_function_body_types<P: HirPhase, F>(
    function: &mut HirFunctionFor<P>,
    remap_type: &mut F,
) where
    F: FnMut(&mut crate::types::Type),
{
    remap_block_types(&mut function.body, remap_type);
}

fn remap_block_types<P: HirPhase, F>(block: &mut crate::hir::HirBlockFor<P>, remap_type: &mut F)
where
    F: FnMut(&mut crate::types::Type),
{
    for stmt in &mut block.stmts {
        remap_stmt_types(stmt, remap_type);
    }
    remap_type(&mut block.ty);
}

fn remap_stmt_types<P: HirPhase, F>(stmt: &mut crate::hir::HirStmtFor<P>, remap_type: &mut F)
where
    F: FnMut(&mut crate::types::Type),
{
    match stmt {
        crate::hir::HirStmtFor::Let { ty, value, .. } => {
            remap_type(ty);
            remap_expr_types(value, remap_type);
        }
        crate::hir::HirStmtFor::Expr(expr) => remap_expr_types(expr, remap_type),
        crate::hir::HirStmtFor::Return(expr) | crate::hir::HirStmtFor::Break(expr) => {
            if let Some(expr) = expr {
                remap_expr_types(expr, remap_type);
            }
        }
        crate::hir::HirStmtFor::Continue => {}
    }
}

fn remap_expr_types<P: HirPhase, F>(expr: &mut crate::hir::HirExprFor<P>, remap_type: &mut F)
where
    F: FnMut(&mut crate::types::Type),
{
    remap_type(&mut expr.ty);
    match &mut expr.kind {
        crate::hir::HirExprKindFor::ArrayLiteral(elems)
        | crate::hir::HirExprKindFor::TupleLiteral(elems) => {
            for elem in elems {
                remap_expr_types(elem, remap_type);
            }
        }
        crate::hir::HirExprKindFor::ArrayRepeat(value, _) => remap_expr_types(value, remap_type),
        crate::hir::HirExprKindFor::FieldAccess(inner, _, _)
        | crate::hir::HirExprKindFor::TupleIndex(inner, _)
        | crate::hir::HirExprKindFor::UnaryOp(_, inner)
        | crate::hir::HirExprKindFor::Ref(_, inner)
        | crate::hir::HirExprKindFor::Deref(inner) => remap_expr_types(inner, remap_type),
        crate::hir::HirExprKindFor::BinOp(_, lhs, rhs)
        | crate::hir::HirExprKindFor::Assign(lhs, rhs) => {
            remap_expr_types(lhs, remap_type);
            remap_expr_types(rhs, remap_type);
        }
        crate::hir::HirExprKindFor::Call(func, args, target) => {
            if let Some(target) = target {
                remap_call_target_types(target, remap_type);
            }
            remap_expr_types(func, remap_type);
            for arg in args {
                remap_expr_types(arg, remap_type);
            }
        }
        crate::hir::HirExprKindFor::MethodCall(func, _, args, _, target) => {
            if let Some(target) = P::method_authority_mut(target) {
                remap_method_target_types(target, remap_type);
            }
            remap_expr_types(func, remap_type);
            for arg in args {
                remap_expr_types(arg, remap_type);
            }
        }
        crate::hir::HirExprKindFor::Try {
            expr,
            branch_method,
            from_residual_target,
            output_ty,
            residual_ty,
            return_ty,
            ..
        } => {
            if let Some(target) = P::method_authority_mut(branch_method) {
                remap_method_target_types(target, remap_type);
            }
            if let Some(target) = P::residual_authority_mut(from_residual_target) {
                remap_call_target_types(target, remap_type);
            }
            remap_expr_types(expr, remap_type);
            remap_type(output_ty);
            remap_type(residual_ty);
            remap_type(return_ty);
        }
        crate::hir::HirExprKindFor::StructLiteral(_, _, fields) => {
            for field in fields {
                remap_expr_types(&mut field.value, remap_type);
            }
        }
        crate::hir::HirExprKindFor::EnumVariant(_, _, args, _)
        | crate::hir::HirExprKindFor::Intrinsic { args, .. } => {
            for arg in args {
                remap_expr_types(arg, remap_type);
            }
        }
        crate::hir::HirExprKindFor::If {
            condition,
            then_branch,
            else_branch,
        } => {
            remap_expr_types(condition, remap_type);
            remap_block_types(then_branch, remap_type);
            if let Some(else_branch) = else_branch {
                remap_block_types(else_branch, remap_type);
            }
        }
        crate::hir::HirExprKindFor::Match { scrutinee, arms } => {
            remap_expr_types(scrutinee, remap_type);
            for arm in arms {
                remap_pattern_types(&mut arm.pattern, remap_type);
                if let Some(guard) = &mut arm.guard {
                    remap_expr_types(guard, remap_type);
                }
                remap_block_types(&mut arm.body, remap_type);
            }
        }
        crate::hir::HirExprKindFor::While { condition, body } => {
            remap_expr_types(condition, remap_type);
            remap_block_types(body, remap_type);
        }
        crate::hir::HirExprKindFor::For { iter, body, .. } => {
            remap_expr_types(iter, remap_type);
            remap_block_types(body, remap_type);
        }
        crate::hir::HirExprKindFor::Loop(body)
        | crate::hir::HirExprKindFor::Block(body)
        | crate::hir::HirExprKindFor::UnsafeBlock(body) => {
            remap_block_types(body, remap_type);
        }
        crate::hir::HirExprKindFor::Lambda {
            params,
            body,
            captures,
        } => {
            for param in params {
                remap_type(&mut param.ty);
            }
            for capture in captures {
                remap_type(&mut capture.ty);
            }
            remap_block_types(body, remap_type);
        }
        crate::hir::HirExprKindFor::Cast(inner, ty) => {
            remap_expr_types(inner, remap_type);
            remap_type(ty);
        }
        crate::hir::HirExprKindFor::IntLiteral(_) | crate::hir::HirExprKindFor::ConstParam(_)
        | crate::hir::HirExprKindFor::FloatLiteral(_)
        | crate::hir::HirExprKindFor::BoolLiteral(_)
        | crate::hir::HirExprKindFor::StringLiteral(_)
        | crate::hir::HirExprKindFor::CharLiteral(_)
        | crate::hir::HirExprKindFor::Unit
        | crate::hir::HirExprKindFor::Var(_)
        | crate::hir::HirExprKindFor::ResolvedVar(_) => {}
    }
}

fn remap_pattern_types<F>(pattern: &mut HirPattern, remap_type: &mut F)
where
    F: FnMut(&mut crate::types::Type),
{
    match pattern {
        HirPattern::Tuple(patterns) | HirPattern::Or(patterns) => {
            for pattern in patterns {
                remap_pattern_types(pattern, remap_type);
            }
        }
        HirPattern::Struct(_, _, args, fields) => {
            for arg in args {
                remap_type(arg);
            }
            for field in fields {
                remap_pattern_types(&mut field.pattern, remap_type);
            }
        }
        HirPattern::Enum(_, _, _, patterns) => {
            for pattern in patterns {
                remap_pattern_types(pattern, remap_type);
            }
        }
        HirPattern::Wildcard | HirPattern::Binding { .. } | HirPattern::Literal(_) => {}
    }
}

fn remap_call_target_types<F>(target: &mut HirCallTarget, remap_type: &mut F)
where
    F: FnMut(&mut crate::types::Type),
{
    if let HirCallTarget::StaticMethod(target) = target {
        remap_type(&mut target.owner_ty);
        remap_method_target_types(&mut target.method, remap_type);
    }
}

fn remap_method_target_types<F>(target: &mut HirMethodCallTarget, remap_type: &mut F)
where
    F: FnMut(&mut crate::types::Type),
{
    target.for_each_type_mut(remap_type);
}

pub(super) fn remap_impl_types<P: HirPhase, F>(imp: &mut HirImplFor<P>, remap_type: &mut F)
where
    F: FnMut(&mut crate::types::Type),
{
    P::visit_impl_receiver_types_mut(&mut imp.receiver_pattern, remap_type);
    for ty in &mut imp.trait_arg_types {
        remap_type(ty);
    }
    for assoc in &mut imp.associated_types {
        remap_type(&mut assoc.ty);
    }
    for bounds in imp.bounds.values_mut() {
        for bound in bounds {
            for ty in &mut bound.type_args {
                remap_type(ty);
            }
        }
    }
    for method in imp.methods.values_mut() {
        remap_function_signature_types(method, remap_type);
        remap_function_body_types(method, remap_type);
    }
}

pub(super) fn remap_impl_interface_types<F>(imp: &mut ProductImplInterface, remap_type: &mut F)
where
    F: FnMut(&mut crate::types::Type),
{
    match &mut imp.receiver_pattern {
        crate::hir::HirImplReceiverPattern::Exact(ty)
        | crate::hir::HirImplReceiverPattern::Constructor(ty) => remap_type(ty),
        crate::hir::HirImplReceiverPattern::SliceFamily { element } => remap_type(element),
    }
    for ty in &mut imp.trait_arg_types {
        remap_type(ty);
    }
    for assoc in &mut imp.associated_types {
        remap_type(&mut assoc.ty);
    }
    for bounds in imp.bounds.values_mut() {
        for bound in bounds {
            for ty in &mut bound.type_args {
                remap_type(ty);
            }
        }
    }
    for method in imp.methods.values_mut() {
        remap_function_interface_signature_types(method, remap_type);
    }
}

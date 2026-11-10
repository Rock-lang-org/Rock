use super::identity_recording::{product_def_id_to_def_id, unambiguous_product_id};
use super::{ProductDefId, ProductIdRemap};
use crate::hir::{
    HirCallTarget, HirFunctionFor, HirImplFor, HirMethodCallTarget, HirPattern, HirPhase,
    HirSelectedMethodTarget, HirVarTarget,
};
use crate::ids::DefId;

pub(super) fn remap_impl_body_location_product_ids<P: HirPhase>(
    imp: &mut HirImplFor<P>,
    id_remap: &ProductIdRemap,
) {
    for method in imp.methods.values_mut() {
        remap_function_body_location_product_ids(method, id_remap);
    }
}

pub(super) fn remap_function_body_location_product_ids<P: HirPhase>(
    function: &mut HirFunctionFor<P>,
    id_remap: &ProductIdRemap,
) {
    remap_block_location_product_ids(&mut function.body, id_remap);
}

fn remap_block_location_product_ids<P: HirPhase>(
    block: &mut crate::hir::HirBlockFor<P>,
    id_remap: &ProductIdRemap,
) {
    for stmt in &mut block.stmts {
        remap_stmt_location_product_ids(stmt, id_remap);
    }
}

fn remap_stmt_location_product_ids<P: HirPhase>(
    stmt: &mut crate::hir::HirStmtFor<P>,
    id_remap: &ProductIdRemap,
) {
    match stmt {
        crate::hir::HirStmtFor::Let { value, .. } | crate::hir::HirStmtFor::Expr(value) => {
            remap_expr_location_product_ids(value, id_remap);
        }
        crate::hir::HirStmtFor::Return(value) | crate::hir::HirStmtFor::Break(value) => {
            if let Some(value) = value {
                remap_expr_location_product_ids(value, id_remap);
            }
        }
        crate::hir::HirStmtFor::Continue => {}
    }
}

pub(super) fn remap_expr_location_product_ids<P: HirPhase>(
    expr: &mut crate::hir::HirExprFor<P>,
    id_remap: &ProductIdRemap,
) {
    match &mut expr.kind {
        crate::hir::HirExprKindFor::FieldAccess(inner, _, location) => {
            remap_expr_location_product_ids(inner, id_remap);
            if let Some(location) = location {
                remap_product_def_id_owner(&mut location.owner, id_remap);
            }
        }
        crate::hir::HirExprKindFor::StructLiteral(_, struct_id, fields) => {
            if let Some(struct_id) = struct_id {
                remap_product_def_id_owner(struct_id, id_remap);
            }
            for field in fields {
                if let Some(location) = &mut field.field {
                    remap_product_def_id_owner(&mut location.owner, id_remap);
                }
                remap_expr_location_product_ids(&mut field.value, id_remap);
            }
        }
        crate::hir::HirExprKindFor::EnumVariant(_, _, args, location) => {
            if let Some(location) = location {
                remap_product_def_id_owner(&mut location.owner, id_remap);
            }
            for arg in args {
                remap_expr_location_product_ids(arg, id_remap);
            }
        }
        crate::hir::HirExprKindFor::Intrinsic { args, .. } => {
            for arg in args {
                remap_expr_location_product_ids(arg, id_remap);
            }
        }
        crate::hir::HirExprKindFor::ArrayLiteral(elems)
        | crate::hir::HirExprKindFor::TupleLiteral(elems) => {
            for elem in elems {
                remap_expr_location_product_ids(elem, id_remap);
            }
        }
        crate::hir::HirExprKindFor::ArrayRepeat(value, _) => {
            remap_expr_location_product_ids(value, id_remap);
        }
        crate::hir::HirExprKindFor::TupleIndex(inner, _)
        | crate::hir::HirExprKindFor::UnaryOp(_, inner)
        | crate::hir::HirExprKindFor::Ref(_, inner)
        | crate::hir::HirExprKindFor::Deref(inner)
        | crate::hir::HirExprKindFor::Cast(inner, _) => {
            remap_expr_location_product_ids(inner, id_remap)
        }
        crate::hir::HirExprKindFor::BinOp(_, lhs, rhs)
        | crate::hir::HirExprKindFor::Assign(lhs, rhs) => {
            remap_expr_location_product_ids(lhs, id_remap);
            remap_expr_location_product_ids(rhs, id_remap);
        }
        crate::hir::HirExprKindFor::Call(func, args, target) => {
            if let Some(target) = target {
                remap_call_target_product_ids(target, id_remap);
            }
            remap_expr_location_product_ids(func, id_remap);
            for arg in args {
                remap_expr_location_product_ids(arg, id_remap);
            }
        }
        crate::hir::HirExprKindFor::MethodCall(func, _, args, _, target) => {
            if let Some(target) = P::method_authority_mut(target) {
                remap_method_target_product_ids(target, id_remap);
            }
            remap_expr_location_product_ids(func, id_remap);
            for arg in args {
                remap_expr_location_product_ids(arg, id_remap);
            }
        }
        crate::hir::HirExprKindFor::Try {
            expr,
            branch_method,
            from_residual_target,
            control_flow_enum,
            break_variant,
            continue_variant,
            ..
        } => {
            if let Some(target) = P::method_authority_mut(branch_method) {
                remap_method_target_product_ids(target, id_remap);
            }
            if let Some(target) = P::residual_authority_mut(from_residual_target) {
                remap_call_target_product_ids(target, id_remap);
            }
            remap_product_def_id_owner(control_flow_enum, id_remap);
            remap_product_def_id_owner(&mut break_variant.owner, id_remap);
            remap_product_def_id_owner(&mut continue_variant.owner, id_remap);
            remap_expr_location_product_ids(expr, id_remap);
        }
        crate::hir::HirExprKindFor::If {
            condition,
            then_branch,
            else_branch,
        } => {
            remap_expr_location_product_ids(condition, id_remap);
            remap_block_location_product_ids(then_branch, id_remap);
            if let Some(else_branch) = else_branch {
                remap_block_location_product_ids(else_branch, id_remap);
            }
        }
        crate::hir::HirExprKindFor::Match { scrutinee, arms } => {
            remap_expr_location_product_ids(scrutinee, id_remap);
            for arm in arms {
                remap_pattern_location_product_ids(&mut arm.pattern, id_remap);
                if let Some(guard) = &mut arm.guard {
                    remap_expr_location_product_ids(guard, id_remap);
                }
                remap_block_location_product_ids(&mut arm.body, id_remap);
            }
        }
        crate::hir::HirExprKindFor::While { condition, body } => {
            remap_expr_location_product_ids(condition, id_remap);
            remap_block_location_product_ids(body, id_remap);
        }
        crate::hir::HirExprKindFor::For { iter, body, .. } => {
            remap_expr_location_product_ids(iter, id_remap);
            remap_block_location_product_ids(body, id_remap);
        }
        crate::hir::HirExprKindFor::Loop(body)
        | crate::hir::HirExprKindFor::Block(body)
        | crate::hir::HirExprKindFor::UnsafeBlock(body) => {
            remap_block_location_product_ids(body, id_remap);
        }
        crate::hir::HirExprKindFor::Lambda { body, .. } => {
            remap_block_location_product_ids(body, id_remap);
        }
        crate::hir::HirExprKindFor::ResolvedVar(reference) => {
            remap_var_target_product_ids(&mut reference.target, id_remap);
        }
        crate::hir::HirExprKindFor::IntLiteral(_) | crate::hir::HirExprKindFor::ConstParam(_)
        | crate::hir::HirExprKindFor::FloatLiteral(_)
        | crate::hir::HirExprKindFor::BoolLiteral(_)
        | crate::hir::HirExprKindFor::StringLiteral(_)
        | crate::hir::HirExprKindFor::CharLiteral(_)
        | crate::hir::HirExprKindFor::Unit
        | crate::hir::HirExprKindFor::Var(_) => {}
    }
}

fn remap_call_target_product_ids(target: &mut HirCallTarget, id_remap: &ProductIdRemap) {
    match target {
        HirCallTarget::Function(id) | HirCallTarget::Extern(id) => {
            remap_product_def_id_owner(id, id_remap);
        }
        HirCallTarget::StaticMethod(target) => {
            remap_method_target_product_ids(&mut target.method, id_remap);
        }
        HirCallTarget::Instance(_) | HirCallTarget::Local(_) | HirCallTarget::Intrinsic(_) => {}
    }
}

fn remap_method_target_product_ids(target: &mut HirMethodCallTarget, id_remap: &ProductIdRemap) {
    match &mut target.target {
        HirSelectedMethodTarget::ImplMethod {
            impl_id,
            method_id,
            selected_trait,
        } => {
            remap_product_def_id_owner(impl_id, id_remap);
            remap_product_def_id_owner(method_id, id_remap);
            if let Some(selected_trait) = selected_trait {
                remap_product_def_id_owner(&mut selected_trait.trait_id, id_remap);
                remap_product_def_id_owner(&mut selected_trait.member_id, id_remap);
            }
        }
        HirSelectedMethodTarget::TraitMethod {
            trait_id,
            member_id,
            ..
        } => {
            remap_product_def_id_owner(trait_id, id_remap);
            remap_product_def_id_owner(member_id, id_remap);
        }
    }
    for binding in target
        .owner_substitution
        .iter_mut()
        .chain(target.method_substitution.iter_mut())
    {
        remap_product_def_id_owner(&mut binding.param.owner, id_remap);
    }
}

fn remap_var_target_product_ids(target: &mut HirVarTarget, id_remap: &ProductIdRemap) {
    match target {
        HirVarTarget::Function(id) | HirVarTarget::Extern(id) => {
            remap_product_def_id_owner(id, id_remap);
        }
        HirVarTarget::Instance(_) | HirVarTarget::Local(_) => {}
    }
}

fn remap_pattern_location_product_ids(pattern: &mut HirPattern, id_remap: &ProductIdRemap) {
    match pattern {
        HirPattern::Struct(_, struct_id, _, fields) => {
            if let Some(struct_id) = struct_id {
                remap_product_def_id_owner(struct_id, id_remap);
            }
            for field in fields {
                if let Some(location) = &mut field.field {
                    remap_product_def_id_owner(&mut location.owner, id_remap);
                }
                remap_pattern_location_product_ids(&mut field.pattern, id_remap);
            }
        }
        HirPattern::Enum(_, _, location, patterns) => {
            if let Some(location) = location {
                remap_product_def_id_owner(&mut location.owner, id_remap);
            }
            for pattern in patterns {
                remap_pattern_location_product_ids(pattern, id_remap);
            }
        }
        HirPattern::Tuple(patterns) | HirPattern::Or(patterns) => {
            for pattern in patterns {
                remap_pattern_location_product_ids(pattern, id_remap);
            }
        }
        HirPattern::Wildcard | HirPattern::Binding { .. } | HirPattern::Literal(_) => {}
    }
}

pub(super) fn remap_product_def_id_owner(owner: &mut DefId, id_remap: &ProductIdRemap) {
    if let Some(product_id) = unambiguous_product_id(ProductDefId::from(*owner), id_remap) {
        *owner = product_def_id_to_def_id(product_id);
    }
}

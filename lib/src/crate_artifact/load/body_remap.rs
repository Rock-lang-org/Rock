use std::collections::BTreeSet;

use super::child_location_validator::{
    ProductChildLocationValidator, MAX_ARTIFACT_HIR_REMAP_DEPTH,
};
use super::generic_remap::{remap_generic_bounds, remap_generic_param_decls, remap_type_def_ids};
use super::nominal_validator::ProductNominalTypeValidator;
use super::target_remap::{
    remap_call_target_child_location, remap_method_target_child_location,
    remap_pattern_child_locations, remap_var_target_child_location,
};
use super::ProductIdentityRemap;
use crate::products::ProductDefId;

pub(super) fn remap_function_child_locations<P: crate::hir::HirPhase>(
    function: &mut crate::hir::HirFunctionFor<P>,
    remap: &ProductIdentityRemap,
    validator: &ProductChildLocationValidator,
    type_validator: &ProductNominalTypeValidator,
) -> Result<(), String> {
    let mut local_scope = BTreeSet::new();
    for param in &function.params {
        local_scope.insert(param.local_id);
    }

    remap_generic_param_decls(&mut function.generic_params, remap, type_validator)?;
    remap_generic_bounds(&mut function.generic_bounds, remap, type_validator)?;
    for param in &mut function.params {
        remap_type_def_ids(&mut param.ty, remap, type_validator)?;
    }
    remap_type_def_ids(&mut function.ret_type, remap, type_validator)?;
    remap_block_child_locations(
        &mut function.body,
        remap,
        validator,
        type_validator,
        &mut local_scope,
        0,
    )
}

fn add_pattern_local_ids(
    pattern: &crate::hir::HirPattern,
    local_scope: &mut BTreeSet<crate::ids::HirLocalId>,
) {
    match pattern {
        crate::hir::HirPattern::Binding { local_id, .. } => {
            local_scope.insert(*local_id);
        }
        crate::hir::HirPattern::Tuple(patterns)
        | crate::hir::HirPattern::Enum(_, _, _, patterns)
        | crate::hir::HirPattern::Or(patterns) => {
            for pattern in patterns {
                add_pattern_local_ids(pattern, local_scope);
            }
        }
        crate::hir::HirPattern::Struct(_, _, _, fields) => {
            for field in fields {
                add_pattern_local_ids(&field.pattern, local_scope);
            }
        }
        crate::hir::HirPattern::Wildcard | crate::hir::HirPattern::Literal(_) => {}
    }
}

fn remap_block_child_locations<P: crate::hir::HirPhase>(
    block: &mut crate::hir::HirBlockFor<P>,
    remap: &ProductIdentityRemap,
    validator: &ProductChildLocationValidator,
    type_validator: &ProductNominalTypeValidator,
    local_scope: &mut BTreeSet<crate::ids::HirLocalId>,
    depth: usize,
) -> Result<(), String> {
    if depth > MAX_ARTIFACT_HIR_REMAP_DEPTH {
        return Err("Product artifact HIR body exceeds maximum remap depth".to_string());
    }

    for stmt in &mut block.stmts {
        remap_stmt_child_locations(stmt, remap, validator, type_validator, local_scope, depth)?;
    }
    remap_type_def_ids(&mut block.ty, remap, type_validator)?;
    Ok(())
}

fn remap_stmt_child_locations<P: crate::hir::HirPhase>(
    stmt: &mut crate::hir::HirStmtFor<P>,
    remap: &ProductIdentityRemap,
    validator: &ProductChildLocationValidator,
    type_validator: &ProductNominalTypeValidator,
    local_scope: &mut BTreeSet<crate::ids::HirLocalId>,
    depth: usize,
) -> Result<(), String> {
    match stmt {
        crate::hir::HirStmtFor::Let {
            local_id,
            ty,
            value,
            ..
        } => {
            remap_type_def_ids(ty, remap, type_validator)?;
            remap_expr_child_locations_with_scope(
                value,
                remap,
                validator,
                type_validator,
                local_scope,
                depth + 1,
            )?;
            local_scope.insert(*local_id);
            Ok(())
        }
        crate::hir::HirStmtFor::Expr(value) => remap_expr_child_locations_with_scope(
            value,
            remap,
            validator,
            type_validator,
            local_scope,
            depth + 1,
        ),
        crate::hir::HirStmtFor::Return(Some(value))
        | crate::hir::HirStmtFor::Break(Some(value)) => remap_expr_child_locations_with_scope(
            value,
            remap,
            validator,
            type_validator,
            local_scope,
            depth + 1,
        ),
        crate::hir::HirStmtFor::Return(None)
        | crate::hir::HirStmtFor::Break(None)
        | crate::hir::HirStmtFor::Continue => Ok(()),
    }
}

#[cfg(test)]
pub(super) fn remap_expr_child_locations<P: crate::hir::HirPhase>(
    expr: &mut crate::hir::HirExprFor<P>,
    remap: &ProductIdentityRemap,
    validator: &ProductChildLocationValidator,
    type_validator: &ProductNominalTypeValidator,
    depth: usize,
) -> Result<(), String> {
    let mut local_scope = BTreeSet::new();
    remap_expr_child_locations_with_scope(
        expr,
        remap,
        validator,
        type_validator,
        &mut local_scope,
        depth,
    )
}

fn remap_expr_child_locations_with_scope<P: crate::hir::HirPhase>(
    expr: &mut crate::hir::HirExprFor<P>,
    remap: &ProductIdentityRemap,
    validator: &ProductChildLocationValidator,
    type_validator: &ProductNominalTypeValidator,
    local_scope: &mut BTreeSet<crate::ids::HirLocalId>,
    depth: usize,
) -> Result<(), String> {
    use crate::hir::HirExprKindFor;

    if depth > MAX_ARTIFACT_HIR_REMAP_DEPTH {
        return Err("Product artifact HIR body exceeds maximum remap depth".to_string());
    }

    remap_type_def_ids(&mut expr.ty, remap, type_validator)?;

    match &mut expr.kind {
        HirExprKindFor::IntLiteral(_)
        | HirExprKindFor::FloatLiteral(_)
        | HirExprKindFor::BoolLiteral(_)
        | HirExprKindFor::StringLiteral(_)
        | HirExprKindFor::CharLiteral(_)
        | HirExprKindFor::Unit
        | HirExprKindFor::Var(_) => Ok(()),
        HirExprKindFor::ResolvedVar(reference) => {
            remap_var_target_child_location(reference, remap, type_validator, local_scope)
        }
        HirExprKindFor::ArrayLiteral(values) | HirExprKindFor::TupleLiteral(values) => {
            for value in values {
                remap_expr_child_locations_with_scope(
                    value,
                    remap,
                    validator,
                    type_validator,
                    local_scope,
                    depth + 1,
                )?;
            }
            Ok(())
        }
        HirExprKindFor::ArrayRepeat(value, _) => remap_expr_child_locations_with_scope(
            value,
            remap,
            validator,
            type_validator,
            local_scope,
            depth + 1,
        ),
        HirExprKindFor::FieldAccess(base, field_name, location) => {
            remap_expr_child_locations_with_scope(
                base,
                remap,
                validator,
                type_validator,
                local_scope,
                depth + 1,
            )?;
            if let Some(location) = location {
                let product_owner = ProductDefId::from(location.owner);
                validator.validate_field(
                    product_owner,
                    location.field_id,
                    &location.name,
                    field_name,
                )?;
                location.owner = remap.def_id(product_owner)?;
            }
            Ok(())
        }
        HirExprKindFor::TupleIndex(base, _)
        | HirExprKindFor::UnaryOp(_, base)
        | HirExprKindFor::Ref(_, base)
        | HirExprKindFor::Deref(base) => remap_expr_child_locations_with_scope(
            base,
            remap,
            validator,
            type_validator,
            local_scope,
            depth + 1,
        ),
        HirExprKindFor::BinOp(_, base, index) | HirExprKindFor::Assign(base, index) => {
            remap_expr_child_locations_with_scope(
                base,
                remap,
                validator,
                type_validator,
                local_scope,
                depth + 1,
            )?;
            remap_expr_child_locations_with_scope(
                index,
                remap,
                validator,
                type_validator,
                local_scope,
                depth + 1,
            )
        }
        HirExprKindFor::Call(callee, args, target) => {
            let authorized_static_callee = matches!(
                (&callee.kind, target.as_ref()),
                (
                    HirExprKindFor::ResolvedVar(crate::hir::HirVarRef {
                        target: crate::hir::HirVarTarget::Function(callee_id),
                        ..
                    }),
                    Some(crate::hir::HirCallTarget::StaticMethod(target))
                ) if target.method.method_id() == Some(*callee_id)
            );
            if let Some(target) = target {
                remap_call_target_child_location(target, remap, type_validator, local_scope)?;
            }
            if authorized_static_callee {
                let Some(crate::hir::HirCallTarget::StaticMethod(target)) = target.as_ref() else {
                    unreachable!("authorized static callee checked above")
                };
                let method_id = target.method.method_id().ok_or_else(|| {
                    "Product artifact static authority has no selected method identity".to_string()
                })?;
                remap_type_def_ids(&mut callee.ty, remap, type_validator)?;
                let HirExprKindFor::ResolvedVar(reference) = &mut callee.kind else {
                    unreachable!("authorized static callee checked above")
                };
                reference.target = crate::hir::HirVarTarget::Function(method_id);
            } else {
                remap_expr_child_locations_with_scope(
                    callee,
                    remap,
                    validator,
                    type_validator,
                    local_scope,
                    depth + 1,
                )?;
            }
            for arg in args {
                remap_expr_child_locations_with_scope(
                    arg,
                    remap,
                    validator,
                    type_validator,
                    local_scope,
                    depth + 1,
                )?;
            }
            Ok(())
        }
        HirExprKindFor::MethodCall(receiver, _, args, _, target) => {
            if let Some(target) = P::method_authority_mut(target) {
                type_validator.validate_method_call_target(target)?;
                remap_method_target_child_location(target, remap, type_validator)?;
            }
            remap_expr_child_locations_with_scope(
                receiver,
                remap,
                validator,
                type_validator,
                local_scope,
                depth + 1,
            )?;
            for arg in args {
                remap_expr_child_locations_with_scope(
                    arg,
                    remap,
                    validator,
                    type_validator,
                    local_scope,
                    depth + 1,
                )?;
            }
            Ok(())
        }
        HirExprKindFor::Try {
            expr,
            branch_method,
            branch_target,
            from_residual_target,
            output_ty,
            residual_ty,
            return_ty,
            control_flow_enum,
            break_variant,
            continue_variant,
            ..
        } => {
            remap_type_def_ids(output_ty, remap, type_validator)?;
            remap_type_def_ids(residual_ty, remap, type_validator)?;
            remap_type_def_ids(return_ty, remap, type_validator)?;

            if let Some(target) = P::method_authority_mut(branch_method) {
                type_validator.validate_method_call_target(target)?;
                remap_method_target_child_location(target, remap, type_validator)?;
            }
            if branch_target.is_some() {
                return Err(
                    "Product artifact contains a local Try branch instance target".to_string(),
                );
            }

            if let Some(target) = P::residual_authority_mut(from_residual_target) {
                remap_call_target_child_location(target, remap, type_validator, local_scope)?;
            }

            let product_control_flow = ProductDefId::from(*control_flow_enum);
            type_validator.validate_enum(product_control_flow)?;
            *control_flow_enum = remap.def_id(product_control_flow)?;
            for location in [break_variant, continue_variant] {
                let product_owner = ProductDefId::from(location.owner);
                validator.validate_variant(
                    product_owner,
                    location.variant_id,
                    &location.name,
                    &location.name,
                )?;
                location.owner = remap.def_id(product_owner)?;
            }

            remap_expr_child_locations_with_scope(
                expr,
                remap,
                validator,
                type_validator,
                local_scope,
                depth + 1,
            )
        }
        HirExprKindFor::StructLiteral(_, struct_id, fields) => {
            let product_struct_id = struct_id.map(ProductDefId::from);
            if let Some(struct_id) = struct_id {
                let product_id = ProductDefId::from(*struct_id);
                type_validator.validate_struct(product_id)?;
                *struct_id = remap.def_id(product_id)?;
            }
            for field in fields {
                if let Some(location) = &mut field.field {
                    let product_owner = ProductDefId::from(location.owner);
                    if let Some(product_struct_id) = product_struct_id {
                        if product_owner != product_struct_id {
                            return Err(format!(
                                "Product artifact struct literal field owner mismatch: literal owner {} but field '{}' owner {}",
                                product_struct_id.local_id.0,
                                field.name,
                                product_owner.local_id.0
                            ));
                        }
                    }
                    validator.validate_field(
                        product_owner,
                        location.field_id,
                        &location.name,
                        &field.name,
                    )?;
                    location.owner = remap.def_id(product_owner)?;
                }
                remap_expr_child_locations_with_scope(
                    &mut field.value,
                    remap,
                    validator,
                    type_validator,
                    local_scope,
                    depth + 1,
                )?;
            }
            Ok(())
        }
        HirExprKindFor::EnumVariant(_, variant_name, args, location) => {
            if let Some(location) = location {
                let product_owner = ProductDefId::from(location.owner);
                validator.validate_variant(
                    product_owner,
                    location.variant_id,
                    &location.name,
                    variant_name,
                )?;
                location.owner = remap.def_id(product_owner)?;
            }
            for arg in args {
                remap_expr_child_locations_with_scope(
                    arg,
                    remap,
                    validator,
                    type_validator,
                    local_scope,
                    depth + 1,
                )?;
            }
            Ok(())
        }
        HirExprKindFor::If {
            condition,
            then_branch,
            else_branch,
        } => {
            remap_expr_child_locations_with_scope(
                condition,
                remap,
                validator,
                type_validator,
                local_scope,
                depth + 1,
            )?;
            let mut then_scope = local_scope.clone();
            remap_block_child_locations(
                then_branch,
                remap,
                validator,
                type_validator,
                &mut then_scope,
                depth + 1,
            )?;
            if let Some(else_branch) = else_branch {
                let mut else_scope = local_scope.clone();
                remap_block_child_locations(
                    else_branch,
                    remap,
                    validator,
                    type_validator,
                    &mut else_scope,
                    depth + 1,
                )?;
            }
            Ok(())
        }
        HirExprKindFor::Match { scrutinee, arms } => {
            remap_expr_child_locations_with_scope(
                scrutinee,
                remap,
                validator,
                type_validator,
                local_scope,
                depth + 1,
            )?;
            for arm in arms {
                let mut arm_scope = local_scope.clone();
                remap_pattern_child_locations(
                    &mut arm.pattern,
                    remap,
                    validator,
                    type_validator,
                    depth + 1,
                )?;
                add_pattern_local_ids(&arm.pattern, &mut arm_scope);
                if let Some(guard) = &mut arm.guard {
                    remap_expr_child_locations_with_scope(
                        guard,
                        remap,
                        validator,
                        type_validator,
                        &mut arm_scope,
                        depth + 1,
                    )?;
                }
                remap_block_child_locations(
                    &mut arm.body,
                    remap,
                    validator,
                    type_validator,
                    &mut arm_scope,
                    depth + 1,
                )?;
            }
            Ok(())
        }
        HirExprKindFor::While { condition, body } => {
            remap_expr_child_locations_with_scope(
                condition,
                remap,
                validator,
                type_validator,
                local_scope,
                depth + 1,
            )?;
            let mut body_scope = local_scope.clone();
            remap_block_child_locations(
                body,
                remap,
                validator,
                type_validator,
                &mut body_scope,
                depth + 1,
            )
        }
        HirExprKindFor::For {
            local_id,
            iter,
            body,
            ..
        } => {
            remap_expr_child_locations_with_scope(
                iter,
                remap,
                validator,
                type_validator,
                local_scope,
                depth + 1,
            )?;
            let mut body_scope = local_scope.clone();
            body_scope.insert(*local_id);
            remap_block_child_locations(
                body,
                remap,
                validator,
                type_validator,
                &mut body_scope,
                depth + 1,
            )
        }
        HirExprKindFor::Loop(body)
        | HirExprKindFor::Block(body)
        | HirExprKindFor::UnsafeBlock(body) => {
            let mut body_scope = local_scope.clone();
            remap_block_child_locations(
                body,
                remap,
                validator,
                type_validator,
                &mut body_scope,
                depth + 1,
            )
        }
        HirExprKindFor::Lambda {
            params,
            body,
            captures,
        } => {
            let mut lambda_scope = BTreeSet::new();
            for param in params {
                remap_type_def_ids(&mut param.ty, remap, type_validator)?;
                lambda_scope.insert(param.local_id);
            }
            for capture in captures {
                if !local_scope.contains(&capture.local_id) {
                    return Err(format!(
                        "Product artifact references unknown local capture {:?}",
                        capture.local_id
                    ));
                }
                remap_type_def_ids(&mut capture.ty, remap, type_validator)?;
                lambda_scope.insert(capture.local_id);
            }
            remap_block_child_locations(
                body,
                remap,
                validator,
                type_validator,
                &mut lambda_scope,
                depth + 1,
            )
        }
        HirExprKindFor::Cast(value, ty) => {
            remap_type_def_ids(ty, remap, type_validator)?;
            remap_expr_child_locations_with_scope(
                value,
                remap,
                validator,
                type_validator,
                local_scope,
                depth + 1,
            )
        }
        HirExprKindFor::Intrinsic { args, .. } => {
            for arg in args {
                remap_expr_child_locations_with_scope(
                    arg,
                    remap,
                    validator,
                    type_validator,
                    local_scope,
                    depth + 1,
                )?;
            }
            Ok(())
        }
    }
}

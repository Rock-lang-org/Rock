use super::*;

/// Substitute only the generic identity or TypeVars reachable from the original
/// self type with a concrete type when injecting trait default methods into
/// concrete impl types.
pub fn substitute_typevars_in_function(
    func: &mut HirFunction,
    source_ty: &Type,
    concrete_ty: &Type,
) {
    let mut target_ids: HashSet<TypeVarId> = HashSet::new();
    let mut target_generic_ids: HashSet<crate::types::GenericParamId> = HashSet::new();
    collect_typevar_ids(source_ty, &mut target_ids);
    collect_generic_param_ids(source_ty, &mut target_generic_ids);

    for param in &mut func.params {
        substitute_typevar_in_type(&mut param.ty, &target_ids, &target_generic_ids, concrete_ty);
        substitute_generic_param_in_type(&mut param.ty, &target_generic_ids, concrete_ty);
    }
    substitute_typevar_in_type(
        &mut func.ret_type,
        &target_ids,
        &target_generic_ids,
        concrete_ty,
    );
    substitute_generic_param_in_type(&mut func.ret_type, &target_generic_ids, concrete_ty);
    substitute_typevars_in_block(
        &mut func.body,
        &target_ids,
        &target_generic_ids,
        concrete_ty,
    );
}

fn collect_typevar_ids(ty: &Type, target_ids: &mut HashSet<TypeVarId>) {
    crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
        if let Type::TypeVar(id) = nested {
            target_ids.insert(*id);
        }
    });
}

fn collect_generic_param_ids(ty: &Type, target_ids: &mut HashSet<crate::types::GenericParamId>) {
    crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
        if let Type::Generic(id) = nested {
            target_ids.insert(*id);
        }
    });
}

fn substitute_typevars_in_pattern(
    pattern: &mut HirPattern,
    target_ids: &HashSet<TypeVarId>,
    target_generic_ids: &HashSet<crate::types::GenericParamId>,
    concrete_ty: &Type,
) {
    match pattern {
        HirPattern::Tuple(patterns) | HirPattern::Or(patterns) => {
            for pattern in patterns {
                substitute_typevars_in_pattern(
                    pattern,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
        }
        HirPattern::Struct(_, _, type_args, field_patterns) => {
            for ty in type_args {
                substitute_typevar_in_type(ty, target_ids, target_generic_ids, concrete_ty);
                substitute_generic_param_in_type(ty, target_generic_ids, concrete_ty);
            }
            for field in field_patterns {
                substitute_typevars_in_pattern(
                    &mut field.pattern,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
        }
        HirPattern::Enum(_, _, _, patterns) => {
            for pattern in patterns {
                substitute_typevars_in_pattern(
                    pattern,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
        }
        HirPattern::Wildcard | HirPattern::Binding { .. } | HirPattern::Literal(_) => {}
    }
}

struct TargetTypeVarFolder<'a> {
    target_ids: &'a HashSet<TypeVarId>,
    concrete_ty: &'a Type,
}

impl crate::type_services::visit::TypeFolder for TargetTypeVarFolder<'_> {
    fn fold_type(&mut self, ty: Type) -> Type {
        match ty {
            Type::TypeVar(id) if self.target_ids.contains(&id) => self.concrete_ty.clone(),
            other => crate::type_services::visit::fold_type_children(other, self),
        }
    }
}

fn substitute_typevar_in_type(
    ty: &mut Type,
    target_ids: &HashSet<TypeVarId>,
    _target_generic_ids: &HashSet<crate::types::GenericParamId>,
    concrete_ty: &Type,
) {
    crate::type_services::visit::fold_type_in_place(
        ty,
        &mut TargetTypeVarFolder {
            target_ids,
            concrete_ty,
        },
    );
}

struct TargetGenericFolder<'a> {
    target_ids: &'a HashSet<crate::types::GenericParamId>,
    concrete_ty: &'a Type,
}

impl crate::type_services::visit::TypeFolder for TargetGenericFolder<'_> {
    fn fold_type(&mut self, ty: Type) -> Type {
        match ty {
            Type::Generic(id) if self.target_ids.contains(&id) => self.concrete_ty.clone(),
            other => crate::type_services::visit::fold_type_children(other, self),
        }
    }
}

fn substitute_generic_param_in_type(
    ty: &mut Type,
    target_ids: &HashSet<crate::types::GenericParamId>,
    concrete_ty: &Type,
) {
    crate::type_services::visit::fold_type_in_place(
        ty,
        &mut TargetGenericFolder {
            target_ids,
            concrete_ty,
        },
    );
}

fn substitute_typevars_in_block(
    block: &mut HirBlock,
    target_ids: &HashSet<TypeVarId>,
    target_generic_ids: &HashSet<crate::types::GenericParamId>,
    concrete_ty: &Type,
) {
    substitute_typevar_in_type(&mut block.ty, target_ids, target_generic_ids, concrete_ty);
    substitute_generic_param_in_type(&mut block.ty, target_generic_ids, concrete_ty);
    for stmt in &mut block.stmts {
        substitute_typevars_in_stmt(stmt, target_ids, target_generic_ids, concrete_ty);
    }
}

fn substitute_typevars_in_stmt(
    stmt: &mut HirStmt,
    target_ids: &HashSet<TypeVarId>,
    target_generic_ids: &HashSet<crate::types::GenericParamId>,
    concrete_ty: &Type,
) {
    match stmt {
        HirStmtFor::Let { ty, value, .. } => {
            substitute_typevar_in_type(ty, target_ids, target_generic_ids, concrete_ty);
            substitute_generic_param_in_type(ty, target_generic_ids, concrete_ty);
            substitute_typevars_in_expr_with_targets(
                value,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
        }
        HirStmtFor::Expr(expr) => {
            substitute_typevars_in_expr_with_targets(
                expr,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
        }
        HirStmtFor::Return(Some(expr)) | HirStmtFor::Break(Some(expr)) => {
            substitute_typevars_in_expr_with_targets(
                expr,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
        }
        HirStmtFor::Return(None) | HirStmtFor::Break(None) | HirStmtFor::Continue => {}
    }
}

fn substitute_typevars_in_method_target_with_targets(
    target: &mut HirMethodCallTarget,
    target_ids: &HashSet<TypeVarId>,
    target_generic_ids: &HashSet<crate::types::GenericParamId>,
    concrete_ty: &Type,
) {
    target.for_each_type_mut(|ty| {
        substitute_typevar_in_type(ty, target_ids, target_generic_ids, concrete_ty);
        substitute_generic_param_in_type(ty, target_generic_ids, concrete_ty);
    });
}

fn substitute_typevars_in_call_target_with_targets(
    target: &mut HirCallTarget,
    target_ids: &HashSet<TypeVarId>,
    target_generic_ids: &HashSet<crate::types::GenericParamId>,
    concrete_ty: &Type,
) {
    if let HirCallTarget::StaticMethod(target) = target {
        substitute_typevar_in_type(
            &mut target.owner_ty,
            target_ids,
            target_generic_ids,
            concrete_ty,
        );
        substitute_generic_param_in_type(&mut target.owner_ty, target_generic_ids, concrete_ty);
        substitute_typevars_in_method_target_with_targets(
            &mut target.method,
            target_ids,
            target_generic_ids,
            concrete_ty,
        );
    }
}

fn substitute_typevars_in_expr_with_targets(
    expr: &mut HirExpr,
    target_ids: &HashSet<TypeVarId>,
    target_generic_ids: &HashSet<crate::types::GenericParamId>,
    concrete_ty: &Type,
) {
    substitute_typevar_in_type(&mut expr.ty, target_ids, target_generic_ids, concrete_ty);
    substitute_generic_param_in_type(&mut expr.ty, target_generic_ids, concrete_ty);
    match &mut expr.kind {
        HirExprKindFor::IntLiteral(_)
        | HirExprKindFor::FloatLiteral(_)
        | HirExprKindFor::BoolLiteral(_)
        | HirExprKindFor::StringLiteral(_)
        | HirExprKindFor::CharLiteral(_)
        | HirExprKindFor::Unit
        | HirExprKindFor::Var(_)
        | HirExprKindFor::ResolvedVar(_) => {}
        HirExprKindFor::ArrayLiteral(elems) | HirExprKindFor::TupleLiteral(elems) => {
            for e in elems.iter_mut() {
                substitute_typevars_in_expr_with_targets(
                    e,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
        }
        HirExprKindFor::ArrayRepeat(value, _) => {
            substitute_typevars_in_expr_with_targets(
                value,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
        }
        HirExprKindFor::FieldAccess(base, _, _)
        | HirExprKindFor::Deref(base)
        | HirExprKindFor::Ref(_, base)
        | HirExprKindFor::UnaryOp(_, base) => {
            substitute_typevars_in_expr_with_targets(
                base,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
        }
        HirExprKindFor::TupleIndex(base, _) => {
            substitute_typevars_in_expr_with_targets(
                base,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
        }
        HirExprKindFor::BinOp(_, base, idx) | HirExprKindFor::Assign(base, idx) => {
            substitute_typevars_in_expr_with_targets(
                base,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
            substitute_typevars_in_expr_with_targets(
                idx,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
        }
        HirExprKindFor::Call(func, args, target) => {
            substitute_typevars_in_expr_with_targets(
                func,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
            for a in args.iter_mut() {
                substitute_typevars_in_expr_with_targets(
                    a,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
            if let Some(target) = target {
                substitute_typevars_in_call_target_with_targets(
                    target,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
        }
        HirExprKindFor::MethodCall(recv, _, args, _, target) => {
            substitute_typevars_in_expr_with_targets(
                recv,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
            for a in args.iter_mut() {
                substitute_typevars_in_expr_with_targets(
                    a,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
            if let Some(target) = target {
                substitute_typevars_in_method_target_with_targets(
                    target,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
        }
        HirExprKindFor::Try {
            expr,
            branch_method,
            from_residual_target,
            output_ty,
            residual_ty,
            return_ty,
            ..
        } => {
            substitute_typevars_in_expr_with_targets(
                expr,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
            if let Some(target) = branch_method {
                substitute_typevars_in_method_target_with_targets(
                    target,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
            if let Some(target) = from_residual_target {
                substitute_typevars_in_call_target_with_targets(
                    target,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
            for ty in [output_ty, residual_ty, return_ty] {
                substitute_typevar_in_type(ty, target_ids, target_generic_ids, concrete_ty);
                substitute_generic_param_in_type(ty, target_generic_ids, concrete_ty);
            }
        }
        HirExprKindFor::StructLiteral(_, _, fields) => {
            for field in fields.iter_mut() {
                substitute_typevars_in_expr_with_targets(
                    &mut field.value,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
        }
        HirExprKindFor::EnumVariant(_, _, args, _) => {
            for a in args.iter_mut() {
                substitute_typevars_in_expr_with_targets(
                    a,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
        }
        HirExprKindFor::If {
            condition,
            then_branch,
            else_branch,
        } => {
            substitute_typevars_in_expr_with_targets(
                condition,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
            substitute_typevars_in_block(then_branch, target_ids, target_generic_ids, concrete_ty);
            if let Some(eb) = else_branch {
                substitute_typevars_in_block(eb, target_ids, target_generic_ids, concrete_ty);
            }
        }
        HirExprKindFor::Match { scrutinee, arms } => {
            substitute_typevars_in_expr_with_targets(
                scrutinee,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
            for arm in arms.iter_mut() {
                substitute_typevars_in_pattern(
                    &mut arm.pattern,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
                if let Some(g) = &mut arm.guard {
                    substitute_typevars_in_expr_with_targets(
                        g,
                        target_ids,
                        target_generic_ids,
                        concrete_ty,
                    );
                }
                substitute_typevars_in_block(
                    &mut arm.body,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
        }
        HirExprKindFor::While { condition, body } => {
            substitute_typevars_in_expr_with_targets(
                condition,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
            substitute_typevars_in_block(body, target_ids, target_generic_ids, concrete_ty);
        }
        HirExprKindFor::For { iter, body, .. } => {
            substitute_typevars_in_expr_with_targets(
                iter,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
            substitute_typevars_in_block(body, target_ids, target_generic_ids, concrete_ty);
        }
        HirExprKindFor::Loop(body)
        | HirExprKindFor::Block(body)
        | HirExprKindFor::UnsafeBlock(body) => {
            substitute_typevars_in_block(body, target_ids, target_generic_ids, concrete_ty);
        }
        HirExprKindFor::Lambda {
            params,
            body,
            captures,
        } => {
            for p in params.iter_mut() {
                substitute_typevar_in_type(&mut p.ty, target_ids, target_generic_ids, concrete_ty);
                substitute_generic_param_in_type(&mut p.ty, target_generic_ids, concrete_ty);
            }
            for capture in captures.iter_mut() {
                substitute_typevar_in_type(
                    &mut capture.ty,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
                substitute_generic_param_in_type(&mut capture.ty, target_generic_ids, concrete_ty);
            }
            substitute_typevars_in_block(body, target_ids, target_generic_ids, concrete_ty);
        }
        HirExprKindFor::Cast(expr, ty) => {
            substitute_typevars_in_expr_with_targets(
                expr,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
            substitute_typevar_in_type(ty, target_ids, target_generic_ids, concrete_ty);
            substitute_generic_param_in_type(ty, target_generic_ids, concrete_ty);
        }
        HirExprKindFor::Intrinsic { args, .. } => {
            for a in args.iter_mut() {
                substitute_typevars_in_expr_with_targets(
                    a,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
        }
    }
}

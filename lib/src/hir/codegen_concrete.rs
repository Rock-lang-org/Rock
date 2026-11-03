use super::*;

pub fn hir_function_is_codegen_concrete<P: HirPhase>(function: &HirFunctionFor<P>) -> bool {
    hir_function_signature_is_codegen_concrete(function)
        && hir_block_is_codegen_concrete(&function.body)
}

pub(crate) fn hir_function_signature_is_codegen_concrete<P: HirPhase>(
    function: &HirFunctionFor<P>,
) -> bool {
    function
        .params
        .iter()
        .all(|param| hir_type_is_codegen_concrete(&param.ty))
        && hir_type_is_codegen_concrete(&function.ret_type)
}

pub(crate) fn function_requires_downstream_specialization<P: HirPhase>(
    function: &HirFunctionFor<P>,
) -> bool {
    !function.generic_params.is_empty()
}

pub(crate) fn function_is_object_provided_candidate<P: HirPhase>(
    function: &HirFunctionFor<P>,
) -> bool {
    !function_requires_downstream_specialization(function)
        && hir_function_is_codegen_concrete(function)
}

pub(crate) fn impl_requires_downstream_specialization<P: HirPhase>(imp: &HirImplFor<P>) -> bool {
    !imp.type_generics.is_empty()
        || !imp.trait_generics.is_empty()
        || imp
            .methods
            .values()
            .any(|method| !method.generic_params.is_empty())
}

fn hir_type_is_codegen_concrete(ty: &Type) -> bool {
    crate::type_services::facts::TypeFacts::is_codegen_concrete(ty)
}

fn hir_block_is_codegen_concrete<P: HirPhase>(block: &HirBlockFor<P>) -> bool {
    hir_type_is_codegen_concrete(&block.ty) && block.stmts.iter().all(hir_stmt_is_codegen_concrete)
}

fn hir_stmt_is_codegen_concrete<P: HirPhase>(stmt: &HirStmtFor<P>) -> bool {
    match stmt {
        HirStmtFor::Let { ty, value, .. } => {
            hir_type_is_codegen_concrete(ty) && hir_expr_is_codegen_concrete(value)
        }
        HirStmtFor::Expr(expr) | HirStmtFor::Return(Some(expr)) | HirStmtFor::Break(Some(expr)) => {
            hir_expr_is_codegen_concrete(expr)
        }
        HirStmtFor::Return(None) | HirStmtFor::Break(None) | HirStmtFor::Continue => true,
    }
}

fn hir_method_target_is_codegen_concrete(target: &HirMethodCallTarget) -> bool {
    target
        .trait_args()
        .iter()
        .all(crate::type_services::facts::TypeFacts::is_concrete_type_argument)
        && target
            .owner_substitution
            .iter()
            .chain(target.method_substitution.iter())
            .all(|binding| {
                crate::type_services::facts::TypeFacts::is_concrete_type_argument(&binding.ty)
            })
}

fn hir_call_target_is_codegen_concrete(target: &HirCallTarget) -> bool {
    match target {
        HirCallTarget::StaticMethod(target) => {
            crate::type_services::facts::TypeFacts::is_concrete_type_argument(&target.owner_ty)
                && hir_method_target_is_codegen_concrete(&target.method)
        }
        _ => true,
    }
}

fn hir_expr_is_codegen_concrete<P: HirPhase>(expr: &HirExprFor<P>) -> bool {
    if !hir_type_is_codegen_concrete(&expr.ty) {
        return false;
    }

    match &expr.kind {
        HirExprKindFor::IntLiteral(_)
        | HirExprKindFor::FloatLiteral(_)
        | HirExprKindFor::BoolLiteral(_)
        | HirExprKindFor::StringLiteral(_)
        | HirExprKindFor::CharLiteral(_)
        | HirExprKindFor::Unit
        | HirExprKindFor::Var(_)
        | HirExprKindFor::ResolvedVar(_) => true,
        HirExprKindFor::ArrayLiteral(elems) | HirExprKindFor::TupleLiteral(elems) => {
            elems.iter().all(hir_expr_is_codegen_concrete)
        }
        HirExprKindFor::ArrayRepeat(value, _) => hir_expr_is_codegen_concrete(value),
        HirExprKindFor::StructLiteral(_, _, fields) => fields
            .iter()
            .all(|field| hir_expr_is_codegen_concrete(&field.value)),
        HirExprKindFor::EnumVariant(_, _, args, _) | HirExprKindFor::Intrinsic { args, .. } => {
            args.iter().all(hir_expr_is_codegen_concrete)
        }
        HirExprKindFor::FieldAccess(base, _, _)
        | HirExprKindFor::TupleIndex(base, _)
        | HirExprKindFor::Deref(base)
        | HirExprKindFor::Ref(_, base)
        | HirExprKindFor::UnaryOp(_, base)
        | HirExprKindFor::Cast(base, _) => hir_expr_is_codegen_concrete(base),
        HirExprKindFor::BinOp(_, base, index) | HirExprKindFor::Assign(base, index) => {
            hir_expr_is_codegen_concrete(base) && hir_expr_is_codegen_concrete(index)
        }
        HirExprKindFor::Call(func, args, target) => {
            target
                .as_ref()
                .is_none_or(hir_call_target_is_codegen_concrete)
                && hir_expr_is_codegen_concrete(func)
                && args.iter().all(hir_expr_is_codegen_concrete)
        }
        HirExprKindFor::MethodCall(recv, _, args, _, target) => {
            P::method_authority(target).is_none_or(hir_method_target_is_codegen_concrete)
                && hir_expr_is_codegen_concrete(recv)
                && args.iter().all(hir_expr_is_codegen_concrete)
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
            hir_expr_is_codegen_concrete(expr)
                && P::method_authority(branch_method)
                    .is_none_or(hir_method_target_is_codegen_concrete)
                && P::residual_authority(from_residual_target)
                    .is_none_or(hir_call_target_is_codegen_concrete)
                && hir_type_is_codegen_concrete(output_ty)
                && hir_type_is_codegen_concrete(residual_ty)
                && hir_type_is_codegen_concrete(return_ty)
        }
        HirExprKindFor::If {
            condition,
            then_branch,
            else_branch,
        } => {
            hir_expr_is_codegen_concrete(condition)
                && hir_block_is_codegen_concrete(then_branch)
                && else_branch
                    .as_ref()
                    .is_none_or(|block| hir_block_is_codegen_concrete(block))
        }
        HirExprKindFor::Match { scrutinee, arms } => {
            hir_expr_is_codegen_concrete(scrutinee)
                && arms.iter().all(|arm| {
                    hir_pattern_is_codegen_concrete(&arm.pattern)
                        && arm.guard.as_ref().is_none_or(hir_expr_is_codegen_concrete)
                        && hir_block_is_codegen_concrete(&arm.body)
                })
        }
        HirExprKindFor::While { condition, body } => {
            hir_expr_is_codegen_concrete(condition) && hir_block_is_codegen_concrete(body)
        }
        HirExprKindFor::For { iter, body, .. } => {
            hir_expr_is_codegen_concrete(iter) && hir_block_is_codegen_concrete(body)
        }
        HirExprKindFor::Loop(body)
        | HirExprKindFor::Block(body)
        | HirExprKindFor::UnsafeBlock(body) => hir_block_is_codegen_concrete(body),
        HirExprKindFor::Lambda {
            params,
            body,
            captures,
        } => {
            params
                .iter()
                .all(|param| hir_type_is_codegen_concrete(&param.ty))
                && captures
                    .iter()
                    .all(|capture| hir_type_is_codegen_concrete(&capture.ty))
                && hir_block_is_codegen_concrete(body)
        }
    }
}

fn hir_pattern_is_codegen_concrete(pattern: &HirPattern) -> bool {
    match pattern {
        HirPattern::Wildcard | HirPattern::Binding { .. } | HirPattern::Literal(_) => true,
        HirPattern::Tuple(patterns) | HirPattern::Or(patterns) => {
            patterns.iter().all(hir_pattern_is_codegen_concrete)
        }
        HirPattern::Struct(_, _, type_args, fields) => {
            type_args.iter().all(hir_type_is_codegen_concrete)
                && fields
                    .iter()
                    .all(|field| hir_pattern_is_codegen_concrete(&field.pattern))
        }
        HirPattern::Enum(_, _, _, patterns) => patterns.iter().all(hir_pattern_is_codegen_concrete),
    }
}

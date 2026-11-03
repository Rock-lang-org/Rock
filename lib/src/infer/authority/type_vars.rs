use std::collections::HashSet;

use crate::hir::{HirBlock, HirExpr, HirExprKind, HirFunction, HirStmt};
use crate::infer::constraints::Constraint;
use crate::infer::{ConstraintOwner, PartialHir};
use crate::types::Type;

use super::materialize_pending::collect_unresolved_authority_vars;

pub(super) fn collect_type_vars(ty: &Type, vars: &mut HashSet<crate::ids::TypeVarId>) {
    crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
        if let Type::TypeVar(id) = nested {
            vars.insert(*id);
        }
    });
}

pub(super) fn collect_block_type_vars(
    block: &HirBlock,
    engine: &crate::infer::InferenceEngine,
    output: &mut HashSet<crate::ids::TypeVarId>,
) {
    fn expr(
        node: &HirExpr,
        engine: &crate::infer::InferenceEngine,
        output: &mut HashSet<crate::ids::TypeVarId>,
    ) {
        collect_type_vars(&engine.resolve(&node.ty), output);
        match &node.kind {
            HirExprKind::Call(callee, args, _) | HirExprKind::MethodCall(callee, _, args, _, _) => {
                expr(callee, engine, output);
                for arg in args {
                    expr(arg, engine, output);
                }
            }
            HirExprKind::Try {
                expr: operand,
                output_ty,
                residual_ty,
                return_ty,
                ..
            } => {
                expr(operand, engine, output);
                for ty in [output_ty, residual_ty, return_ty] {
                    collect_type_vars(&engine.resolve(ty), output);
                }
            }
            HirExprKind::FieldAccess(inner, _, _)
            | HirExprKind::Deref(inner)
            | HirExprKind::Ref(_, inner)
            | HirExprKind::Cast(inner, _)
            | HirExprKind::TupleIndex(inner, _)
            | HirExprKind::UnaryOp(_, inner)
            | HirExprKind::ArrayRepeat(inner, _) => expr(inner, engine, output),
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                expr(condition, engine, output);
                collect_block_type_vars(then_branch, engine, output);
                if let Some(else_branch) = else_branch {
                    collect_block_type_vars(else_branch, engine, output);
                }
            }
            HirExprKind::Match { scrutinee, arms } => {
                expr(scrutinee, engine, output);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        expr(guard, engine, output);
                    }
                    collect_block_type_vars(&arm.body, engine, output);
                }
            }
            HirExprKind::While { condition, body } => {
                expr(condition, engine, output);
                collect_block_type_vars(body, engine, output);
            }
            HirExprKind::For { iter, body, .. } => {
                expr(iter, engine, output);
                collect_block_type_vars(body, engine, output);
            }
            HirExprKind::Block(body) | HirExprKind::Loop(body) | HirExprKind::UnsafeBlock(body) => {
                collect_block_type_vars(body, engine, output)
            }
            HirExprKind::Lambda { params, body, .. } => {
                for param in params {
                    collect_type_vars(&engine.resolve(&param.ty), output);
                }
                collect_block_type_vars(body, engine, output);
            }
            HirExprKind::Assign(left, right) | HirExprKind::BinOp(_, left, right) => {
                expr(left, engine, output);
                expr(right, engine, output);
            }
            HirExprKind::Intrinsic { args, .. }
            | HirExprKind::TupleLiteral(args)
            | HirExprKind::ArrayLiteral(args)
            | HirExprKind::EnumVariant(_, _, args, _) => {
                for arg in args {
                    expr(arg, engine, output);
                }
            }
            HirExprKind::StructLiteral(_, _, fields) => {
                for field in fields {
                    expr(&field.value, engine, output);
                }
            }
            HirExprKind::Var(_)
            | HirExprKind::ResolvedVar(_)
            | HirExprKind::IntLiteral(_)
            | HirExprKind::FloatLiteral(_)
            | HirExprKind::BoolLiteral(_)
            | HirExprKind::StringLiteral(_)
            | HirExprKind::CharLiteral(_)
            | HirExprKind::Unit => {}
        }
    }

    collect_type_vars(&engine.resolve(&block.ty), output);
    for statement in &block.stmts {
        match statement {
            HirStmt::Let { ty, value, .. } => {
                collect_type_vars(&engine.resolve(ty), output);
                expr(value, engine, output);
            }
            HirStmt::Expr(value) | HirStmt::Return(Some(value)) | HirStmt::Break(Some(value)) => {
                expr(value, engine, output)
            }
            HirStmt::Return(None) | HirStmt::Break(None) | HirStmt::Continue => {}
        }
    }
}

pub(super) fn constrained_signature_vars(
    hir: &PartialHir,
    function: &HirFunction,
) -> HashSet<crate::ids::TypeVarId> {
    let mut signature_vars = HashSet::new();
    for ty in function
        .params
        .iter()
        .map(|param| &param.ty)
        .chain(std::iter::once(&function.ret_type))
    {
        collect_type_vars(&hir.engine.resolve(ty), &mut signature_vars);
    }
    let mut shared = HashSet::new();
    for (obligation, constraint) in hir.constraint_store.iter() {
        if hir.constraint_store.owner(obligation) != Some(ConstraintOwner::Body(function.id)) {
            continue;
        }
        let mut obligation_vars = HashSet::new();
        match constraint {
            Constraint::Trait { ty, bound, .. } => {
                if matches!(hir.engine.resolve(ty), Type::TypeVar(_))
                    && (hir
                        .language_items
                        .callable_protocols()
                        .any(|(_, id, _)| id == bound.trait_id)
                        || hir
                            .language_items
                            .fold
                            .as_ref()
                            .is_some_and(|fold| fold.trait_id == bound.trait_id))
                {
                    // Callable and fold requirements are polymorphic predicates, not
                    // evidence fixing the declaration to one caller's type.
                    continue;
                }
                collect_type_vars(&hir.engine.resolve(ty), &mut obligation_vars);
                for arg in &bound.type_args {
                    collect_type_vars(&hir.engine.resolve(arg), &mut obligation_vars);
                }
            }
            Constraint::Equality { left, right, .. } => {
                collect_type_vars(&hir.engine.resolve(left), &mut obligation_vars);
                collect_type_vars(&hir.engine.resolve(right), &mut obligation_vars);
            }
            Constraint::Coercion {
                actual, expected, ..
            } => {
                collect_type_vars(&hir.engine.resolve(actual), &mut obligation_vars);
                collect_type_vars(&hir.engine.resolve(expected), &mut obligation_vars);
            }
            Constraint::Try {
                carrier,
                output,
                residual,
                return_ty,
                ..
            } => {
                for ty in [carrier, output, residual, return_ty] {
                    collect_type_vars(&hir.engine.resolve(ty), &mut obligation_vars);
                }
            }
            Constraint::IntLiteral { var, .. } | Constraint::FloatLiteral { var, .. } => {
                collect_type_vars(
                    &hir.engine.resolve(&Type::TypeVar(*var)),
                    &mut obligation_vars,
                );
            }
        }
        shared.extend(signature_vars.intersection(&obligation_vars).copied());
    }
    let mut authority_vars = HashSet::new();
    collect_unresolved_authority_vars(&function.body, &hir.engine, &mut authority_vars);
    shared.extend(signature_vars.intersection(&authority_vars).copied());
    shared
}

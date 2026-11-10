use std::collections::HashSet;

use crate::hir::{HirBlock, HirExpr, HirExprKind, HirStmt};
use crate::infer::{ConstraintOwner, ObligationState, PartialHir};
use crate::lower::ResolveError;
use crate::types::Type;

use super::{AuthorityObligation, AuthorityObligationKind, AuthorityPassResult};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct AuthorityCounts {
    pub(super) methods: usize,
    pub(super) pending_methods: usize,
    pub(super) residuals: usize,
    pub(super) pending_residuals: usize,
    pub(super) fields: usize,
    pub(super) pending_fields: usize,
}

pub(in crate::infer) fn pending_authority_error(
    hir: &PartialHir,
    owners: Option<&HashSet<ConstraintOwner>>,
    obligations: &[AuthorityObligation],
) -> ResolveError {
    let message = if authority_counts(hir, owners).pending_residuals > 0 {
        "cannot resolve '?' because its carrier or enclosing residual remains unresolved"
            .to_string()
    } else if authority_counts(hir, owners).pending_methods > 0 {
        "cannot resolve method authority because the receiver remains unresolved".to_string()
    } else if let Some((member, receiver)) = first_pending_field(hir, owners) {
        format!(
            "cannot resolve member '{member}' because receiver type {} remains unresolved",
            hir.engine.display_type(&receiver)
        )
    } else {
        "cannot resolve field authority because the receiver remains unresolved".to_string()
    };
    let span = obligations
        .iter()
        .find(|obligation| {
            obligation.kind != AuthorityObligationKind::Propagation
                && obligation.state != ObligationState::Solved
        })
        .and_then(|obligation| obligation.span.clone())
        .expect("pending source authority obligation must carry its expression span");
    ResolveError::with_span_code(message, span, crate::diagnostic::DiagnosticCode::Selection)
}

pub(in crate::infer) fn pending_authority_context(
    obligations: &[AuthorityObligation],
) -> Option<&str> {
    obligations
        .iter()
        .find(|obligation| {
            obligation.kind != AuthorityObligationKind::Propagation
                && obligation.state != ObligationState::Solved
        })
        .map(|obligation| obligation.context.as_str())
}

fn first_pending_field(
    hir: &PartialHir,
    owners: Option<&HashSet<ConstraintOwner>>,
) -> Option<(String, Type)> {
    fn block(block: &HirBlock, engine: &crate::infer::InferenceEngine) -> Option<(String, Type)> {
        block.stmts.iter().find_map(|statement| match statement {
            HirStmt::Let { value, .. }
            | HirStmt::Expr(value)
            | HirStmt::Return(Some(value))
            | HirStmt::Break(Some(value)) => expr(value, engine),
            HirStmt::Return(None) | HirStmt::Break(None) | HirStmt::Continue => None,
        })
    }

    fn expr(node: &HirExpr, engine: &crate::infer::InferenceEngine) -> Option<(String, Type)> {
        match &node.kind {
            HirExprKind::FieldAccess(receiver, name, None) => {
                Some((name.clone(), engine.resolve(&receiver.ty)))
            }
            HirExprKind::FieldAccess(receiver, _, Some(_)) => expr(receiver, engine),
            HirExprKind::Call(callee, args, _) => {
                expr(callee, engine).or_else(|| args.iter().find_map(|arg| expr(arg, engine)))
            }
            HirExprKind::MethodCall(receiver, _, args, _, _) => {
                expr(receiver, engine).or_else(|| args.iter().find_map(|arg| expr(arg, engine)))
            }
            HirExprKind::Try { expr: operand, .. }
            | HirExprKind::Deref(operand)
            | HirExprKind::Ref(_, operand)
            | HirExprKind::Cast(operand, _)
            | HirExprKind::TupleIndex(operand, _)
            | HirExprKind::UnaryOp(_, operand)
            | HirExprKind::ArrayRepeat(operand, _) => expr(operand, engine),
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => expr(condition, engine)
                .or_else(|| block(then_branch, engine))
                .or_else(|| else_branch.as_ref().and_then(|body| block(body, engine))),
            HirExprKind::Match { scrutinee, arms } => expr(scrutinee, engine).or_else(|| {
                arms.iter().find_map(|arm| {
                    arm.guard
                        .as_ref()
                        .and_then(|guard| expr(guard, engine))
                        .or_else(|| block(&arm.body, engine))
                })
            }),
            HirExprKind::While { condition, body } => {
                expr(condition, engine).or_else(|| block(body, engine))
            }
            HirExprKind::For { iter, body, .. } => {
                expr(iter, engine).or_else(|| block(body, engine))
            }
            HirExprKind::Block(body)
            | HirExprKind::Loop(body)
            | HirExprKind::UnsafeBlock(body)
            | HirExprKind::Lambda { body, .. } => block(body, engine),
            HirExprKind::Assign(left, right) | HirExprKind::BinOp(_, left, right) => {
                expr(left, engine).or_else(|| expr(right, engine))
            }
            HirExprKind::Intrinsic { args, .. }
            | HirExprKind::TupleLiteral(args)
            | HirExprKind::ArrayLiteral(args)
            | HirExprKind::EnumVariant(_, _, args, _) => {
                args.iter().find_map(|arg| expr(arg, engine))
            }
            HirExprKind::StructLiteral(_, _, fields) => {
                fields.iter().find_map(|field| expr(&field.value, engine))
            }
            HirExprKind::Var(_)
            | HirExprKind::ResolvedVar(_)
            | HirExprKind::IntLiteral(_) | HirExprKind::ConstParam(_)
            | HirExprKind::FloatLiteral(_)
            | HirExprKind::BoolLiteral(_)
            | HirExprKind::StringLiteral(_)
            | HirExprKind::CharLiteral(_)
            | HirExprKind::Unit => None,
        }
    }

    hir.functions
        .iter()
        .filter(|(id, _)| owner_selected(owners, ConstraintOwner::Body(**id)))
        .map(|(_, function)| function)
        .chain(
            hir.traits
                .values()
                .flat_map(|trait_def| trait_def.methods.values())
                .filter(|function| owner_selected(owners, ConstraintOwner::Body(function.id))),
        )
        .chain(
            hir.impls
                .values()
                .flat_map(|imp| imp.methods.values())
                .filter(|function| owner_selected(owners, ConstraintOwner::Body(function.id))),
        )
        .find_map(|function| block(&function.body, &hir.engine))
}

pub(super) fn owner_selected(
    owners: Option<&HashSet<ConstraintOwner>>,
    owner: ConstraintOwner,
) -> bool {
    owners.is_none_or(|owners| owners.contains(&owner))
}

pub(super) fn authority_pass_result(
    hir: &PartialHir,
    before_generation: u64,
    before_counts: AuthorityCounts,
    owners: Option<&HashSet<ConstraintOwner>>,
) -> AuthorityPassResult {
    let after_counts = authority_counts(hir, owners);
    AuthorityPassResult {
        progress: before_generation != hir.engine.rigid_substitution_generation()
            || before_counts != after_counts,
        pending: after_counts.pending_methods > 0
            || after_counts.pending_residuals > 0
            || after_counts.pending_fields > 0,
        ambiguous: false,
    }
}

pub(super) fn authority_counts(
    hir: &PartialHir,
    owners: Option<&HashSet<ConstraintOwner>>,
) -> AuthorityCounts {
    fn block(block: &HirBlock, counts: &mut AuthorityCounts) {
        for statement in &block.stmts {
            match statement {
                HirStmt::Let { value, .. }
                | HirStmt::Expr(value)
                | HirStmt::Return(Some(value))
                | HirStmt::Break(Some(value)) => expr(value, counts),
                HirStmt::Return(None) | HirStmt::Break(None) | HirStmt::Continue => {}
            }
        }
    }

    fn expr(node: &HirExpr, counts: &mut AuthorityCounts) {
        match &node.kind {
            HirExprKind::MethodCall(receiver, _, args, _, target) => {
                if target.is_some() {
                    counts.methods += 1;
                } else {
                    counts.pending_methods += 1;
                }
                expr(receiver, counts);
                for arg in args {
                    expr(arg, counts);
                }
            }
            HirExprKind::Try {
                expr: operand,
                branch_method,
                from_residual_target,
                ..
            } => {
                if branch_method.is_some() {
                    counts.methods += 1;
                } else {
                    counts.pending_residuals += 1;
                }
                if from_residual_target.is_some() {
                    counts.residuals += 1;
                } else {
                    counts.pending_residuals += 1;
                }
                expr(operand, counts);
            }
            HirExprKind::FieldAccess(receiver, _, location) => {
                if location.is_some() {
                    counts.fields += 1;
                } else {
                    counts.pending_fields += 1;
                }
                expr(receiver, counts);
            }
            HirExprKind::Call(callee, args, _) => {
                expr(callee, counts);
                for arg in args {
                    expr(arg, counts);
                }
            }
            HirExprKind::Deref(inner)
            | HirExprKind::Ref(_, inner)
            | HirExprKind::Cast(inner, _)
            | HirExprKind::TupleIndex(inner, _)
            | HirExprKind::UnaryOp(_, inner)
            | HirExprKind::ArrayRepeat(inner, _) => expr(inner, counts),
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                expr(condition, counts);
                block(then_branch, counts);
                if let Some(else_branch) = else_branch {
                    block(else_branch, counts);
                }
            }
            HirExprKind::Match { scrutinee, arms } => {
                expr(scrutinee, counts);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        expr(guard, counts);
                    }
                    block(&arm.body, counts);
                }
            }
            HirExprKind::While { condition, body } => {
                expr(condition, counts);
                block(body, counts);
            }
            HirExprKind::For { iter, body, .. } => {
                expr(iter, counts);
                block(body, counts);
            }
            HirExprKind::Block(body) | HirExprKind::Loop(body) | HirExprKind::UnsafeBlock(body) => {
                block(body, counts)
            }
            HirExprKind::Lambda { body, .. } => block(body, counts),
            HirExprKind::Assign(left, right) | HirExprKind::BinOp(_, left, right) => {
                expr(left, counts);
                expr(right, counts);
            }
            HirExprKind::Intrinsic { args, .. }
            | HirExprKind::TupleLiteral(args)
            | HirExprKind::ArrayLiteral(args)
            | HirExprKind::EnumVariant(_, _, args, _) => {
                for arg in args {
                    expr(arg, counts);
                }
            }
            HirExprKind::StructLiteral(_, _, fields) => {
                for field in fields {
                    expr(&field.value, counts);
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

    let mut counts = AuthorityCounts::default();
    for (id, function) in &hir.functions {
        if owner_selected(owners, ConstraintOwner::Body(*id)) {
            block(&function.body, &mut counts);
        }
    }
    for trait_def in hir.traits.values() {
        for function in trait_def.methods.values() {
            if owner_selected(owners, ConstraintOwner::Body(function.id)) {
                block(&function.body, &mut counts);
            }
        }
    }
    for impl_def in hir.impls.values() {
        for function in impl_def.methods.values() {
            if owner_selected(owners, ConstraintOwner::Body(function.id)) {
                block(&function.body, &mut counts);
            }
        }
    }
    counts
}

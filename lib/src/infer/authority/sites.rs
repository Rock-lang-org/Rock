use std::collections::HashSet;

use crate::hir::{HirBlock, HirExpr, HirExprKind, HirStmt};
use crate::infer::{ConstraintOwner, PartialHir};
use crate::lower::ResolveError;
use crate::types::Type;

use super::materialize_pending::materialize_pending;
use super::obligations::function_for_owner;
use super::status::{authority_counts, authority_pass_result};
use super::type_predicates::contains_recovery_type;
use super::type_vars::collect_type_vars;
use super::{native_call_type, AuthorityObligationKind, AuthorityPassResult, AuthoritySiteId};

type AuthoritySite = (
    AuthorityObligationKind,
    AuthoritySiteId,
    String,
    crate::lexer::Span,
    HashSet<crate::ids::TypeVarId>,
);

pub(super) fn collect_authority_sites(
    root: &HirBlock,
    engine: &crate::infer::InferenceEngine,
    output: &mut Vec<AuthoritySite>,
) {
    fn dependencies(
        types: impl IntoIterator<Item = Type>,
        engine: &crate::infer::InferenceEngine,
    ) -> HashSet<crate::ids::TypeVarId> {
        let mut output = HashSet::new();
        for ty in types {
            collect_type_vars(&engine.resolve(&ty), &mut output);
        }
        output
    }

    fn block(
        body: &HirBlock,
        engine: &crate::infer::InferenceEngine,
        output: &mut Vec<AuthoritySite>,
    ) {
        for statement in &body.stmts {
            match statement {
                HirStmt::Let { value, .. }
                | HirStmt::Expr(value)
                | HirStmt::Return(Some(value))
                | HirStmt::Break(Some(value)) => expr(value, engine, output, false),
                HirStmt::Return(None) | HirStmt::Break(None) | HirStmt::Continue => {}
            }
        }
    }

    fn expr(
        node: &HirExpr,
        engine: &crate::infer::InferenceEngine,
        output: &mut Vec<AuthoritySite>,
        suppress_field_slot: bool,
    ) {
        match &node.kind {
            HirExprKind::Call(callee, args, target) => {
                if !native_call_type(&callee.ty)
                    && !matches!(callee.kind, HirExprKind::FieldAccess(..))
                {
                    let kind = AuthorityObligationKind::CallableCall;
                    output.push((
                        kind,
                        AuthoritySiteId::new(kind, &node.span),
                        "function call".into(),
                        node.span.clone(),
                        dependencies(
                            std::iter::once(callee.ty.clone())
                                .chain(std::iter::once(node.ty.clone()))
                                .chain(args.iter().map(|arg| arg.ty.clone())),
                            engine,
                        ),
                    ));
                }
                if let HirExprKind::FieldAccess(receiver, name, location) = &callee.kind {
                    if target.is_none()
                        && (location.is_none()
                            || contains_recovery_type(&engine.resolve(&receiver.ty)))
                    {
                        let kind = AuthorityObligationKind::DeferredCall;
                        let types = std::iter::once(node.ty.clone())
                            .chain(std::iter::once(receiver.ty.clone()))
                            .chain(args.iter().map(|arg| arg.ty.clone()));
                        output.push((
                            kind,
                            AuthoritySiteId::new(kind, &node.span),
                            format!("method or operator call '{name}'"),
                            node.span.clone(),
                            dependencies(types, engine),
                        ));
                    }
                    expr(callee, engine, output, true);
                } else {
                    expr(callee, engine, output, false);
                }
                for arg in args {
                    expr(arg, engine, output, false);
                }
            }
            HirExprKind::MethodCall(receiver, name, args, _, target) => {
                if target.is_none() {
                    let kind = AuthorityObligationKind::MethodCall;
                    let types = std::iter::once(node.ty.clone())
                        .chain(std::iter::once(receiver.ty.clone()))
                        .chain(args.iter().map(|arg| arg.ty.clone()));
                    output.push((
                        kind,
                        AuthoritySiteId::new(kind, &node.span),
                        format!("method call '{name}'"),
                        node.span.clone(),
                        dependencies(types, engine),
                    ));
                }
                expr(receiver, engine, output, false);
                for arg in args {
                    expr(arg, engine, output, false);
                }
            }
            HirExprKind::FieldAccess(receiver, name, location) => {
                if !suppress_field_slot {
                    if location.is_none() {
                        let kind = AuthorityObligationKind::Field;
                        output.push((
                            kind,
                            AuthoritySiteId::new(kind, &node.span),
                            format!("field access '{name}'"),
                            node.span.clone(),
                            dependencies([node.ty.clone(), receiver.ty.clone()], engine),
                        ));
                    }
                }
                expr(receiver, engine, output, false);
            }
            HirExprKind::Try {
                expr: operand,
                branch_method,
                from_residual_target,
                output_ty,
                residual_ty,
                return_ty,
                ..
            } => {
                if branch_method.is_none() {
                    let kind = AuthorityObligationKind::TryBranch;
                    output.push((
                        kind,
                        AuthoritySiteId::new(kind, &node.span),
                        "Try::branch".to_string(),
                        node.span.clone(),
                        dependencies(
                            [operand.ty.clone(), output_ty.clone(), residual_ty.clone()],
                            engine,
                        ),
                    ));
                }
                if from_residual_target.is_none() {
                    let kind = AuthorityObligationKind::FromResidual;
                    output.push((
                        kind,
                        AuthoritySiteId::new(kind, &node.span),
                        "FromResidual::from_residual".to_string(),
                        node.span.clone(),
                        dependencies([residual_ty.clone(), return_ty.clone()], engine),
                    ));
                }
                expr(operand, engine, output, false);
            }
            HirExprKind::Deref(inner)
            | HirExprKind::Ref(_, inner)
            | HirExprKind::Cast(inner, _)
            | HirExprKind::TupleIndex(inner, _)
            | HirExprKind::UnaryOp(_, inner)
            | HirExprKind::ArrayRepeat(inner, _) => expr(inner, engine, output, false),
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                expr(condition, engine, output, false);
                block(then_branch, engine, output);
                if let Some(else_branch) = else_branch {
                    block(else_branch, engine, output);
                }
            }
            HirExprKind::Match { scrutinee, arms } => {
                expr(scrutinee, engine, output, false);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        expr(guard, engine, output, false);
                    }
                    block(&arm.body, engine, output);
                }
            }
            HirExprKind::While { condition, body } => {
                expr(condition, engine, output, false);
                block(body, engine, output);
            }
            HirExprKind::For { iter, body, .. } => {
                expr(iter, engine, output, false);
                block(body, engine, output);
            }
            HirExprKind::Block(body)
            | HirExprKind::Loop(body)
            | HirExprKind::UnsafeBlock(body)
            | HirExprKind::Lambda { body, .. } => block(body, engine, output),
            HirExprKind::Assign(left, right) | HirExprKind::BinOp(_, left, right) => {
                expr(left, engine, output, false);
                expr(right, engine, output, false);
            }
            HirExprKind::Intrinsic { args, .. }
            | HirExprKind::TupleLiteral(args)
            | HirExprKind::ArrayLiteral(args)
            | HirExprKind::EnumVariant(_, _, args, _) => {
                for arg in args {
                    expr(arg, engine, output, false);
                }
            }
            HirExprKind::StructLiteral(_, _, fields) => {
                for field in fields {
                    expr(&field.value, engine, output, false);
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

    block(root, engine, output);
}

pub(super) fn materialize_authority_site(
    hir: &mut PartialHir,
    owner: ConstraintOwner,
    site: AuthoritySiteId,
    strict: bool,
    try_strict: bool,
) -> Result<AuthorityPassResult, Vec<ResolveError>> {
    let owners = HashSet::from([owner]);
    let before_generation = hir.engine.rigid_substitution_generation();
    let before_counts = authority_counts(hir, Some(&owners));
    let ambiguous =
        materialize_pending(hir, strict, try_strict, Some(&owners), Some(site.clone()))?;
    let mut result = authority_pass_result(hir, before_generation, before_counts, Some(&owners));
    result.pending = function_for_owner(hir, owner).is_some_and(|function| {
        let mut sites = Vec::new();
        collect_authority_sites(&function.body, &hir.engine, &mut sites);
        sites.iter().any(|(_, candidate, ..)| *candidate == site)
    });
    result.ambiguous = ambiguous;
    Ok(result)
}

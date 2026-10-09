//! Shared structural HIR traversal; visiting syntax grants no semantic authority.

use super::{HirBlockFor, HirExprFor, HirExprKindFor as E, HirPhase, HirStmtFor};

pub(crate) enum HirChild<'a, P: HirPhase> {
    Expr(&'a HirExprFor<P>),
    Block(&'a HirBlockFor<P>),
}

pub(crate) fn visit_expr_children<'a, P: HirPhase>(
    expr: &'a HirExprFor<P>,
    visit: &mut impl FnMut(HirChild<'a, P>),
) {
    match &expr.kind {
        E::PackagePack { payload, .. } => visit(HirChild::Expr(payload)),
        E::Open { source, body, .. } | E::PackageOpen { source, body, .. } => {
            visit(HirChild::Expr(source));
            visit(HirChild::Block(body));
        }
        E::OwnedObjectCall { owner, args, .. } => {
            visit(HirChild::Expr(owner));
            args.iter().for_each(|expr| visit(HirChild::Expr(expr)));
        }
        E::ArrayLiteral(values) | E::TupleLiteral(values) => {
            values.iter().for_each(|expr| visit(HirChild::Expr(expr)))
        }
        E::ArrayRepeat(value, _)
        | E::ObjectCoercion(value, _)
        | E::UnaryOp(_, value)
        | E::FieldAccess(value, _, _)
        | E::TupleIndex(value, _)
        | E::Ref(_, value)
        | E::Deref(value)
        | E::Cast(value, _) => visit(HirChild::Expr(value)),
        E::BinOp(_, left, right) | E::Assign(left, right) => {
            visit(HirChild::Expr(left));
            visit(HirChild::Expr(right));
        }
        E::Call(callee, args, _) | E::MethodCall(callee, _, args, _, _) => {
            visit(HirChild::Expr(callee));
            args.iter().for_each(|expr| visit(HirChild::Expr(expr)));
        }
        E::Try { expr, .. } => visit(HirChild::Expr(expr)),
        E::StructLiteral(_, _, fields) => fields
            .iter()
            .for_each(|field| visit(HirChild::Expr(&field.value))),
        E::EnumVariant(_, _, args, _) | E::Intrinsic { args, .. } => {
            args.iter().for_each(|expr| visit(HirChild::Expr(expr)))
        }
        E::If {
            condition,
            then_branch,
            else_branch,
        } => {
            visit(HirChild::Expr(condition));
            visit(HirChild::Block(then_branch));
            if let Some(branch) = else_branch {
                visit(HirChild::Block(branch));
            }
        }
        E::Match { scrutinee, arms } => {
            visit(HirChild::Expr(scrutinee));
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    visit(HirChild::Expr(guard));
                }
                visit(HirChild::Block(&arm.body));
            }
        }
        E::While { condition, body } => {
            visit(HirChild::Expr(condition));
            visit(HirChild::Block(body));
        }
        E::For { iter, body, .. } => {
            visit(HirChild::Expr(iter));
            visit(HirChild::Block(body));
        }
        E::Loop(body) | E::Block(body) | E::UnsafeBlock(body) | E::Lambda { body, .. } => {
            visit(HirChild::Block(body))
        }
        E::IntLiteral(_)
        | E::FloatLiteral(_)
        | E::BoolLiteral(_)
        | E::StringLiteral(_)
        | E::CharLiteral(_)
        | E::Unit
        | E::Var(_)
        | E::ResolvedVar(_) => {}
    }
}

pub(crate) fn visit_block<P: HirPhase>(
    block: &HirBlockFor<P>,
    visit: &mut impl FnMut(&HirExprFor<P>),
) {
    for statement in &block.stmts {
        let expression = match statement {
            HirStmtFor::Let { value, .. }
            | HirStmtFor::Expr(value)
            | HirStmtFor::Return(Some(value))
            | HirStmtFor::Break(Some(value)) => Some(value),
            _ => None,
        };
        if let Some(expression) = expression {
            visit_expr(expression, visit);
        }
    }
}

pub(crate) enum HirChildMut<'a, P: HirPhase> {
    Expr(&'a mut HirExprFor<P>),
    Block(&'a mut HirBlockFor<P>),
}

/// Conservatively find a break owned by the loop containing this body.
/// Nested loop bodies and lambdas have independent exit owners; a nested
/// while condition or for iterator is still evaluated in the outer owner.
pub(crate) fn has_loop_break<P: HirPhase>(body: &HirBlockFor<P>) -> bool {
    fn expression<P: HirPhase>(expr: &HirExprFor<P>) -> bool {
        match &expr.kind {
            E::Lambda { .. } | E::Loop(_) => false,
            E::While { condition, .. } => expression(condition),
            E::For { iter, .. } => expression(iter),
            _ => {
                let mut found = false;
                visit_expr_children(expr, &mut |child| {
                    found |= match child {
                        HirChild::Expr(expr) => expression(expr),
                        HirChild::Block(block) => has_loop_break(block),
                    };
                });
                found
            }
        }
    }
    body.stmts.iter().any(|statement| match statement {
        HirStmtFor::Break(_) => true,
        HirStmtFor::Let { value, .. } | HirStmtFor::Expr(value)
            | HirStmtFor::Return(Some(value)) => expression(value),
        _ => false,
    })
}

pub(crate) fn visit_expr_children_mut<P: HirPhase>(
    expr: &mut HirExprFor<P>,
    visit: &mut impl FnMut(HirChildMut<'_, P>),
) {
    match &mut expr.kind {
        E::PackagePack { payload, .. } => visit(HirChildMut::Expr(payload)),
        E::Open { source, body, .. } | E::PackageOpen { source, body, .. } => {
            visit(HirChildMut::Expr(source));
            visit(HirChildMut::Block(body));
        }
        E::OwnedObjectCall { owner, args, .. } => {
            visit(HirChildMut::Expr(owner));
            args.iter_mut()
                .for_each(|expr| visit(HirChildMut::Expr(expr)));
        }
        E::ArrayLiteral(values) | E::TupleLiteral(values) => values
            .iter_mut()
            .for_each(|expr| visit(HirChildMut::Expr(expr))),
        E::ArrayRepeat(value, _)
        | E::ObjectCoercion(value, _)
        | E::UnaryOp(_, value)
        | E::FieldAccess(value, _, _)
        | E::TupleIndex(value, _)
        | E::Ref(_, value)
        | E::Deref(value)
        | E::Cast(value, _) => visit(HirChildMut::Expr(value)),
        E::BinOp(_, left, right) | E::Assign(left, right) => {
            visit(HirChildMut::Expr(left));
            visit(HirChildMut::Expr(right));
        }
        E::Call(callee, args, _) | E::MethodCall(callee, _, args, _, _) => {
            visit(HirChildMut::Expr(callee));
            args.iter_mut()
                .for_each(|expr| visit(HirChildMut::Expr(expr)));
        }
        E::Try { expr, .. } => visit(HirChildMut::Expr(expr)),
        E::StructLiteral(_, _, fields) => fields
            .iter_mut()
            .for_each(|field| visit(HirChildMut::Expr(&mut field.value))),
        E::EnumVariant(_, _, args, _) | E::Intrinsic { args, .. } => args
            .iter_mut()
            .for_each(|expr| visit(HirChildMut::Expr(expr))),
        E::If {
            condition,
            then_branch,
            else_branch,
        } => {
            visit(HirChildMut::Expr(condition));
            visit(HirChildMut::Block(then_branch));
            if let Some(branch) = else_branch {
                visit(HirChildMut::Block(branch));
            }
        }
        E::Match { scrutinee, arms } => {
            visit(HirChildMut::Expr(scrutinee));
            for arm in arms {
                if let Some(guard) = &mut arm.guard {
                    visit(HirChildMut::Expr(guard));
                }
                visit(HirChildMut::Block(&mut arm.body));
            }
        }
        E::While { condition, body } => {
            visit(HirChildMut::Expr(condition));
            visit(HirChildMut::Block(body));
        }
        E::For { iter, body, .. } => {
            visit(HirChildMut::Expr(iter));
            visit(HirChildMut::Block(body));
        }
        E::Loop(body) | E::Block(body) | E::UnsafeBlock(body) | E::Lambda { body, .. } => {
            visit(HirChildMut::Block(body))
        }
        E::IntLiteral(_)
        | E::FloatLiteral(_)
        | E::BoolLiteral(_)
        | E::StringLiteral(_)
        | E::CharLiteral(_)
        | E::Unit
        | E::Var(_)
        | E::ResolvedVar(_) => {}
    }
}

fn visit_expr<P: HirPhase>(expr: &HirExprFor<P>, visit: &mut impl FnMut(&HirExprFor<P>)) {
    visit(expr);
    visit_expr_children(expr, &mut |child| match child {
        HirChild::Expr(expr) => visit_expr(expr, visit),
        HirChild::Block(block) => visit_block(block, visit),
    });
}

//! Read-only traversal of accepted HIR bodies shared by the lint passes.

use crate::hir::{
    AcceptedHirBlock, AcceptedHirExpr, AcceptedHirFunction, HirExprKindFor, HirParam, HirPattern,
    HirStmtFor,
};
use crate::ids::HirLocalId;

pub(crate) trait HirVisitor {
    fn visit_type(&mut self, _ty: &crate::types::Type) {}
    /// Called before an expression's children; return `false` to skip them.
    fn enter_expr(&mut self, _expr: &AcceptedHirExpr) -> bool {
        true
    }

    /// A local introduced by a parameter, `let`, `for`, closure or pattern.
    fn bind_local(&mut self, _name: &str, _local: HirLocalId) {}

    fn visit_pattern(&mut self, _pattern: &HirPattern) {}
}

pub(crate) fn walk_function(visitor: &mut impl HirVisitor, function: &AcceptedHirFunction) {
    walk_params(visitor, &function.params);
    walk_block(visitor, &function.body);
}

fn walk_params(visitor: &mut impl HirVisitor, params: &[HirParam]) {
    for param in params {
        visitor.bind_local(&param.name, param.local_id);
    }
}

pub(crate) fn walk_block(visitor: &mut impl HirVisitor, block: &AcceptedHirBlock) {
    visitor.visit_type(&block.ty);
    for stmt in &block.stmts {
        match stmt {
            HirStmtFor::Let {
                name,
                local_id,
                value,
                ty,
                ..
            } => {
                visitor.visit_type(ty);
                walk_expr(visitor, value);
                visitor.bind_local(name, *local_id);
            }
            HirStmtFor::Expr(value)
            | HirStmtFor::Return(Some(value))
            | HirStmtFor::Break(Some(value)) => walk_expr(visitor, value),
            HirStmtFor::Return(None) | HirStmtFor::Break(None) | HirStmtFor::Continue => {}
        }
    }
}

pub(crate) fn walk_pattern(visitor: &mut impl HirVisitor, pattern: &HirPattern) {
    visitor.visit_pattern(pattern);
    match pattern {
        HirPattern::Binding { name, local_id, .. } => visitor.bind_local(name, *local_id),
        HirPattern::Tuple(patterns)
        | HirPattern::Or(patterns)
        | HirPattern::Enum(_, _, _, patterns) => patterns
            .iter()
            .for_each(|pattern| walk_pattern(visitor, pattern)),
        HirPattern::Struct(_, _, _, fields) => fields
            .iter()
            .for_each(|field| walk_pattern(visitor, &field.pattern)),
        HirPattern::Wildcard | HirPattern::Literal(_) => {}
    }
}

pub(crate) fn walk_expr(visitor: &mut impl HirVisitor, expr: &AcceptedHirExpr) {
    if !visitor.enter_expr(expr) {
        return;
    }
    match &expr.kind {
        HirExprKindFor::ArrayLiteral(values)
        | HirExprKindFor::TupleLiteral(values)
        | HirExprKindFor::EnumVariant(_, _, values, _)
        | HirExprKindFor::Intrinsic { args: values, .. } => {
            values.iter().for_each(|value| walk_expr(visitor, value))
        }
        HirExprKindFor::ArrayRepeat(value, _)
        | HirExprKindFor::UnaryOp(_, value)
        | HirExprKindFor::FieldAccess(value, _, _)
        | HirExprKindFor::TupleIndex(value, _)
        | HirExprKindFor::Ref(_, value)
        | HirExprKindFor::Deref(value)
        | HirExprKindFor::Cast(value, _)
        | HirExprKindFor::Try { expr: value, .. } => walk_expr(visitor, value),
        HirExprKindFor::BinOp(_, left, right) | HirExprKindFor::Assign(left, right) => {
            walk_expr(visitor, left);
            walk_expr(visitor, right);
        }
        HirExprKindFor::Call(callee, args, _)
        | HirExprKindFor::MethodCall(callee, _, args, _, _) => {
            walk_expr(visitor, callee);
            args.iter().for_each(|arg| walk_expr(visitor, arg));
        }
        HirExprKindFor::StructLiteral(_, _, fields) => fields
            .iter()
            .for_each(|field| walk_expr(visitor, &field.value)),
        HirExprKindFor::If {
            condition,
            then_branch,
            else_branch,
        } => {
            walk_expr(visitor, condition);
            walk_block(visitor, then_branch);
            if let Some(else_branch) = else_branch {
                walk_block(visitor, else_branch);
            }
        }
        HirExprKindFor::Match { scrutinee, arms } => {
            walk_expr(visitor, scrutinee);
            for arm in arms {
                walk_pattern(visitor, &arm.pattern);
                if let Some(guard) = &arm.guard {
                    walk_expr(visitor, guard);
                }
                walk_block(visitor, &arm.body);
            }
        }
        HirExprKindFor::While { condition, body } => {
            walk_expr(visitor, condition);
            walk_block(visitor, body);
        }
        HirExprKindFor::For {
            var,
            local_id,
            iter,
            body,
        } => {
            walk_expr(visitor, iter);
            visitor.bind_local(var, *local_id);
            walk_block(visitor, body);
        }
        HirExprKindFor::Loop(body)
        | HirExprKindFor::Block(body)
        | HirExprKindFor::UnsafeBlock(body) => walk_block(visitor, body),
        HirExprKindFor::Lambda { params, body, .. } => {
            walk_params(visitor, params);
            walk_block(visitor, body);
        }
        HirExprKindFor::IntLiteral(_)
        | HirExprKindFor::ConstParam(_)
        | HirExprKindFor::FloatLiteral(_)
        | HirExprKindFor::BoolLiteral(_)
        | HirExprKindFor::StringLiteral(_)
        | HirExprKindFor::CharLiteral(_)
        | HirExprKindFor::Unit
        | HirExprKindFor::Var(_)
        | HirExprKindFor::ResolvedVar(_) => {}
    }
}

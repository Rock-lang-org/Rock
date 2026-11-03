use crate::hir::{HirBlock, HirExpr, HirExprKind, HirFunction, HirStmt};
use crate::ids::HirLocalId;
use crate::types::Type;

use super::MethodAuthorityContext;

impl MethodAuthorityContext<'_> {
    pub(super) fn collect_local_bindings(&mut self, function: &HirFunction) {
        for param in &function.params {
            self.record_local_binding(param.local_id, &param.name, param.ty.clone(), param.mutable);
        }
        self.collect_local_bindings_block(&function.body);
    }

    fn record_local_binding(&mut self, local_id: HirLocalId, name: &str, ty: Type, mutable: bool) {
        self.next_local_id = self.next_local_id.max(local_id.0.saturating_add(1));
        if mutable {
            self.mutable_locals.insert(local_id);
        }
        self.local_bindings
            .insert(local_id, (name.to_string(), ty, mutable));
    }

    fn collect_local_bindings_block(&mut self, block: &HirBlock) {
        for stmt in &block.stmts {
            match stmt {
                HirStmt::Let {
                    name,
                    local_id,
                    ty,
                    value,
                    mutable,
                } => {
                    self.record_local_binding(*local_id, name, ty.clone(), *mutable);
                    self.collect_local_bindings_expr(value);
                }
                HirStmt::Expr(expr) | HirStmt::Return(Some(expr)) | HirStmt::Break(Some(expr)) => {
                    self.collect_local_bindings_expr(expr)
                }
                HirStmt::Return(None) | HirStmt::Break(None) | HirStmt::Continue => {}
            }
        }
    }

    fn collect_local_bindings_expr(&mut self, expr: &HirExpr) {
        match &expr.kind {
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.collect_local_bindings_expr(condition);
                self.collect_local_bindings_block(then_branch);
                if let Some(else_branch) = else_branch {
                    self.collect_local_bindings_block(else_branch);
                }
            }
            HirExprKind::Match { scrutinee, arms } => {
                self.collect_local_bindings_expr(scrutinee);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.collect_local_bindings_expr(guard);
                    }
                    self.collect_local_bindings_block(&arm.body);
                }
            }
            HirExprKind::While { condition, body } => {
                self.collect_local_bindings_expr(condition);
                self.collect_local_bindings_block(body);
            }
            HirExprKind::For { iter, body, .. } => {
                self.collect_local_bindings_expr(iter);
                self.collect_local_bindings_block(body);
            }
            HirExprKind::Block(body) | HirExprKind::Loop(body) | HirExprKind::UnsafeBlock(body) => {
                self.collect_local_bindings_block(body)
            }
            HirExprKind::Lambda { params, body, .. } => {
                for param in params {
                    self.record_local_binding(
                        param.local_id,
                        &param.name,
                        param.ty.clone(),
                        param.mutable,
                    );
                }
                self.collect_local_bindings_block(body);
            }
            HirExprKind::Call(callee, args, _) | HirExprKind::MethodCall(callee, _, args, _, _) => {
                self.collect_local_bindings_expr(callee);
                for arg in args {
                    self.collect_local_bindings_expr(arg);
                }
            }
            HirExprKind::Try { expr, .. }
            | HirExprKind::FieldAccess(expr, _, _)
            | HirExprKind::Deref(expr)
            | HirExprKind::Ref(_, expr)
            | HirExprKind::Cast(expr, _)
            | HirExprKind::TupleIndex(expr, _) => self.collect_local_bindings_expr(expr),
            HirExprKind::Assign(left, right) | HirExprKind::BinOp(_, left, right) => {
                self.collect_local_bindings_expr(left);
                self.collect_local_bindings_expr(right);
            }
            HirExprKind::UnaryOp(_, inner) | HirExprKind::ArrayRepeat(inner, _) => {
                self.collect_local_bindings_expr(inner)
            }
            HirExprKind::Intrinsic { args, .. }
            | HirExprKind::TupleLiteral(args)
            | HirExprKind::ArrayLiteral(args)
            | HirExprKind::EnumVariant(_, _, args, _) => {
                for arg in args {
                    self.collect_local_bindings_expr(arg);
                }
            }
            HirExprKind::StructLiteral(_, _, fields) => {
                for field in fields {
                    self.collect_local_bindings_expr(&field.value);
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
}

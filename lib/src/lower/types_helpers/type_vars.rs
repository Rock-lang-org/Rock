//! Type variable collection and replacement utilities

use crate::hir::*;

use crate::lower::Lowerer;

fn resolve_pattern_types(pattern: &mut HirPattern, lowerer: &Lowerer) {
    match pattern {
        HirPattern::Tuple(patterns) | HirPattern::Or(patterns) => {
            for pattern in patterns {
                resolve_pattern_types(pattern, lowerer);
            }
        }
        HirPattern::Struct(_, _, type_args, field_patterns) => {
            for ty in type_args {
                *ty = lowerer.engine.resolve(ty);
            }
            for field in field_patterns {
                resolve_pattern_types(&mut field.pattern, lowerer);
            }
        }
        HirPattern::Enum(_, _, _, patterns) => {
            for pattern in patterns {
                resolve_pattern_types(pattern, lowerer);
            }
        }
        HirPattern::Wildcard | HirPattern::Binding { .. } | HirPattern::Literal(_) => {}
    }
}

impl Lowerer {
    /// Resolve all TypeVars in a HirFunction using the inference engine.
    /// This replaces unified TypeVars with their resolved concrete types.
    pub(crate) fn resolve_all_types_in_function(&mut self, func: &mut HirFunction) {
        for param in &mut func.params {
            param.ty = self.engine.resolve(&param.ty);
        }
        func.ret_type = self.engine.resolve(&func.ret_type);
        self.resolve_all_types_in_block(&mut func.body);
    }

    pub(crate) fn resolve_all_types_in_block(&mut self, block: &mut HirBlock) {
        block.ty = self.engine.resolve(&block.ty);
        for stmt in &mut block.stmts {
            self.resolve_all_types_in_stmt(stmt);
        }
        if let Some(stmt) = block.stmts.last() {
            match stmt {
                HirStmt::Expr(expr) | HirStmt::Return(Some(expr)) | HirStmt::Break(Some(expr)) => {
                    block.ty = expr.ty.clone();
                }
                HirStmt::Let { .. }
                | HirStmt::Return(None)
                | HirStmt::Break(None)
                | HirStmt::Continue => {}
            }
        }
    }

    pub(crate) fn resolve_all_types_in_stmt(&mut self, stmt: &mut HirStmt) {
        match stmt {
            HirStmt::Let { ty, value, .. } => {
                *ty = self.engine.resolve(ty);
                self.resolve_all_types_in_expr(value);
            }
            HirStmt::Expr(expr) => {
                self.resolve_all_types_in_expr(expr);
            }
            HirStmt::Return(Some(expr)) => {
                self.resolve_all_types_in_expr(expr);
            }
            HirStmt::Break(Some(expr)) => {
                self.resolve_all_types_in_expr(expr);
            }
            _ => {}
        }
    }

    pub(crate) fn resolve_all_types_in_expr(&mut self, expr: &mut HirExpr) {
        expr.ty = self.engine.resolve(&expr.ty);
        match &mut expr.kind {
            HirExprKind::Call(func_expr, args, target) => {
                self.resolve_all_types_in_expr(func_expr);
                for arg in args.iter_mut() {
                    self.resolve_all_types_in_expr(arg);
                }
                if let Some(HirCallTarget::StaticMethod(target)) = target {
                    target.owner_ty = self.engine.resolve(&target.owner_ty);
                    target
                        .method
                        .for_each_type_mut(|ty| *ty = self.engine.resolve(ty));
                }
            }
            HirExprKind::MethodCall(recv, _, args, _, target) => {
                self.resolve_all_types_in_expr(recv);
                for arg in args {
                    self.resolve_all_types_in_expr(arg);
                }
                if let Some(target) = target {
                    target.for_each_type_mut(|ty| *ty = self.engine.resolve(ty));
                }
            }
            HirExprKind::Try {
                expr,
                branch_method,
                from_residual_target,
                output_ty,
                residual_ty,
                return_ty,
                ..
            } => {
                self.resolve_all_types_in_expr(expr);
                if let Some(target) = branch_method {
                    target.for_each_type_mut(|ty| *ty = self.engine.resolve(ty));
                }
                if let Some(HirCallTarget::StaticMethod(target)) = from_residual_target {
                    target.owner_ty = self.engine.resolve(&target.owner_ty);
                    target
                        .method
                        .for_each_type_mut(|ty| *ty = self.engine.resolve(ty));
                }
                *output_ty = self.engine.resolve(output_ty);
                *residual_ty = self.engine.resolve(residual_ty);
                *return_ty = self.engine.resolve(return_ty);
            }
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.resolve_all_types_in_expr(condition);
                self.resolve_all_types_in_block(then_branch);
                if let Some(eb) = else_branch {
                    self.resolve_all_types_in_block(eb);
                }
            }
            HirExprKind::Block(block) | HirExprKind::UnsafeBlock(block) => {
                self.resolve_all_types_in_block(block);
            }
            HirExprKind::Lambda {
                params,
                body,
                captures,
            } => {
                for p in params {
                    p.ty = self.engine.resolve(&p.ty);
                }
                for capture in captures {
                    capture.ty = self.engine.resolve(&capture.ty);
                }
                self.resolve_all_types_in_block(body);
            }
            HirExprKind::While { condition, body } => {
                self.resolve_all_types_in_expr(condition);
                self.resolve_all_types_in_block(body);
            }
            HirExprKind::For { iter, body, .. } => {
                self.resolve_all_types_in_expr(iter);
                self.resolve_all_types_in_block(body);
            }
            HirExprKind::Loop(block) => {
                self.resolve_all_types_in_block(block);
            }
            HirExprKind::Assign(lhs, rhs) => {
                self.resolve_all_types_in_expr(lhs);
                self.resolve_all_types_in_expr(rhs);
            }
            HirExprKind::Var(_)
            | HirExprKind::ResolvedVar(_)
            | HirExprKind::IntLiteral(_) | HirExprKind::ConstParam(_)
            | HirExprKind::FloatLiteral(_)
            | HirExprKind::BoolLiteral(_)
            | HirExprKind::StringLiteral(_)
            | HirExprKind::Unit
            | HirExprKind::CharLiteral(_) => {}
            HirExprKind::BinOp(_, lhs, rhs) => {
                self.resolve_all_types_in_expr(lhs);
                self.resolve_all_types_in_expr(rhs);
            }
            HirExprKind::UnaryOp(_, inner) => {
                self.resolve_all_types_in_expr(inner);
            }
            HirExprKind::Intrinsic { args, .. } => {
                for arg in args {
                    self.resolve_all_types_in_expr(arg);
                }
            }
            HirExprKind::FieldAccess(base, _, _) => {
                self.resolve_all_types_in_expr(base);
            }
            HirExprKind::ArrayLiteral(elems) => {
                for e in elems {
                    self.resolve_all_types_in_expr(e);
                }
            }
            HirExprKind::ArrayRepeat(value, _) => self.resolve_all_types_in_expr(value),
            HirExprKind::StructLiteral(_, _, fields) => {
                for field in fields {
                    self.resolve_all_types_in_expr(&mut field.value);
                }
            }
            HirExprKind::TupleIndex(base, _) => {
                self.resolve_all_types_in_expr(base);
            }
            HirExprKind::TupleLiteral(elems) => {
                for e in elems {
                    self.resolve_all_types_in_expr(e);
                }
            }
            HirExprKind::EnumVariant(_, _, args, _) => {
                for arg in args {
                    self.resolve_all_types_in_expr(arg);
                }
            }
            HirExprKind::Match { scrutinee, arms } => {
                self.resolve_all_types_in_expr(scrutinee);
                for arm in arms {
                    resolve_pattern_types(&mut arm.pattern, self);
                    if let Some(guard) = &mut arm.guard {
                        self.resolve_all_types_in_expr(guard);
                    }
                    self.resolve_all_types_in_block(&mut arm.body);
                }
            }
            HirExprKind::Ref(_, inner) | HirExprKind::Deref(inner) => {
                self.resolve_all_types_in_expr(inner);
            }
            HirExprKind::Cast(inner, ty) => {
                self.resolve_all_types_in_expr(inner);
                *ty = self.engine.resolve(ty);
            }
        }
    }
}

#[cfg(test)]
mod tests;

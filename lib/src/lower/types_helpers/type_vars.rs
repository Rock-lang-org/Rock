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
            HirExprKind::Open { source, binding, body } => {
                self.resolve_all_types_in_expr(source);
                binding.visit_types_mut(&mut |ty| *ty = self.engine.resolve(ty));
                self.resolve_all_types_in_block(body);
            }
            HirExprKind::OwnedObjectCall { owner, args, call } => {
                self.resolve_all_types_in_expr(owner);
                for arg in args {
                    self.resolve_all_types_in_expr(arg);
                }
                call.visit_types_mut(&mut |ty| *ty = self.engine.resolve(ty));
            }
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
            | HirExprKind::IntLiteral(_)
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
            HirExprKind::ObjectCoercion(inner, coercion) => {
                self.resolve_all_types_in_expr(inner);
                coercion.visit_types_mut(&mut |ty| *ty = self.engine.resolve(ty));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::hir::{
        HirBlock, HirCallTarget, HirExpr, HirExprKind, HirMatchArm, HirMethodCallTarget,
        HirPattern, HirStaticMethodTarget, HirStmt, HirTypeBinding,
    };
    use crate::ids::{CrateId, DefId, LocalDefId};
    use crate::lower::Lowerer;
    use crate::types::{GenericParamId, Type};

    #[test]
    fn resolve_all_types_in_expr_resolves_match_arm_guards() {
        let mut lowerer = Lowerer::new_for_test();
        let guard_ty = lowerer.engine.fresh_type_var();
        lowerer.engine.unify(&guard_ty, &Type::Bool).unwrap();
        let mut expr = HirExpr {
            kind: HirExprKind::Match {
                scrutinee: Box::new(HirExpr {
                    kind: HirExprKind::IntLiteral(0),
                    ty: Type::I64,
                    span: crate::lexer::Span::test(),
                }),
                arms: vec![HirMatchArm {
                    pattern: HirPattern::Wildcard,
                    guard: Some(HirExpr {
                        kind: HirExprKind::BoolLiteral(true),
                        ty: guard_ty,
                        span: crate::lexer::Span::test(),
                    }),
                    body: HirBlock {
                        stmts: vec![HirStmt::Expr(HirExpr {
                            kind: HirExprKind::IntLiteral(1),
                            ty: Type::I64,
                            span: crate::lexer::Span::test(),
                        })],
                        ty: Type::I64,
                    },
                }],
            },
            ty: Type::I64,
            span: crate::lexer::Span::test(),
        };

        lowerer.resolve_all_types_in_expr(&mut expr);

        let HirExprKind::Match { arms, .. } = &expr.kind else {
            panic!("expected match expression");
        };
        assert_eq!(arms[0].guard.as_ref().unwrap().ty, Type::Bool);
    }

    #[test]
    fn resolve_all_types_in_expr_resolves_method_target_trait_args() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_arg = lowerer.engine.fresh_type_var();
        lowerer.engine.unify(&trait_arg, &Type::I64).unwrap();
        let mut expr = HirExpr {
            kind: HirExprKind::MethodCall(
                Box::new(HirExpr {
                    kind: HirExprKind::Var("value".to_string()),
                    ty: Type::I64,
                    span: crate::lexer::Span::test(),
                }),
                "method".to_string(),
                Vec::new(),
                None,
                Some(HirMethodCallTarget::trait_method(
                    DefId::new(CrateId(0), LocalDefId(1)),
                    DefId::new(CrateId(0), LocalDefId(2)),
                    vec![trait_arg],
                    crate::hir::HirTraitDispatchKind::TraitBound,
                )),
            ),
            ty: Type::I64,
            span: crate::lexer::Span::test(),
        };

        lowerer.resolve_all_types_in_expr(&mut expr);

        let HirExprKind::MethodCall(_, _, _, _, Some(target)) = &expr.kind else {
            panic!("expected method call target");
        };
        assert_eq!(target.trait_args(), &[Type::I64]);
    }

    #[test]
    fn resolve_all_types_in_expr_resolves_static_method_substitution() {
        let mut lowerer = Lowerer::new_for_test();
        let method_id = DefId::new(CrateId(0), LocalDefId(3));
        let method_param = GenericParamId {
            owner: method_id,
            index: 0,
        };
        let method_ty = lowerer.engine.fresh_type_var();
        lowerer.engine.unify(&method_ty, &Type::I64).unwrap();
        let mut method = HirMethodCallTarget::trait_method(
            DefId::new(CrateId(0), LocalDefId(4)),
            method_id,
            Vec::new(),
            crate::hir::HirTraitDispatchKind::TraitBound,
        );
        method.method_substitution = vec![HirTypeBinding {
            param: method_param,
            ty: method_ty,
        }];
        let mut expr = HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::Var("generic_static".to_string()),
                    ty: Type::function(Vec::new(), Type::I64),
                    span: crate::lexer::Span::test(),
                }),
                Vec::new(),
                Some(HirCallTarget::StaticMethod(HirStaticMethodTarget {
                    owner_ty: Type::I64,
                    method,
                })),
            ),
            ty: Type::I64,
            span: crate::lexer::Span::test(),
        };

        lowerer.resolve_all_types_in_expr(&mut expr);

        let HirExprKind::Call(_, _, Some(HirCallTarget::StaticMethod(target))) = &expr.kind else {
            panic!("expected static method target");
        };
        assert_eq!(target.method.method_substitution.len(), 1);
        assert_eq!(target.method.method_substitution[0].param, method_param);
        assert_eq!(target.method.method_substitution[0].ty, Type::I64);
    }
}

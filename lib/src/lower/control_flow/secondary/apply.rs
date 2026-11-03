//! Secondary expression application (calls, field access, indexing).

use crate::ast;
use crate::hir::*;
use crate::lexer::Span;
use crate::lower::expression::ExprUse;
use crate::lower::Lowerer;
use crate::types::Type;

impl Lowerer {
    pub(super) fn fold_unary_calls(
        &mut self,
        mut expr: HirExpr,
        args: Vec<HirExpr>,
        span: Span,
    ) -> HirExpr {
        for arg in args {
            let resolved_callee_ty = self.resolve_projection_type(&self.engine.resolve(&expr.ty));
            if let Type::Reference { inner, .. } = resolved_callee_ty {
                if matches!(
                    self.resolve_projection_type(&self.engine.resolve(&inner)),
                    Type::Function { .. }
                ) {
                    let expr_span = expr.span.clone();
                    expr = HirExpr {
                        ty: *inner,
                        kind: HirExprKind::Deref(Box::new(expr)),
                        span: expr_span,
                    };
                }
            }
            let ret_ty = self.engine.fresh_type_var_at(span.clone());
            let safety = match self.resolve_projection_type(&self.engine.resolve(&expr.ty)) {
                Type::Function { safety, .. } => safety,
                _ => crate::types::FunctionSafety::Safe,
            };
            let expected_fn_ty =
                Type::function_with_safety(vec![arg.ty.clone()], ret_ty.clone(), safety);
            let _ = self.engine.unify(&expr.ty, &expected_fn_ty);
            let resolved_ret_ty = self.engine.resolve(&ret_ty);
            self.report_unsafe_function_value_call_from_type(&expr);

            let target = self.call_target_for_callee(&expr);
            expr = HirExpr {
                ty: resolved_ret_ty,
                kind: HirExprKind::Call(Box::new(expr), vec![arg], target),
                span: span.clone(),
            };
        }

        expr
    }

    pub(super) fn index_display_type(&self, ty: &Type) -> String {
        self.display_type(ty)
    }

    pub(crate) fn apply_secondary(
        &mut self,
        expr: HirExpr,
        secondary: &ast::SecondaryExpr,
        use_kind: ExprUse,
        method_as_value: bool,
    ) -> HirExpr {
        let span = expr.span.clone();
        match secondary {
            ast::SecondaryExpr::Arguments(args) => self.apply_arguments_secondary(expr, args, span),
            ast::SecondaryExpr::Dot(ident_or_num) => {
                self.apply_dot_secondary(expr, ident_or_num, span, method_as_value)
            }
            ast::SecondaryExpr::Indice(index_expr) => {
                self.apply_indice_secondary(expr, index_expr, span, use_kind)
            }
            ast::SecondaryExpr::Interogation => self.lower_try_interogation(expr, span),
        }
    }
}

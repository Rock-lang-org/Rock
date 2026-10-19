use crate::ast;
use crate::hir::*;
use crate::lower::Lowerer;
use crate::types::Type;

impl Lowerer {
    pub(crate) fn lower_loop(&mut self, loop_expr: &ast::Loop) -> HirExpr {
        match loop_expr {
            ast::Loop::While(cond, body, _) => {
                let condition = self.lower_expression(&cond.expression);
                let span = condition.span.clone();
                let _ = self.engine.unify(&condition.ty, &Type::Bool);

                self.push_scope();
                self.fold_loop_stack.push(None);
                let body = self.lower_block(body);
                self.fold_loop_stack.pop();
                self.pop_scope();

                HirExpr {
                    ty: Type::Unit,
                    kind: HirExprKind::While {
                        condition: Box::new(condition),
                        body,
                    },
                    span: span.clone(),
                }
            }
            ast::Loop::For(pattern, iter_expr, body, _) => {
                let iter = self.lower_expression(iter_expr);
                let span = iter.span.clone();
                let (var_name, _) = self.extract_pattern_binding(pattern);

                let range_variant = match &iter.kind {
                    HirExprKind::EnumVariant(_, _, _, Some(location)) => self
                        .language_items
                        .range
                        .as_ref()
                        .filter(|items| items.enum_id == location.owner)
                        .map(|items| {
                            (
                                location.variant_id == items.exclusive_variant_id,
                                location.variant_id == items.inclusive_variant_id,
                            )
                        }),
                    _ => None,
                };
                let elem_ty = if let Some((exclusive, inclusive)) = range_variant {
                    if !exclusive && !inclusive {
                        self.diagnostics.push_with_span(
                            "for loops currently require a bounded range".to_string(),
                            span.clone(),
                        );
                        return self.error_expression_at(span);
                    }
                    if self.language_items.fold.is_some() {
                        return self.lower_fold_loop(pattern, iter, body, span);
                    }
                    Type::I64
                } else {
                    match self.engine.resolve(&iter.ty) {
                        Type::Array(element, _) | Type::Slice(element) => *element,
                        _ => {
                            return self.lower_fold_loop(pattern, iter, body, span);
                        }
                    }
                };

                self.push_scope();
                let local_id = self.fresh_local_id();
                if let (Some(owner), Some(span)) = (
                    self.current_body_def_id(),
                    Self::pattern_binding_span(pattern),
                ) {
                    self.source_map.insert_local_in_scope(
                        owner,
                        local_id,
                        span,
                        self.source_scope_stack.last().copied(),
                    );
                }
                self.scope
                    .define_local(var_name.clone(), elem_ty, false, local_id);
                self.fold_loop_stack.push(None);
                let body = self.lower_block(body);
                self.fold_loop_stack.pop();
                self.pop_scope();

                HirExpr {
                    ty: Type::Unit,
                    kind: HirExprKind::For {
                        var: var_name,
                        local_id,
                        iter: Box::new(iter),
                        body,
                    },
                    span,
                }
            }
            ast::Loop::Loop(body, span) => {
                self.push_scope();
                self.fold_loop_stack.push(None);
                let body = self.lower_block(body);
                self.fold_loop_stack.pop();
                self.pop_scope();
                HirExpr {
                    ty: Type::Unit,
                    kind: HirExprKind::Loop(body),
                    span: span.clone(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ast::{
        Block, Expression, Ident, IdentOrType, IdentifierPath, Literal, LiteralKind, Operand,
        Pattern, PatternKind, PrimaryExpr, Statement, UnaryExpr,
    };
    use crate::hir::{HirExprKind, HirStmt, HirVarRef, HirVarTarget};
    use crate::lexer::Span;
    use crate::lower::Lowerer;

    fn ident(name: &str) -> Ident {
        Ident {
            name: name.to_string(),
            span: Span::test(),
        }
    }

    fn binding_pattern(name: &str) -> Pattern {
        Pattern {
            binding: Some(ident(name)),
            kind: PatternKind::Ident(crate::ast::IdentPattern {
                name: ident(name),
                mut_: false,
            }),
        }
    }

    fn array_expr(start: u64, end: u64) -> Expression {
        let endpoint = |value| {
            Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                operand: Operand::Literal(Literal {
                    kind: LiteralKind::Number(value),
                    span: Span::test(),
                }),
                secondaries: None,
                type_annotation: None,
            }))
        };
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Literal(Literal {
                kind: LiteralKind::Array(crate::ast::Array {
                    elements: vec![endpoint(start), endpoint(end)],
                }),
                span: Span::test(),
            }),
            secondaries: None,
            type_annotation: None,
        }))
    }

    fn var_expr(name: &str) -> Expression {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Ident(IdentifierPath {
                path: vec![IdentOrType::Ident(ident(name))],
            }),
            secondaries: None,
            type_annotation: None,
        }))
    }

    #[test]
    fn for_loop_variable_read_uses_loop_local_id() {
        let mut lowerer = Lowerer::new_for_test();
        let loop_expr = crate::ast::Loop::For(
            binding_pattern("item"),
            array_expr(0, 2),
            Block {
                statements: vec![Statement::Expression(var_expr("item"))],
            },
            Span::test(),
        );

        let hir = lowerer.with_test_body_context(|lowerer| lowerer.lower_loop(&loop_expr));

        let HirExprKind::For { local_id, body, .. } = hir.kind else {
            panic!("expected for loop");
        };
        let HirStmt::Expr(expr) = &body.stmts[0] else {
            panic!("expected loop body expression");
        };
        match &expr.kind {
            HirExprKind::ResolvedVar(HirVarRef {
                target: HirVarTarget::Local(id),
                name,
            }) => {
                assert_eq!(name, "item");
                assert_eq!(*id, local_id);
            }
            other => panic!("expected loop local read, got {other:?}"),
        }
    }

    #[test]
    fn loop_control_statements_do_not_require_a_value_span() {
        let mut lowerer = Lowerer::new_for_test();
        for statement in [
            Statement::Continue(None),
            Statement::Return(None),
            Statement::Break(None),
        ] {
            let loop_expr = crate::ast::Loop::Loop(
                Block {
                    statements: vec![statement],
                },
                Span::test(),
            );
            let hir = lowerer.with_test_body_context(|lowerer| lowerer.lower_loop(&loop_expr));
            assert!(matches!(hir.kind, HirExprKind::Loop(_)));
            assert_eq!(hir.span, Span::test());
        }
    }
}

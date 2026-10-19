//! Statement lowering methods for Lowerer

use crate::ast;
use crate::hir::*;
use crate::lower::expression::ExprUse;
use crate::types::Type;

use crate::lower::Lowerer;

impl Lowerer {
    // ============================================================
    // Statement Lowering Methods
    // ============================================================

    pub(crate) fn lower_block(&mut self, block: &ast::Block) -> HirBlock {
        let mut stmts = Vec::new();
        let mut last_ty = Type::Unit;

        for (i, stmt) in block.statements.iter().enumerate() {
            let is_last = i == block.statements.len() - 1;
            // Check if this is a tuple destructuring assignment
            if let ast::Statement::Assignment(assign) = stmt {
                if let ast::AssignmentLHS::Pattern { pattern, .. } = &assign.lhs {
                    if let ast::PatternKind::Tuple(sub_patterns) = &pattern.kind {
                        let expanded = self.lower_tuple_destructuring(sub_patterns, &assign.rhs);
                        for s in &expanded {
                            last_ty = self.stmt_type(s);
                        }
                        stmts.extend(expanded);
                        continue;
                    }
                }
            }
            let hir_stmt = self.lower_statement(stmt, is_last);
            last_ty = self.stmt_type(&hir_stmt);
            stmts.push(hir_stmt);
        }

        HirBlock { stmts, ty: last_ty }
    }

    pub(crate) fn lower_lambda_body(&mut self, lambda: &ast::LambdaDecl) -> HirBlock {
        let mut body = self.lower_block(&lambda.body);
        if matches!(lambda.arrow_kind, ast::LambdaArrowKind::Unit) {
            let span = lambda.span.clone();
            body.stmts.push(HirStmt::Expr(HirExpr {
                ty: Type::Unit,
                kind: HirExprKind::Unit,
                span,
            }));
            body.ty = Type::Unit;
        }
        body
    }

    pub(crate) fn stmt_type(&self, stmt: &HirStmt) -> Type {
        match stmt {
            HirStmt::Let { .. } => Type::Unit,
            HirStmt::Expr(e) => e.ty.clone(),
            HirStmt::Return(Some(e)) => e.ty.clone(),
            HirStmt::Return(None) => Type::Unit,
            HirStmt::Break(_) => Type::Never,
            HirStmt::Continue => Type::Never,
        }
    }

    pub(crate) fn lower_statement(&mut self, stmt: &ast::Statement, _is_last: bool) -> HirStmt {
        match stmt {
            ast::Statement::Assignment(assign) => self.lower_assignment(assign),
            ast::Statement::Expression(expr) => {
                let hir_expr = self.lower_expression(expr);
                HirStmt::Expr(hir_expr)
            }
            ast::Statement::Return(Some(expr)) => {
                let hir_expr = self.lower_expression(expr);
                HirStmt::Return(Some(self.wrap_fold_return(hir_expr)))
            }
            ast::Statement::Return(None) => {
                if let Some(context) = self.fold_loop_stack.iter().flatten().last() {
                    let unit = context.unit();
                    HirStmt::Return(Some(self.wrap_fold_return(unit)))
                } else {
                    HirStmt::Return(None)
                }
            }
            ast::Statement::Break(Some(expr)) => {
                let hir_expr = self.lower_expression(expr);
                if let Some(Some(context)) = self.fold_loop_stack.last() {
                    HirStmt::Expr(HirExpr {
                        ty: Type::Never,
                        span: hir_expr.span.clone(),
                        kind: HirExprKind::Block(HirBlock {
                            ty: Type::Never,
                            stmts: vec![
                                HirStmt::Expr(hir_expr),
                                HirStmt::Return(Some(context.stop())),
                            ],
                        }),
                    })
                } else {
                    HirStmt::Break(Some(hir_expr))
                }
            }
            ast::Statement::Break(None) => match self.fold_loop_stack.last() {
                Some(Some(context)) => HirStmt::Return(Some(context.stop())),
                _ => HirStmt::Break(None),
            },
            ast::Statement::Continue(_) => match self.fold_loop_stack.last() {
                Some(Some(context)) => HirStmt::Return(Some(context.next())),
                _ => HirStmt::Continue,
            },
        }
    }

    pub(crate) fn lower_assignment(&mut self, assign: &ast::Assignment) -> HirStmt {
        let mut rhs = self.lower_expression(&assign.rhs);

        match &assign.lhs {
            ast::AssignmentLHS::Pattern {
                pattern,
                type_annotation,
            } => {
                let (name, mutable) = self.extract_pattern_binding(pattern);

                // Check if variable already exists in scope (reassignment vs new binding)
                if type_annotation.is_none() && self.scope.lookup(&name).is_some() {
                    // Reassignment to existing variable
                    let binding = self.scope.lookup(&name).cloned();
                    let lhs_ty = binding
                        .as_ref()
                        .map(|binding| binding.ty.clone())
                        .unwrap_or_else(|| rhs.ty.clone());
                    if let Err(e) = self.engine.unify(&rhs.ty, &lhs_ty) {
                        let span = rhs.span.clone();
                        self.diagnostics.push_type_with_span(
                            format!("Assignment type mismatch: {}", e.render(&self.engine)),
                            span,
                        );
                    }
                    let operation_span = rhs.span.clone();
                    let lhs_kind = binding
                        .and_then(|binding| binding.local_id)
                        .map(|local_id| {
                            HirExprKind::ResolvedVar(HirVarRef {
                                name: name.clone(),
                                target: HirVarTarget::Local(local_id),
                            })
                        })
                        .unwrap_or_else(|| HirExprKind::Var(name));
                    let lhs_expr = HirExpr {
                        ty: lhs_ty,
                        kind: lhs_kind,
                        span: operation_span.clone(),
                    };
                    return HirStmt::Expr(HirExpr {
                        ty: Type::Unit,
                        kind: HirExprKind::Assign(Box::new(lhs_expr), Box::new(rhs)),
                        span: operation_span,
                    });
                }

                let mut ty = rhs.ty.clone();

                if let Some(ann) = type_annotation {
                    self.source_map.record_type_annotation(ann.span());
                    let ann_ty = self.lower_parse_type(ann);
                    if let Err(e) = self.engine.unify(&ty, &ann_ty) {
                        self.diagnostics.push_type_with_span(
                            format!(
                                "Type annotation mismatch for '{}': {}",
                                name,
                                e.render(&self.engine)
                            ),
                            ann.span(),
                        );
                    }
                    ty = ann_ty;
                    self.resolve_all_types_in_expr(&mut rhs);
                }

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
                    .define_local(name.clone(), ty.clone(), mutable, local_id);

                HirStmt::Let {
                    name,
                    local_id,
                    ty,
                    value: rhs,
                    mutable,
                }
            }
            ast::AssignmentLHS::Expression(unary_expr) => {
                // This is a reassignment (e.g., x = 5 or x.field = 5)
                let lhs_expr = self.lower_unary_expr_with_use(unary_expr, ExprUse::AssignmentPlace);
                // For *ptr = val, unify the pointer's element type with the value type.
                // This is critical for generic impls: e.g. in `impl Vec T`, the deref
                // `*(~PtrAdd new_ptr, self.len)` has type `Generic("T")`, and after
                // resolve_all_types_in_function the elem param becomes `Generic("T")`,
                // enabling `substitute_generics({"T"→TypeVar(K)})` at call sites.
                if let HirExprKind::Deref(_) = &lhs_expr.kind {
                    if let Err(e) = self.engine.unify(&rhs.ty, &lhs_expr.ty) {
                        self.diagnostics.push_type_with_span(
                            format!("Assignment type mismatch: {}", e.render(&self.engine)),
                            rhs.span.clone(),
                        );
                    }
                } else if let Err(e) = self.engine.unify(&rhs.ty, &lhs_expr.ty) {
                    self.diagnostics.push_type_with_span(
                        format!("Assignment type mismatch: {}", e.render(&self.engine)),
                        rhs.span.clone(),
                    );
                }
                let operation_span = rhs.span.clone();
                HirStmt::Expr(HirExpr {
                    ty: Type::Unit,
                    kind: HirExprKind::Assign(Box::new(lhs_expr), Box::new(rhs)),
                    span: operation_span,
                })
            }
        }
    }

    pub(crate) fn lower_tuple_destructuring(
        &mut self,
        sub_patterns: &[ast::Pattern],
        rhs: &ast::Expression,
    ) -> Vec<HirStmt> {
        let mut stmts = Vec::new();
        let rhs_expr = self.lower_expression(rhs);
        let rhs_span = rhs_expr.span.clone();

        // Create a temp variable for the tuple value
        let uid = self.tuple_temp_counter;
        self.tuple_temp_counter = self
            .tuple_temp_counter
            .checked_add(1)
            .expect("tuple destructuring temp counter exhausted");
        let tmp_name = format!("__tuple_tmp_{}", uid);
        let tmp_ty = rhs_expr.ty.clone();
        let tmp_local_id = self.fresh_local_id();
        self.scope
            .define_local(tmp_name.clone(), tmp_ty.clone(), false, tmp_local_id);
        stmts.push(HirStmt::Let {
            name: tmp_name.clone(),
            local_id: tmp_local_id,
            ty: tmp_ty.clone(),
            value: rhs_expr,
            mutable: false,
        });

        // Extract each element of the tuple
        for (idx, sub_pat) in sub_patterns.iter().enumerate() {
            let (name, mutable) = self.extract_pattern_binding(sub_pat);
            if name == "_" {
                continue; // Skip wildcard bindings
            }

            // Determine the element type from the tuple type
            let elem_ty = match &tmp_ty {
                Type::Tuple(types) if idx < types.len() => types[idx].clone(),
                _ => self.engine.fresh_type_var_at(rhs_span.clone()),
            };

            let index_expr = HirExpr {
                ty: elem_ty.clone(),
                kind: HirExprKind::TupleIndex(
                    Box::new(HirExpr {
                        ty: tmp_ty.clone(),
                        kind: HirExprKind::ResolvedVar(HirVarRef {
                            name: tmp_name.clone(),
                            target: HirVarTarget::Local(tmp_local_id),
                        }),
                        span: rhs_span.clone(),
                    }),
                    idx as u32,
                ),
                span: rhs_span.clone(),
            };

            let local_id = self.fresh_local_id();
            self.scope
                .define_local(name.clone(), elem_ty.clone(), mutable, local_id);
            stmts.push(HirStmt::Let {
                name,
                local_id,
                ty: elem_ty,
                value: index_expr,
                mutable,
            });
        }

        stmts
    }

    pub(crate) fn extract_pattern_binding(&self, pattern: &ast::Pattern) -> (String, bool) {
        match &pattern.kind {
            ast::PatternKind::Ident(ident_pat) => (ident_pat.name.name.clone(), ident_pat.mut_),
            ast::PatternKind::Wildcard => ("_".to_string(), false),
            _ => {
                let name = pattern
                    .binding
                    .as_ref()
                    .map(|b| b.name.clone())
                    .unwrap_or_else(|| "_".to_string());
                (name, false)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ast::{
        Assignment, AssignmentLHS, Expression, Ident, IdentPattern, Literal, LiteralKind, Operand,
        Pattern, PatternKind, PrimaryExpr, UnaryExpr,
    };
    use crate::hir::{HirExprKind, HirStmt, HirVarRef, HirVarTarget};
    use crate::ids::TypeVarId;
    use crate::lexer::Span;
    use crate::lower::Lowerer;
    use crate::types::Type;

    fn number_expr(value: u64) -> Expression {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Literal(Literal {
                kind: LiteralKind::Number(value),
                span: Span::test(),
            }),
            secondaries: None,
            type_annotation: None,
        }))
    }

    fn int_expr(value: u64) -> Expression {
        number_expr(value)
    }

    fn tuple_expr(elements: Vec<Expression>) -> Expression {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Tuple(crate::ast::Tuple { elements }),
            secondaries: None,
            type_annotation: None,
        }))
    }

    fn ident(name: &str) -> Ident {
        Ident {
            name: name.to_string(),
            span: Span::test(),
        }
    }

    fn binding_pattern(name: &str) -> Pattern {
        Pattern {
            binding: Some(ident(name)),
            kind: PatternKind::Ident(IdentPattern {
                name: ident(name),
                mut_: false,
            }),
        }
    }

    #[test]
    fn tuple_destructuring_temp_name_does_not_consume_type_var_id() {
        let mut lowerer = Lowerer::new_for_test();

        let _ = lowerer.with_test_body_context(|lowerer| {
            lowerer.lower_tuple_destructuring(&[], &number_expr(1))
        });

        assert_eq!(lowerer.engine.fresh_type_var(), Type::TypeVar(TypeVarId(0)));
    }

    #[test]
    fn tuple_destructuring_assigns_ids_to_temp_and_bindings() {
        let mut lowerer = Lowerer::new_for_test();
        let stmts = lowerer.with_test_body_context(|lowerer| {
            lowerer.lower_tuple_destructuring(
                &[binding_pattern("left"), binding_pattern("right")],
                &tuple_expr(vec![int_expr(1), int_expr(2)]),
            )
        });

        let HirStmt::Let {
            name: temp_name,
            local_id: temp_id,
            ..
        } = &stmts[0]
        else {
            panic!("expected tuple temp let");
        };
        assert!(temp_name.starts_with("__tuple_tmp_"));

        let HirStmt::Let {
            name: left_name,
            local_id: left_id,
            value,
            ..
        } = &stmts[1]
        else {
            panic!("expected left binding let");
        };
        assert_eq!(left_name, "left");
        assert_ne!(temp_id, left_id);
        match &value.kind {
            HirExprKind::TupleIndex(base, 0) => match &base.kind {
                HirExprKind::ResolvedVar(HirVarRef {
                    target: HirVarTarget::Local(id),
                    ..
                }) => {
                    assert_eq!(id, temp_id);
                }
                other => panic!("expected tuple temp local ref, got {other:?}"),
            },
            other => panic!("expected tuple index, got {other:?}"),
        }
    }

    #[test]
    fn reassignment_targets_shadowing_local_id() {
        let mut lowerer = Lowerer::new_for_test();
        let (inner, stmt) = lowerer.with_test_body_context(|lowerer| {
            let outer = lowerer.fresh_local_id();
            lowerer
                .scope
                .define_local("x".to_string(), Type::I64, true, outer);
            lowerer.scope.push();
            let inner = lowerer.fresh_local_id();
            lowerer
                .scope
                .define_local("x".to_string(), Type::I64, true, inner);
            let stmt = lowerer.lower_assignment(&Assignment {
                lhs: AssignmentLHS::Pattern {
                    pattern: binding_pattern("x"),
                    type_annotation: None,
                },
                rhs: number_expr(2),
            });
            (inner, stmt)
        });

        let HirStmt::Expr(expr) = stmt else {
            panic!("expected assignment expression statement");
        };
        let HirExprKind::Assign(lhs, _) = expr.kind else {
            panic!("expected assignment expression");
        };
        match lhs.kind {
            HirExprKind::ResolvedVar(HirVarRef {
                target: HirVarTarget::Local(id),
                name,
            }) => {
                assert_eq!(name, "x");
                assert_eq!(id, inner);
            }
            other => panic!("expected assignment lhs local ref, got {other:?}"),
        }
    }
}

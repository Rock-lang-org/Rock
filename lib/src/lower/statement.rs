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
        self.mark_escaping_lambdas(&lambda.body);
        let mut body = self.lower_block(&lambda.body);
        // A literal returned from an inferred callable carries numeric evidence,
        // not a committed I64: callers may supply its expected integer type.
        let inferred_return = self.current_body_return_type().is_some_and(|ty| {
            matches!(self.engine.resolve(&ty), Type::TypeVar(_))
        });
        if inferred_return && !matches!(lambda.arrow_kind, ast::LambdaArrowKind::Unit) {
            if let Some(HirStmt::Expr(value)) = body.stmts.last_mut() {
                if matches!(value.kind, HirExprKind::IntLiteral(_)) {
                    let ty = self.engine.fresh_type_var_at(value.span.clone());
                    if let Type::TypeVar(var) = ty {
                        self.constraint_store.add_int_literal(var, value.span.clone());
                    }
                    value.ty = ty.clone();
                    body.ty = ty;
                }
            }
        }
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
                if let Some(context) = self.body.fold_loops().last() {
                    let unit = context.unit();
                    HirStmt::Return(Some(self.wrap_fold_return(unit)))
                } else {
                    HirStmt::Return(None)
                }
            }
            ast::Statement::Break(Some(expr)) => {
                let hir_expr = self.lower_expression(expr);
                if let Some(context) = self.body.innermost_fold_loop() {
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
            ast::Statement::Break(None) => match self.body.innermost_fold_loop() {
                Some(context) => HirStmt::Return(Some(context.stop())),
                _ => HirStmt::Break(None),
            },
            ast::Statement::Continue(_) => match self.body.innermost_fold_loop() {
                Some(context) => HirStmt::Return(Some(context.next())),
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

                // An existing local is reassigned. Top-level items are never
                // reassigned: the name starts a new local that shadows them.
                let reassigns_local = self
                    .scope
                    .lookup(&name)
                    .is_some_and(|binding| binding.local_id.is_some() && !binding.is_top_level);
                if type_annotation.is_none() && reassigns_local {
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
                        self.body.current_source_scope(),
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
        let uid = self.body.fresh_tuple_temp_id();
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
mod tests;

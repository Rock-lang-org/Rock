//! Statement lowering methods for Lowerer

use crate::ast;
use crate::hir::*;
use crate::lower::expression::ExprUse;
use crate::types::Type;

use crate::lower::Lowerer;

/// A `return` whose value type is checked against its body's return type.
pub(crate) struct PendingReturnCheck {
    value_ty: Type,
    expected: Type,
    span: crate::lexer::Span,
    owner: Option<ReturnOwner>,
}

/// The function or closure a `return` leaves, for diagnostic context.
#[derive(Clone)]
pub(crate) struct ReturnOwner {
    /// Declaration site: the function name, or the closure's arrow.
    pub(crate) span: crate::lexer::Span,
    /// How diagnostics name it, e.g. "function `explicit`" or "closure".
    pub(crate) description: String,
}

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
                if let ast::AssignmentLHS::Pattern {
                    pattern,
                    type_annotation,
                } = &assign.lhs
                {
                    if Self::is_destructuring_pattern(pattern) {
                        let expanded = self.lower_pattern_destructuring(
                            pattern,
                            type_annotation.as_ref(),
                            &assign.rhs,
                        );
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
        self.prepare_capture_liveness(&lambda.body);
        let prelude = self.param_destructuring_prelude(&lambda.parameters);
        let mut body = self.lower_block(&lambda.body);
        body.stmts.splice(0..0, prelude);
        // A literal returned from an inferred callable carries numeric evidence,
        // not a committed I64: callers may supply its expected integer type.
        let inferred_return = self
            .current_body_return_type()
            .is_some_and(|ty| matches!(self.engine.resolve(&ty), Type::TypeVar(_)));
        if inferred_return && !matches!(lambda.arrow_kind, ast::LambdaArrowKind::Unit) {
            if let Some(HirStmt::Expr(value)) = body.stmts.last_mut() {
                if matches!(value.kind, HirExprKind::IntLiteral(_)) {
                    let ty = self.engine.fresh_type_var_at(value.span.clone());
                    if let Type::TypeVar(var) = ty {
                        self.constraint_store
                            .add_int_literal(var, value.span.clone());
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
            // Control leaves the block: its value never reaches the enclosing
            // expression, so a returning `if`/`match` arm does not shape it.
            HirStmt::Return(_) => Type::Never,
            HirStmt::Break(_) => Type::Never,
            HirStmt::Continue => Type::Never,
        }
    }

    /// A `return` exits the enclosing function or closure, so its value (unit
    /// for a bare `return`) must have that body's return type. Inside `for`
    /// loops, the outermost loop recorded the type of the body it exits.
    fn check_return_type(&mut self, value_ty: &Type, span: &crate::lexer::Span) {
        let outermost_loop = self.body.fold_loops().next().map(|l| l.return_ty().clone());
        let expected = match outermost_loop {
            Some(return_ty) => Some(return_ty),
            None => self.current_body_return_type(),
        };
        let Some(expected) = expected else {
            return;
        };
        let check = PendingReturnCheck {
            value_ty: value_ty.clone(),
            expected,
            span: span.clone(),
            owner: self.return_owner.clone(),
        };
        // An inferred return type is still open here. Binding it to an early
        // `return` would blame the body's final value instead: defer until the
        // owner has unified its body (`flush_return_checks`).
        if matches!(self.engine.resolve(&check.expected), Type::TypeVar(_)) {
            self.pending_return_checks.push(check);
        } else {
            self.apply_return_check(check, None);
        }
    }

    /// Makes `owner` the target named by `return` diagnostics while lowering
    /// its body; returns the previous owner for `restore_return_owner`.
    pub(crate) fn enter_return_owner(
        &mut self,
        span: crate::lexer::Span,
        description: String,
    ) -> Option<ReturnOwner> {
        self.return_owner.replace(ReturnOwner { span, description })
    }

    pub(crate) fn restore_return_owner(&mut self, previous: Option<ReturnOwner>) {
        self.return_owner = previous;
    }

    /// Marks where a body's deferred return checks start.
    pub(crate) fn return_checks_mark(&self) -> usize {
        self.pending_return_checks.len()
    }

    /// Call before unifying a body with its return type. When the body never
    /// completes (it ends in `return`), its `Never` type must not decide the
    /// return type: the returned values do, so run their checks first.
    pub(crate) fn flush_return_checks_for_diverging_body(&mut self, mark: usize, body_ty: &Type) {
        if self.engine.resolve(body_ty) == Type::Never {
            self.flush_return_checks(mark, None);
        }
    }

    /// The span of the value a body completes with, which fixes an inferred
    /// return type. None when the body ends in `return`, `break`, ...
    fn body_result_span(body: &HirBlock) -> Option<crate::lexer::Span> {
        if body.ty == Type::Never {
            return None;
        }
        match body.stmts.last()? {
            HirStmt::Expr(value) if value.ty != Type::Never => Some(value.span.clone()),
            _ => None,
        }
    }

    /// A callable body ending in `return` has type `Never`, but later phases
    /// compare the body with the callable's signature: give it the return type.
    pub(crate) fn type_diverging_body_as_return(body: &mut HirBlock, return_ty: &Type) {
        if body.ty == Type::Never && *return_ty != Type::Never {
            body.ty = return_ty.clone();
        }
    }

    /// Runs the return checks deferred since `mark`, after the body's owner
    /// unified the body value with its return type.
    /// `body` is the owner's lowered body, used to point at its final value.
    pub(crate) fn flush_return_checks(&mut self, mark: usize, body: Option<&HirBlock>) {
        if mark >= self.pending_return_checks.len() {
            return;
        }
        let result_span = body.and_then(Self::body_result_span);
        for check in self.pending_return_checks.split_off(mark) {
            self.apply_return_check(check, result_span.clone());
        }
    }

    fn report_return_mismatch(
        &mut self,
        message: String,
        check: PendingReturnCheck,
        result_span: Option<crate::lexer::Span>,
        expected: String,
    ) {
        let mut labels = Vec::new();
        if let Some(owner) = &check.owner {
            labels.push(crate::diagnostic::DiagnosticLabel::new(
                format!("{} returns {expected}", owner.description),
                owner.span.clone(),
            ));
        }
        if let Some(span) = result_span.filter(|span| *span != check.span) {
            labels.push(crate::diagnostic::DiagnosticLabel::new(
                format!("this final value makes it {expected}"),
                span,
            ));
        }
        self.diagnostics
            .push_type_with_span_and_labels(message, check.span, labels);
    }

    fn apply_return_check(
        &mut self,
        check: PendingReturnCheck,
        result_span: Option<crate::lexer::Span>,
    ) {
        // A trailing integer literal leaves the return type an open integer
        // variable. Unifying it with a non-integer `return` would blame the
        // literal: report the `return` instead.
        let expected = self.engine.resolve(&check.expected);
        let value = self.engine.resolve(&check.value_ty);
        if let Type::TypeVar(_) = expected {
            let integer_literal = self
                .constraint_store
                .int_literal_vars()
                .any(|var| self.engine.resolve(&Type::TypeVar(var)) == expected);
            let concrete_non_integer =
                !matches!(value, Type::TypeVar(_) | Type::Never | Type::Error)
                    && !value.is_integer();
            if integer_literal && concrete_non_integer {
                let message = format!(
                    "Return type mismatch: this body returns an integer, but this `return` gives `{}`",
                    self.display_type(&value)
                );
                self.report_return_mismatch(message, check, result_span, "an integer".to_string());
                return;
            }
        }
        if let Err(error) = self.engine.unify(&check.value_ty, &check.expected) {
            let message = format!("Return type mismatch: {}", error.render(&self.engine));
            let expected = self.engine.resolve(&check.expected);
            let expected = format!("`{}`", self.display_type(&expected));
            self.report_return_mismatch(message, check, result_span, expected);
        }
    }

    pub(crate) fn lower_statement(&mut self, stmt: &ast::Statement, _is_last: bool) -> HirStmt {
        match stmt {
            ast::Statement::Assignment(assign) => self.lower_assignment(assign),
            ast::Statement::Expression(expr) => {
                let hir_expr = self.lower_expression(expr);
                HirStmt::Expr(hir_expr)
            }
            ast::Statement::Return(Some(expr), _) => {
                let hir_expr = self.lower_expression(expr);
                self.check_return_type(&hir_expr.ty, &hir_expr.span);
                HirStmt::Return(Some(self.wrap_fold_return(hir_expr)))
            }
            ast::Statement::Return(None, span) => {
                self.check_return_type(&Type::Unit, span);
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
                    let ann_ty = self.lower_annotation_type(ann);
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
                    if type_annotation.is_none() {
                        self.source_map.mark_inferred_binding(owner, local_id);
                    }
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

    /// Lowers `pattern = rhs` for a destructuring pattern. The value is kept
    /// in a temporary and each binding becomes a `let` of its projection, so
    /// the pattern's bindings stay in scope for the rest of the block. Typing
    /// and binding modes come from `lower_pattern`, shared with `match`.
    pub(crate) fn lower_pattern_destructuring(
        &mut self,
        pattern: &ast::Pattern,
        type_annotation: Option<&ast::ParseType>,
        rhs: &ast::Expression,
    ) -> Vec<HirStmt> {
        let mut rhs_expr = self.lower_expression(rhs);
        if let Some(annotation) = type_annotation {
            self.source_map.record_type_annotation(annotation.span());
            let annotated = self.lower_annotation_type(annotation);
            if let Err(error) = self.engine.unify(&rhs_expr.ty, &annotated) {
                self.diagnostics.push_type_with_span(
                    format!("Type annotation mismatch: {}", error.render(&self.engine)),
                    annotation.span(),
                );
            }
            rhs_expr.ty = annotated;
        }
        let rhs_span = rhs_expr.span.clone();
        let value_ty = rhs_expr.ty.clone();
        self.bind_destructuring(pattern, rhs_expr, &value_ty, rhs_span)
    }

    /// Binds `pattern` against `value` (of type `value_ty`) as a temporary
    /// followed by one `let` per binding. Refutable patterns are rejected.
    pub(crate) fn bind_destructuring(
        &mut self,
        pattern: &ast::Pattern,
        value: HirExpr,
        value_ty: &Type,
        span: crate::lexer::Span,
    ) -> Vec<HirStmt> {
        let hir_pattern = self.lower_pattern(pattern, value_ty);
        if !crate::lower::control_flow::pattern::pattern_is_irrefutable(&hir_pattern) {
            self.diagnostics.push_type_with_span(
                "refutable pattern in assignment: it may not match every value; use `match`"
                    .to_string(),
                Self::pattern_binding_span(pattern).unwrap_or_else(|| span.clone()),
            );
        }
        let value_ty = self.engine.resolve(value_ty);
        let mut stmts = Vec::new();
        // A local is read in place, so taking some of its fields moves only
        // those (and copies nothing else); other values go to a temporary.
        let place = if matches!(
            value.kind,
            HirExprKind::ResolvedVar(HirVarRef {
                target: HirVarTarget::Local(_),
                ..
            })
        ) {
            HirExpr {
                ty: value_ty,
                ..value
            }
        } else {
            let uid = self.body.fresh_pattern_temp_id();
            let tmp_name = format!("__pattern_tmp_{uid}");
            let tmp_local_id = self.fresh_local_id();
            self.scope
                .define_local(tmp_name.clone(), value_ty.clone(), false, tmp_local_id);
            stmts.push(HirStmt::Let {
                name: tmp_name.clone(),
                local_id: tmp_local_id,
                ty: value_ty.clone(),
                value,
                mutable: false,
            });
            HirExpr {
                ty: value_ty,
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: tmp_name,
                    target: HirVarTarget::Local(tmp_local_id),
                }),
                span: span.clone(),
            }
        };
        self.push_pattern_projections(&hir_pattern, place, &span, &mut stmts);
        stmts
    }

    /// Destructures the parameters written as patterns (`(a, b) -> ...`).
    /// Each one arrives under its hidden parameter name, already in scope.
    pub(crate) fn param_destructuring_prelude(&mut self, params: &[ast::Pattern]) -> Vec<HirStmt> {
        let mut stmts = Vec::new();
        for pattern in params {
            if !Self::param_is_destructuring(pattern) {
                continue;
            }
            let name = Self::destructured_param_name(pattern);
            let Some(binding) = self.scope.lookup(&name).cloned() else {
                continue;
            };
            let Some(local_id) = binding.local_id else {
                continue;
            };
            // A pattern with no binding has nothing to bind.
            let Some(span) = Self::pattern_binding_span(pattern) else {
                continue;
            };
            let value = HirExpr {
                ty: binding.ty.clone(),
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name,
                    target: HirVarTarget::Local(local_id),
                }),
                span: span.clone(),
            };
            stmts.extend(self.bind_destructuring(pattern, value, &binding.ty, span));
        }
        stmts
    }

    /// A parameter pattern that destructures its argument. A reference
    /// pattern around a plain name (`&x`) only declares a reference parameter.
    pub(crate) fn param_is_destructuring(pattern: &ast::Pattern) -> bool {
        match &pattern.kind {
            ast::PatternKind::Tuple(_)
            | ast::PatternKind::Instance(_)
            | ast::PatternKind::Array(_) => true,
            ast::PatternKind::Nested(inner)
            | ast::PatternKind::Reference { pattern: inner, .. } => {
                Self::param_is_destructuring(inner)
            }
            _ => false,
        }
    }

    /// Emits a `let` for each binding of `pattern`, reading it from `place`.
    fn push_pattern_projections(
        &mut self,
        pattern: &HirPattern,
        place: HirExpr,
        span: &crate::lexer::Span,
        stmts: &mut Vec<HirStmt>,
    ) {
        let place_ty = self.engine.resolve(&place.ty);
        let project = |kind, ty| HirExpr {
            ty,
            kind,
            span: span.clone(),
        };
        match pattern {
            HirPattern::Binding {
                name,
                local_id,
                mutable,
                mode,
            } => {
                let (ty, value) = match mode {
                    crate::hir::HirBindingMode::Value => (place_ty, place),
                    crate::hir::HirBindingMode::Ref { mutable } => {
                        let ty = Type::Reference {
                            mutable: *mutable,
                            inner: Box::new(place_ty),
                        };
                        let value =
                            project(HirExprKind::Ref(*mutable, Box::new(place)), ty.clone());
                        (ty, value)
                    }
                };
                stmts.push(HirStmt::Let {
                    name: name.clone(),
                    local_id: *local_id,
                    ty,
                    value,
                    mutable: *mutable,
                });
            }
            HirPattern::Tuple(patterns) => {
                let Type::Tuple(element_types) = place_ty else {
                    return;
                };
                for (index, (pattern, ty)) in patterns.iter().zip(element_types).enumerate() {
                    let element = project(
                        HirExprKind::TupleIndex(Box::new(place.clone()), index as u32),
                        ty,
                    );
                    self.push_pattern_projections(pattern, element, span, stmts);
                }
            }
            HirPattern::Struct(type_name, _, type_args, fields) => {
                for field in fields {
                    let ty = self
                        .lower_struct_field_type(type_name, type_args, &field.name, span)
                        .unwrap_or(Type::Error);
                    let access = project(
                        HirExprKind::FieldAccess(
                            Box::new(place.clone()),
                            field.name.clone(),
                            field.field.clone(),
                        ),
                        ty,
                    );
                    self.push_pattern_projections(&field.pattern, access, span, stmts);
                }
            }
            HirPattern::Deref(inner) => {
                let Type::Reference { inner: target, .. } = place_ty else {
                    return;
                };
                let target = project(HirExprKind::Deref(Box::new(place)), *target);
                self.push_pattern_projections(inner, target, span, stmts);
            }
            HirPattern::Array { .. } => {
                self.push_array_pattern_bindings(pattern, place, span, stmts)
            }
            // Refutable patterns were already reported.
            HirPattern::Wildcard
            | HirPattern::Literal(_)
            | HirPattern::Enum(..)
            | HirPattern::Or(_) => {}
        }
    }

    /// Patterns that take a value apart rather than naming it.
    pub(crate) fn is_destructuring_pattern(pattern: &ast::Pattern) -> bool {
        match &pattern.kind {
            ast::PatternKind::Tuple(_)
            | ast::PatternKind::Array(_)
            | ast::PatternKind::Instance(_)
            | ast::PatternKind::Reference { .. } => true,
            ast::PatternKind::Nested(inner) => Self::is_destructuring_pattern(inner),
            _ => false,
        }
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

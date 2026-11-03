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
mod tests;

//! Expression lowering entry points and application precedence.

use super::operand::{AstExprOperand, ExprUse};
use super::APPLICATION_PRECEDENCE;
use crate::ast;
use crate::hir::*;
use crate::lower::Lowerer;
use crate::types::Type;

impl Lowerer {
    // ============================================================
    // Expression Lowering Methods
    // ============================================================

    pub(crate) fn op_precedence(&self, op: &str) -> Result<u8, String> {
        self.infix_precedence.get(op).copied().ok_or_else(|| {
            format!(
                "Operator '{}' has no precedence declaration (use 'infix N {}')",
                op, op
            )
        })
    }

    pub(crate) fn lower_expression(&mut self, expr: &ast::Expression) -> HirExpr {
        let lowered = self.lower_expression_with_use(expr, ExprUse::Value);
        self.source_map.record_expression(lowered.span.clone());
        lowered
    }

    fn flatten_owned_binop(
        expr: &ast::Expression,
        operands: &mut Vec<ast::Expression>,
        operators: &mut Vec<ast::Operator>,
    ) {
        match expr {
            ast::Expression::BinopExpr(lhs, operator, rhs) => {
                operands.push(ast::Expression::UnaryExpr(lhs.clone()));
                operators.push(operator.clone());
                Self::flatten_owned_binop(rhs, operands, operators);
            }
            _ => operands.push(expr.clone()),
        }
    }

    fn build_owned_binop_chain(
        operands: &[ast::Expression],
        operators: &[ast::Operator],
    ) -> Option<ast::Expression> {
        let mut expression = operands.last()?.clone();
        for index in (0..operators.len()).rev() {
            let ast::Expression::UnaryExpr(lhs) = operands[index].clone() else {
                return None;
            };
            expression =
                ast::Expression::BinopExpr(lhs, operators[index].clone(), Box::new(expression));
        }
        Some(expression)
    }

    fn append_ast_secondaries(
        expression: ast::Expression,
        mut trailing: Vec<ast::SecondaryExpr>,
    ) -> ast::Expression {
        if trailing.is_empty() {
            return expression;
        }

        if let ast::Expression::UnaryExpr(ast::UnaryExpr::PrimaryExpr(mut primary)) = expression {
            primary
                .secondaries
                .get_or_insert_with(Vec::new)
                .append(&mut trailing);
            ast::Expression::UnaryExpr(ast::UnaryExpr::PrimaryExpr(primary))
        } else {
            ast::Expression::UnaryExpr(ast::UnaryExpr::PrimaryExpr(ast::PrimaryExpr {
                operand: ast::Operand::Expression(Box::new(expression)),
                secondaries: Some(trailing),
                type_annotation: None,
            }))
        }
    }

    fn apply_application_precedence(&self, expr: &ast::Expression) -> Option<ast::Expression> {
        match expr {
            ast::Expression::BinopExpr(lhs, operator, rhs) => {
                if let Some(rhs) = self.apply_application_precedence(rhs) {
                    return Some(ast::Expression::BinopExpr(
                        lhs.clone(),
                        operator.clone(),
                        Box::new(rhs),
                    ));
                }

                let lhs_expression = ast::Expression::UnaryExpr(lhs.clone());
                if let Some(lhs) = self.apply_application_precedence(&lhs_expression) {
                    let mut operands = Vec::new();
                    let mut operators = Vec::new();
                    Self::flatten_owned_binop(&lhs, &mut operands, &mut operators);
                    operators.push(operator.clone());
                    operands.push((**rhs).clone());
                    return Self::build_owned_binop_chain(&operands, &operators);
                }
            }
            ast::Expression::CastExpr(inner, ty) => {
                if let Some(inner) = self.apply_application_precedence(inner) {
                    return Some(ast::Expression::CastExpr(Box::new(inner), ty.clone()));
                }
            }
            ast::Expression::UnaryExpr(_) => {}
            ast::Expression::Range(_) => {}
        }

        let ast::Expression::UnaryExpr(ast::UnaryExpr::PrimaryExpr(primary)) = expr else {
            return None;
        };
        let secondaries = primary.secondaries.as_ref()?;

        for (secondary_index, secondary) in secondaries.iter().enumerate() {
            let ast::SecondaryExpr::Arguments(arguments) = secondary else {
                continue;
            };

            for (argument_index, argument) in arguments.iter().enumerate() {
                let Some(argument) = self.apply_application_precedence(&argument.arg) else {
                    continue;
                };

                let mut primary = primary.clone();
                let secondaries = primary.secondaries.as_mut()?;
                let ast::SecondaryExpr::Arguments(arguments) = &mut secondaries[secondary_index]
                else {
                    unreachable!();
                };
                arguments[argument_index].arg = argument;
                return Some(ast::Expression::UnaryExpr(ast::UnaryExpr::PrimaryExpr(
                    primary,
                )));
            }
        }

        for (secondary_index, secondary) in secondaries.iter().enumerate() {
            let ast::SecondaryExpr::Arguments(arguments) = secondary else {
                continue;
            };
            let Some(last_argument) = arguments.last() else {
                continue;
            };

            let mut operands = Vec::new();
            let mut operators = Vec::new();
            Self::flatten_owned_binop(&last_argument.arg, &mut operands, &mut operators);
            let Some(split_at) = operators.iter().position(|operator| {
                self.infix_precedence
                    .get(&operator.value)
                    .is_some_and(|precedence| *precedence <= APPLICATION_PRECEDENCE)
            }) else {
                continue;
            };

            let argument =
                Self::build_owned_binop_chain(&operands[..=split_at], &operators[..split_at])?;
            let mut call_secondaries = secondaries[..=secondary_index].to_vec();
            let ast::SecondaryExpr::Arguments(call_arguments) =
                &mut call_secondaries[secondary_index]
            else {
                unreachable!();
            };
            call_arguments.last_mut()?.arg = argument;

            let mut call_primary = primary.clone();
            call_primary.secondaries = Some(call_secondaries);
            let call = ast::Expression::UnaryExpr(ast::UnaryExpr::PrimaryExpr(call_primary));

            let mut outer_operands = vec![call];
            outer_operands.extend_from_slice(&operands[split_at + 1..]);
            let outer = Self::build_owned_binop_chain(&outer_operands, &operators[split_at..])?;
            let trailing = secondaries[secondary_index + 1..].to_vec();
            return Some(Self::append_ast_secondaries(outer, trailing));
        }

        None
    }

    pub(super) fn lower_expression_with_use(
        &mut self,
        expr: &ast::Expression,
        use_kind: ExprUse,
    ) -> HirExpr {
        if let ast::Expression::Range(range) = expr {
            if use_kind == ExprUse::AssignmentPlace {
                self.diagnostics.push_type_with_span(
                    "Range expressions cannot be used as assignment places".to_string(),
                    range.span.clone(),
                );
                return self.error_expression_at(range.span.clone());
            }

            let Some(range_items) = self.language_items.range.clone() else {
                self.diagnostics.push_with_span(
                    "Cannot use range syntax because the Range language-item protocol is unavailable"
                        .to_string(),
                    range.span.clone(),
                );
                return self.error_expression_at(range.span.clone());
            };

            let start = range
                .start
                .as_ref()
                .map(|start| self.lower_expression(start));
            let end = range.end.as_ref().map(|end| self.lower_expression(end));
            if let Some(start) = &start {
                let _ = self.engine.unify(&start.ty, &Type::I64);
            }
            if let Some(end) = &end {
                let _ = self.engine.unify(&end.ty, &Type::I64);
            }

            let variant_id = match (start.is_some(), end.is_some(), range.inclusive) {
                (false, false, false) => range_items.full_variant_id,
                (true, false, false) => range_items.from_variant_id,
                (false, true, false) => range_items.to_variant_id,
                (false, true, true) => range_items.to_inclusive_variant_id,
                (true, true, false) => range_items.exclusive_variant_id,
                (true, true, true) => range_items.inclusive_variant_id,
                (_, false, true) => unreachable!("parser rejects inclusive ranges without an end"),
            };
            let Some(enumeration) = self.items.enumeration(range_items.enum_id) else {
                self.diagnostics.push_with_span(
                    "Range language item refers to an unavailable enum".to_string(),
                    range.span.clone(),
                );
                return self.error_expression_at(range.span.clone());
            };
            let Some(variant) = enumeration
                .variants
                .iter()
                .find(|variant| variant.id == variant_id)
            else {
                self.diagnostics.push_with_span(
                    "Range language item refers to an unavailable variant".to_string(),
                    range.span.clone(),
                );
                return self.error_expression_at(range.span.clone());
            };
            let enum_name = enumeration.name.clone();
            let variant_name = variant.name.clone();
            let mut args = Vec::new();
            if let Some(start) = start {
                args.push(start);
            }
            if let Some(end) = end {
                args.push(end);
            }
            return HirExpr {
                ty: Type::Enum {
                    id: range_items.enum_id,
                    args: Vec::new(),
                },
                kind: HirExprKind::EnumVariant(
                    enum_name,
                    variant_name.clone(),
                    args,
                    Some(HirVariantLocation {
                        owner: range_items.enum_id,
                        variant_id,
                        name: variant_name,
                    }),
                ),
                span: range.span.clone(),
            };
        }

        if let Some(expression) = self.apply_application_precedence(expr) {
            return self.lower_expression_with_use(&expression, use_kind);
        }

        // Handle `expr as Type` casts
        if let ast::Expression::CastExpr(inner, parse_ty) = expr {
            if use_kind == ExprUse::AssignmentPlace {
                self.diagnostics.push_type_with_span(
                    "Cast expressions cannot be used as assignment places".to_string(),
                    parse_ty.span(),
                );
                return self.error_expression_at(parse_ty.span());
            }
            let inner_hir = self.lower_expression_with_use(inner, use_kind);
            let target_ty = self.lower_parse_type(parse_ty);
            let resolved_inner_ty =
                self.resolve_projection_type(&self.engine.resolve(&inner_hir.ty));
            let resolved_target_ty = self.resolve_projection_type(&self.engine.resolve(&target_ty));

            if Self::is_fat_raw_slice_pointer(&resolved_inner_ty) && target_ty.is_integer() {
                self.diagnostics.push_type_with_span(
                    "Cannot cast fat raw slice pointer to integer".to_string(),
                    parse_ty.span(),
                );
            }

            if resolved_inner_ty.is_integer() && Self::is_fat_raw_slice_pointer(&resolved_target_ty)
            {
                self.diagnostics.push_with_span(
                    "Cannot cast integer to fat raw slice pointer".to_string(),
                    parse_ty.span(),
                );
            }

            return HirExpr {
                ty: target_ty.clone(),
                kind: HirExprKind::Cast(Box::new(inner_hir), target_ty),
                span: parse_ty.span(),
            };
        }

        // Flatten the right-recursive BinopExpr chain into a list
        let mut operands: Vec<AstExprOperand<'_>> = Vec::new();
        let mut operators: Vec<String> = Vec::new();

        self.flatten_binop(expr, &mut operands, &mut operators);

        if operands.len() == 1 {
            return self.lower_ast_expr_operand_with_use(&operands[0], use_kind);
        }

        // Build tree respecting precedence using precedence climbing
        self.build_precedence_tree_with_use(&operands, &operators, 0, operands.len() - 1, use_kind)
    }

    /// Flatten a right-recursive BinopExpr chain into operands and operators
    pub(crate) fn flatten_binop<'a>(
        &mut self,
        expr: &'a ast::Expression,
        operands: &mut Vec<AstExprOperand<'a>>,
        operators: &mut Vec<String>,
    ) {
        match expr {
            ast::Expression::BinopExpr(lhs, op, rhs) => {
                operands.push(AstExprOperand::Unary(lhs));
                operators.push(op.value.clone());
                self.flatten_binop(rhs, operands, operators);
            }
            ast::Expression::UnaryExpr(unary) => {
                operands.push(AstExprOperand::Unary(unary));
            }
            ast::Expression::CastExpr(_, _) => {
                operands.push(AstExprOperand::Cast(expr));
            }
            ast::Expression::Range(_) => {
                unreachable!("ranges are lowered before binary flattening")
            }
        }
    }

    pub(super) fn lower_ast_expr_operand_with_use(
        &mut self,
        operand: &AstExprOperand<'_>,
        use_kind: ExprUse,
    ) -> HirExpr {
        match operand {
            AstExprOperand::Unary(unary) => {
                if use_kind == ExprUse::Value {
                    self.lower_unary_expr(unary)
                } else {
                    self.lower_unary_expr_with_use(unary, use_kind)
                }
            }
            AstExprOperand::Cast(expr) => self.lower_expression_with_use(expr, use_kind),
        }
    }
}

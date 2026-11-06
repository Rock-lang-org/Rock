//! Finds lambdas that a body returns.
//!
//! A returned lambda outlives the frame that created it, so it must own what
//! it captures: borrowing a parameter or local of that frame would dangle.
//! Bodies mark their returned lambdas before lowering; lambda lowering then
//! captures by move for marked lambdas.

use crate::ast;
use crate::lower::Lowerer;

impl Lowerer {
    /// Marks the lambdas `block` returns: its tail expression, its top-level
    /// `return` values, and locals of the block returned by name.
    pub(crate) fn mark_escaping_lambdas(&mut self, block: &ast::Block) {
        let mut returned = Vec::new();
        if let Some(ast::Statement::Expression(tail)) = block.statements.last() {
            returned.push(tail);
        }
        for statement in &block.statements {
            if let ast::Statement::Return(Some(value)) = statement {
                returned.push(value);
            }
        }

        for value in returned {
            if let Some(lambda) = Self::expression_lambda(value) {
                self.body.mark_escaping_lambda(lambda);
            } else if let Some(name) = Self::expression_local_name(value) {
                for lambda in Self::lambdas_bound_to(block, name) {
                    self.body.mark_escaping_lambda(lambda);
                }
            }
        }
    }

    pub(crate) fn is_escaping_lambda(&self, lambda: &ast::LambdaDecl) -> bool {
        self.body.is_escaping_lambda(lambda)
    }

    fn expression_operand(expression: &ast::Expression) -> Option<&ast::Operand> {
        let ast::Expression::UnaryExpr(ast::UnaryExpr::PrimaryExpr(primary)) = expression else {
            return None;
        };
        if primary.secondaries.as_ref().is_some_and(|items| !items.is_empty()) {
            return None;
        }
        match &primary.operand {
            ast::Operand::Expression(inner) => Self::expression_operand(inner),
            operand => Some(operand),
        }
    }

    fn expression_lambda(expression: &ast::Expression) -> Option<&ast::LambdaDecl> {
        match Self::expression_operand(expression)? {
            ast::Operand::LambdaDecl(lambda) => Some(lambda),
            _ => None,
        }
    }

    fn expression_local_name(expression: &ast::Expression) -> Option<&str> {
        match Self::expression_operand(expression)? {
            ast::Operand::Ident(ast::IdentifierPath { path }) => match path.as_slice() {
                [ast::IdentOrType::Ident(ident)] => Some(ident.name.as_str()),
                _ => None,
            },
            _ => None,
        }
    }

    fn lambdas_bound_to<'a>(
        block: &'a ast::Block,
        name: &'a str,
    ) -> impl Iterator<Item = &'a ast::LambdaDecl> {
        block.statements.iter().filter_map(move |statement| {
            let ast::Statement::Assignment(assignment) = statement else {
                return None;
            };
            let ast::AssignmentLHS::Pattern { pattern, .. } = &assignment.lhs else {
                return None;
            };
            let ast::PatternKind::Ident(ident) = &pattern.kind else {
                return None;
            };
            if ident.name.name != name {
                return None;
            }
            Self::expression_lambda(&assignment.rhs)
        })
    }
}

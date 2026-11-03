//! Expression operand kinds and span helpers.

use crate::ast;
use crate::lexer::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExprUse {
    Value,
    AssignmentPlace,
}

pub(crate) enum AstExprOperand<'a> {
    Unary(&'a ast::UnaryExpr),
    Cast(&'a ast::Expression),
}

pub(super) fn ast_expr_operand_span(operand: &AstExprOperand<'_>) -> Option<Span> {
    fn path_span(path: &[ast::IdentOrType]) -> Option<Span> {
        path.first().map(|segment| match segment {
            ast::IdentOrType::Ident(ident) => ident.span.clone(),
            ast::IdentOrType::Type(ty) => ty.span(),
        })
    }

    fn operand_span(operand: &ast::Operand) -> Option<Span> {
        match operand {
            ast::Operand::Literal(literal) => Some(literal.span.clone()),
            ast::Operand::Ident(path) => path_span(&path.path),
            ast::Operand::CallHole(span) => Some(span.clone()),
            ast::Operand::SelfIdent(ident) => Some(ident.span.clone()),
            ast::Operand::Instance(instance) => path_span(&instance.name.path),
            ast::Operand::Tuple(tuple) => tuple.elements.first().and_then(ast_expr_span),
            ast::Operand::Expression(expr) => ast_expr_span(expr),
            ast::Operand::If(if_expr) => ast_expr_span(&if_expr.condition.expression),
            ast::Operand::Match(match_expr) => ast_expr_span(&match_expr.expr),
            ast::Operand::Loop(_) | ast::Operand::LambdaDecl(_) | ast::Operand::Unsafe(_, _) => {
                None
            }
            ast::Operand::NativeOperator(operator) => Some(operator.span.clone()),
            ast::Operand::MacroInvoc(invocation) => Some(invocation.name.span.clone()),
        }
    }

    fn ast_unary_span(unary: &ast::UnaryExpr) -> Option<Span> {
        match unary {
            ast::UnaryExpr::PrimaryExpr(primary) => operand_span(&primary.operand),
            ast::UnaryExpr::UnaryExpr(operator, _) => Some(operator.span.clone()),
        }
    }

    fn ast_expr_span(expression: &ast::Expression) -> Option<Span> {
        match expression {
            ast::Expression::BinopExpr(_, operator, _) => Some(operator.span.clone()),
            ast::Expression::UnaryExpr(unary) => ast_unary_span(unary),
            ast::Expression::CastExpr(_, ty) => Some(ty.span()),
            ast::Expression::Range(range) => Some(range.span.clone()),
        }
    }

    match operand {
        AstExprOperand::Unary(unary) => ast_unary_span(unary),
        AstExprOperand::Cast(expression) => ast_expr_span(expression),
    }
}

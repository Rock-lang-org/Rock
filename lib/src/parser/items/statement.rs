use crate::{
    lexer::TokenType,
    parser::{engine::*, Assignment, AssignmentLHS, Statement},
};

use super::{expression, parse_type, pattern, seek, unary_expr};

fn return_statement(stream: Input) -> IResult<Statement> {
    let span = stream.seek()?.span.clone();
    preceded(TokenType::Keyword("return".to_string()), expression.opt())
        .map(move |value| Statement::Return(value, span.clone()))
        .process(stream)
}

pub fn statement(stream: Input) -> IResult<Statement> {
    return_statement
        .or(
            preceded(TokenType::Keyword("continue".to_string()), expression.opt())
                .map(Statement::Continue),
        )
        .or(
            preceded(TokenType::Keyword("break".to_string()), expression.opt())
                .map(Statement::Break),
        )
        .or(assignment.map(Statement::Assignment))
        .or(expression.map(Statement::Expression))
        .process(stream)
        .map_err(|e| e.with_context("statement"))
}

pub fn assignment(stream: Input) -> IResult<Assignment> {
    (assignment_lhs, TokenType::Equal, expression)
        .map(|(lhs, _, rhs)| Assignment { lhs, rhs })
        .process(stream)
        .map_err(|e| e.with_context("assignment"))
}

pub fn assignment_lhs(stream: Input) -> IResult<AssignmentLHS> {
    followed(
        (pattern, preceded(TokenType::Colon, parse_type).opt()),
        seek(TokenType::Equal),
    )
    .map(|(pattern, type_annotation)| AssignmentLHS::Pattern {
        pattern,
        type_annotation,
    })
    .or(followed(unary_expr, seek(TokenType::Equal)).map(AssignmentLHS::Expression))
    .process(stream)
}

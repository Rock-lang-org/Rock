use crate::{
    lexer::TokenType,
    parser::{engine::*, Array, Expression},
};

use super::{empty_lines, expression, indent};

pub fn repeat_array(stream: Input) -> IResult<(Expression, usize)> {
    let (stream, _) = TokenType::OpenBracket.process(stream)?;
    let mut bracket_depth = 0;
    let mut paren_depth = 0;
    let mut separator = None;
    let mut lookahead = stream;
    let mut index = 0;
    while !lookahead.is_empty() {
        let (next, token) = lookahead.consume()?;
        match &token.token_type {
            TokenType::OpenBracket => bracket_depth += 1,
            TokenType::CloseBracket if bracket_depth > 0 => bracket_depth -= 1,
            TokenType::CloseBracket if paren_depth == 0 => break,
            TokenType::OpenParen => paren_depth += 1,
            TokenType::CloseParen if paren_depth > 0 => paren_depth -= 1,
            TokenType::Operator(name) | TokenType::StuckOperator(name)
                if name == ";" && bracket_depth == 0 && paren_depth == 0 =>
            {
                separator = Some(index);
                break;
            }
            _ => {}
        }
        lookahead = next;
        index += 1;
    }
    let Some(separator) = separator else {
        return Err(ParseError::Fail);
    };

    let value_stream = stream.prefix(separator)?;
    let (value_rest, value) = expression(value_stream)?;
    if !value_rest.is_empty() {
        return Err(ParseError::Fail);
    }

    let separator_stream = value_rest.resume_input_from(stream.advance(separator)?);
    let (stream, _) = TokenType::Operator(";".to_string())
        .or(TokenType::StuckOperator(";".to_string()))
        .process(separator_stream)?;
    let (stream, len) = super::primitives::int(stream)?;
    let (stream, _) = TokenType::CloseBracket.process(stream)?;
    Ok((stream, (value, len as usize)))
}

pub fn multiline_array(stream: Input) -> IResult<Vec<Expression>> {
    indented(preceded(
        TokenType::Eol,
        preceded(
            empty_lines,
            separated_trailing(
                preceded(indent, separated1(expression, TokenType::Coma)),
                (
                    TokenType::Coma.opt(),
                    TokenType::Eol.followed_by(empty_lines),
                ),
            ),
        )
        .map(|elements| elements.into_iter().flatten().collect::<Vec<_>>()),
    ))
    .followed_by(indent)
    .process(stream)
}

pub fn monoline_array(stream: Input) -> IResult<Vec<Expression>> {
    separated_trailing(expression, TokenType::Coma).process(stream)
}

pub fn array(stream: Input) -> IResult<Array> {
    delimited(
        TokenType::OpenBracket,
        multiline_array.or(monoline_array),
        TokenType::CloseBracket,
    )
    .map(|elements| Array { elements })
    .process(stream)
    .map_err(|e| e.with_context("array"))
}

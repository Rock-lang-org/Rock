use crate::{
    lexer::TokenType,
    ast::ParseArrayLen,
    parser::{engine::*, Array, Expression},
};

use super::{empty_lines, expression, indent};

pub fn repeat_array(stream: Input) -> IResult<(Expression, ParseArrayLen)> {
    let (stream, _) = TokenType::OpenBracket.process(stream)?;
    let mut bracket_depth = 0;
    let mut paren_depth = 0;
    let mut separator = None;
    for (index, token) in stream.tokens.iter().enumerate() {
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
    }
    let Some(separator) = separator else {
        return Err(ParseError::Fail);
    };

    let value_stream = Input {
        tokens: &stream.tokens[..separator],
        ..stream
    };
    let (value_rest, value) = expression(value_stream)?;
    if !value_rest.tokens.is_empty() {
        return Err(ParseError::Fail);
    }

    let separator_stream = Input {
        tokens: &stream.tokens[separator..],
        ..value_rest
    };
    let (stream, _) = TokenType::Operator(";".to_string())
        .or(TokenType::StuckOperator(";".to_string()))
        .process(separator_stream)?;
    let (stream, len) = super::parse_type::parse_array_len(stream)?;
    let (stream, _) = TokenType::CloseBracket.process(stream)?;
    Ok((stream, (value, len)))
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

use crate::lexer::{Lexer, Token, TokenType};
use crate::parser::engine::{IResult, ParseCtx};
use crate::Config;
use std::path::PathBuf;

/// Helper function to create tokens from input string
pub fn lex_test(input: &str) -> Vec<Token> {
    let mut tokens = Lexer::new(PathBuf::from("/test.rk"), input)
        .unwrap()
        .with_newline_at_end(false)
        .collect()
        .unwrap();

    // Remove indent token
    tokens.remove(0);

    // Remove EOF token
    tokens.pop();

    tokens
}

/// Helper function to create a ParseCtx from input string
pub fn make_ctx(input: &str) -> ParseCtx<'static> {
    let tokens = Box::leak(Box::new(lex_test(input)));
    let config = Box::leak(Box::new(Config::default()));
    let eof_path = Box::leak(Box::new(crate::lexer::SourcePath::from("/test.rk")));
    let mut context = ParseCtx::from(tokens, config);
    context.eof_location = Some((eof_path, input.len()));
    context
}

/// Helper to create a parser that matches an identifier
pub fn ident_parser() -> impl FnMut(ParseCtx) -> IResult<TokenType> {
    |ctx: ParseCtx| {
        let (ctx, token) = ctx.consume()?;
        match token.token_type {
            TokenType::Ident(_) => Ok((ctx, token.token_type)),
            _ => Err(crate::parser::engine::ParseError::UnexpectedToken(
                "Ident".to_string(),
                token,
            )),
        }
    }
}

/// Helper to create a parser that matches a number
pub fn number_parser() -> impl FnMut(ParseCtx) -> IResult<TokenType> {
    |ctx: ParseCtx| {
        let (ctx, token) = ctx.consume()?;
        match token.token_type {
            TokenType::Number(_) => Ok((ctx, token.token_type)),
            _ => Err(crate::parser::engine::ParseError::UnexpectedToken(
                "Number".to_string(),
                token,
            )),
        }
    }
}

/// Helper to create a parser that matches a string
pub fn string_parser() -> impl FnMut(ParseCtx) -> IResult<TokenType> {
    |ctx: ParseCtx| {
        let (ctx, token) = ctx.consume()?;
        match token.token_type {
            TokenType::String(_) => Ok((ctx, token.token_type)),
            _ => Err(crate::parser::engine::ParseError::UnexpectedToken(
                "String".to_string(),
                token,
            )),
        }
    }
}

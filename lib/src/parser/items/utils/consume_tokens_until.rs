use crate::lexer::Token;
use crate::lexer::TokenType;
use crate::parser::engine::*;

pub fn consume_tokens_until(token_type: TokenType) -> impl Fn(Input) -> IResult<Vec<Token>> {
    move |stream: Input| {
        let mut tokens = Vec::new();
        let mut remaining_tokens = stream.tokens;

        while let Some(token) = remaining_tokens.first() {
            if token.token_type == token_type {
                return Ok((
                    Input {
                        tokens: remaining_tokens,
                        ..stream
                    },
                    tokens,
                ));
            }

            tokens.push(token.clone());
            remaining_tokens = &remaining_tokens[1..];
        }

        Ok((
            Input {
                tokens: remaining_tokens,
                ..stream
            },
            tokens,
        ))
    }
}

/// Consumes tokens up to the `)` that closes the current parenthesis,
/// skipping over nested `( ... )` pairs.
pub fn consume_tokens_until_matching_close_paren(stream: Input) -> IResult<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut remaining_tokens = stream.tokens;
    let mut depth = 0usize;

    while let Some(token) = remaining_tokens.first() {
        match token.token_type {
            TokenType::OpenParen => depth += 1,
            TokenType::CloseParen if depth == 0 => break,
            TokenType::CloseParen => depth -= 1,
            _ => {}
        }

        tokens.push(token.clone());
        remaining_tokens = &remaining_tokens[1..];
    }

    Ok((
        Input {
            tokens: remaining_tokens,
            ..stream
        },
        tokens,
    ))
}

use crate::lexer::Token;
use crate::lexer::TokenType;
use crate::parser::engine::*;

pub fn consume_tokens_until(token_type: TokenType) -> impl Fn(Input) -> IResult<Vec<Token>> {
    move |stream: Input| {
        let mut tokens = Vec::new();
        let mut remaining = stream;

        while !remaining.is_empty() {
            let token = remaining.seek()?;
            if token.token_type == token_type {
                return Ok((remaining, tokens));
            }

            tokens.push(token);
            remaining = remaining.advance(1)?;
        }

        Ok((remaining, tokens))
    }
}

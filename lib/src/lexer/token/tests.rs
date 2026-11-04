use super::*;

impl From<TokenType> for Token {
    fn from(token_type: TokenType) -> Self {
        Self {
            token_type,
            span: Span::test(),
        }
    }
}

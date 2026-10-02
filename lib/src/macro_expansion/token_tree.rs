use crate::lexer::{Span, Token, TokenType};
use crate::macro_expansion::{ExpansionId, GeneratedSourceId, TokenOrigin};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delimiter {
    Paren,
    Bracket,
    Brace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenTree {
    Leaf {
        token: Token,
        origin: TokenOrigin,
    },
    Delimited {
        delimiter: Delimiter,
        open_origin: TokenOrigin,
        inner: TokenStream,
        close_origin: TokenOrigin,
    },
}

impl TokenTree {
    pub fn origin(&self) -> Option<&TokenOrigin> {
        match self {
            TokenTree::Leaf { origin, .. } => Some(origin),
            TokenTree::Delimited { open_origin, .. } => Some(open_origin),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TokenStream {
    pub trees: Vec<TokenTree>,
}

impl TokenStream {
    pub fn from_tokens(tokens: Vec<Token>) -> Self {
        Self {
            trees: tokens
                .into_iter()
                .map(|token| TokenTree::Leaf {
                    origin: TokenOrigin::Source(token.span.clone()),
                    token,
                })
                .collect(),
        }
    }

    pub fn captured_from_tokens(
        tokens: Vec<Token>,
        expansion: ExpansionId,
        invocation_span: Span,
    ) -> Self {
        Self {
            trees: tokens
                .into_iter()
                .map(|token| TokenTree::Leaf {
                    origin: TokenOrigin::Captured {
                        expansion,
                        invocation_span: invocation_span.clone(),
                        capture_span: token.span.clone(),
                    },
                    token,
                })
                .collect(),
        }
    }

    pub fn generated_from_tokens(
        tokens: Vec<Token>,
        expansion: ExpansionId,
        generated_source: GeneratedSourceId,
    ) -> Self {
        Self {
            trees: tokens
                .into_iter()
                .map(|mut token| {
                    token.span.file_path = format!("<macro-expansion:{}>", expansion.0).into();
                    TokenTree::Leaf {
                        origin: TokenOrigin::Generated {
                            expansion,
                            generated_source,
                            definition_span: token.span.clone(),
                        },
                        token,
                    }
                })
                .collect(),
        }
    }

    pub fn to_tokens(&self) -> Vec<Token> {
        self.to_tokens_with_origins()
            .into_iter()
            .map(|(token, _)| token)
            .collect()
    }

    pub fn to_tokens_with_origins(&self) -> Vec<(Token, TokenOrigin)> {
        let mut tokens = Vec::new();
        for tree in &self.trees {
            match tree {
                TokenTree::Leaf { token, origin } => tokens.push((token.clone(), origin.clone())),
                TokenTree::Delimited {
                    delimiter,
                    open_origin,
                    inner,
                    close_origin,
                } => {
                    match delimiter {
                        Delimiter::Paren => tokens.push((
                            token_with_origin_span(TokenType::OpenParen, open_origin),
                            open_origin.clone(),
                        )),
                        Delimiter::Bracket => tokens.push((
                            token_with_origin_span(TokenType::OpenBracket, open_origin),
                            open_origin.clone(),
                        )),
                        Delimiter::Brace => tokens.push((
                            token_with_origin_span(TokenType::OpenBrace, open_origin),
                            open_origin.clone(),
                        )),
                    }
                    tokens.extend(inner.to_tokens_with_origins());
                    match delimiter {
                        Delimiter::Paren => tokens.push((
                            token_with_origin_span(TokenType::CloseParen, close_origin),
                            close_origin.clone(),
                        )),
                        Delimiter::Bracket => tokens.push((
                            token_with_origin_span(TokenType::CloseBracket, close_origin),
                            close_origin.clone(),
                        )),
                        Delimiter::Brace => tokens.push((
                            token_with_origin_span(TokenType::CloseBrace, close_origin),
                            close_origin.clone(),
                        )),
                    }
                }
            }
        }
        tokens
    }
}

fn token_with_origin_span(token_type: TokenType, origin: &TokenOrigin) -> Token {
    Token {
        token_type,
        span: origin_span(origin),
    }
}

fn origin_span(origin: &TokenOrigin) -> Span {
    match origin {
        TokenOrigin::Source(span) => span.clone(),
        TokenOrigin::Captured { capture_span, .. } => capture_span.clone(),
        TokenOrigin::Generated {
            definition_span, ..
        } => definition_span.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::lexer::{Span, Token, TokenType};
    use crate::macro_expansion::TokenOrigin;

    #[test]
    fn token_stream_preserves_leaf_tokens_and_origins() {
        let span = Span {
            file_path: "tokens.rk".into(),
            start: 0,
            end: 1,
        };
        let token = Token {
            token_type: TokenType::Ident("x".to_string()),
            span: span.clone(),
        };

        let stream = TokenStream::from_tokens(vec![token.clone()]);

        assert_eq!(stream.to_tokens(), vec![token]);
        assert_eq!(stream.trees[0].origin(), Some(&TokenOrigin::Source(span)));
    }

    #[test]
    fn delimited_token_tree_flattens_with_delimiters() {
        let stream = TokenStream {
            trees: vec![TokenTree::Delimited {
                delimiter: Delimiter::Paren,
                open_origin: TokenOrigin::Source(Span::test()),
                inner: TokenStream::from_tokens(vec![Token::from(TokenType::Ident(
                    "x".to_string(),
                ))]),
                close_origin: TokenOrigin::Source(Span::test()),
            }],
        };

        let tokens = stream.to_tokens();

        assert!(matches!(tokens[0].token_type, TokenType::OpenParen));
        assert!(matches!(tokens[1].token_type, TokenType::Ident(_)));
        assert!(matches!(tokens[2].token_type, TokenType::CloseParen));
    }

    #[test]
    fn delimited_token_tree_flattens_delimiters_with_source_origin_spans() {
        let open_span = Span {
            file_path: "delimited.rk".into(),
            start: 4,
            end: 5,
        };
        let close_span = Span {
            file_path: "delimited.rk".into(),
            start: 8,
            end: 9,
        };
        let stream = TokenStream {
            trees: vec![TokenTree::Delimited {
                delimiter: Delimiter::Paren,
                open_origin: TokenOrigin::Source(open_span.clone()),
                inner: TokenStream::from_tokens(vec![Token::from(TokenType::Ident(
                    "x".to_string(),
                ))]),
                close_origin: TokenOrigin::Source(close_span.clone()),
            }],
        };

        let tokens = stream.to_tokens();

        assert_eq!(tokens[0].span, open_span);
        assert_eq!(tokens[2].span, close_span);
    }

    #[test]
    fn token_stream_flattens_tokens_with_origins_in_order() {
        let open_span = Span {
            file_path: "origins.rk".into(),
            start: 0,
            end: 1,
        };
        let inner_span = Span {
            file_path: "origins.rk".into(),
            start: 1,
            end: 2,
        };
        let close_span = Span {
            file_path: "origins.rk".into(),
            start: 2,
            end: 3,
        };
        let inner_token = Token {
            token_type: TokenType::Ident("x".to_string()),
            span: inner_span.clone(),
        };
        let stream = TokenStream {
            trees: vec![TokenTree::Delimited {
                delimiter: Delimiter::Paren,
                open_origin: TokenOrigin::Source(open_span.clone()),
                inner: TokenStream::from_tokens(vec![inner_token]),
                close_origin: TokenOrigin::Source(close_span.clone()),
            }],
        };

        let tokens = stream.to_tokens_with_origins();

        assert!(matches!(tokens[0].0.token_type, TokenType::OpenParen));
        assert_eq!(tokens[0].1, TokenOrigin::Source(open_span));
        assert!(matches!(tokens[1].0.token_type, TokenType::Ident(_)));
        assert_eq!(tokens[1].1, TokenOrigin::Source(inner_span));
        assert!(matches!(tokens[2].0.token_type, TokenType::CloseParen));
        assert_eq!(tokens[2].1, TokenOrigin::Source(close_span));
    }
}

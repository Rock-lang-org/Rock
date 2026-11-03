use crate::lexer::{Span, Token, TokenType};
use crate::macro_expansion::{ExpansionId, GeneratedSourceId, TokenOrigin};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delimiter {
    Paren,
    Bracket,
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
mod tests;

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
            inner: TokenStream::from_tokens(vec![Token::from(TokenType::Ident("x".to_string()))]),
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
            inner: TokenStream::from_tokens(vec![Token::from(TokenType::Ident("x".to_string()))]),
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

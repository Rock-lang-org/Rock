use super::*;

use crate::ast::{Ident, MacroDecl, MacroEntry, MacroFragment};
use crate::lexer::{Span, Token, TokenType};
use crate::macro_expansion::{ExpansionId, GeneratedSourceId, TokenOrigin, TokenStream};

fn ident(name: &str) -> Ident {
    Ident {
        name: name.to_string(),
        span: Span::test(),
    }
}

fn span(start: usize, end: usize) -> Span {
    Span {
        file_path: "declarative_test.rk".into(),
        start,
        end,
    }
}

#[test]
fn declarative_macro_compiles_capture_kinds_and_template_tokens() {
    let decl = MacroDecl {
        name: ident("make"),
        entries: vec![MacroEntry {
            defs: vec![MacroFragment::Ident(ident("name"))],
            body: vec![
                MacroFragment::Ident(ident("name")),
                MacroFragment::Token(Token::from(TokenType::Equal)),
            ],
        }],
        exported: false,
    };

    let compiled = DeclarativeMacro::from_ast(&decl);

    assert_eq!(compiled.name, "make");
    assert_eq!(compiled.arms.len(), 1);
    assert!(matches!(
        compiled.arms[0].matcher.fragments[0],
        MatcherFragment::Capture {
            kind: CaptureKind::Ident,
            ..
        }
    ));
    assert!(matches!(
        compiled.arms[0].template.fragments[0],
        TemplateFragment::Capture { .. }
    ));
}

#[test]
fn declarative_template_expands_direct_capture_tokens() {
    let mut captures = CaptureSet::new();
    captures.insert_direct(
        "name".to_string(),
        TokenStream::from_tokens(vec![Token::from(TokenType::Ident("main".to_string()))]),
    );
    let template = MacroTemplate {
        fragments: vec![
            TemplateFragment::Capture {
                name: "name".to_string(),
            },
            TemplateFragment::Token(Token::from(TokenType::Equal)),
        ],
    };

    let expanded = template.expand(&captures).unwrap();

    assert!(matches!(
        expanded.to_tokens()[0].token_type,
        TokenType::Ident(_)
    ));
    assert!(matches!(
        expanded.to_tokens()[1].token_type,
        TokenType::Equal
    ));
}

#[test]
fn declarative_template_marks_captured_and_generated_origins() {
    let expansion = ExpansionId(9);
    let generated_source = GeneratedSourceId(4);
    let invocation_span = span(1, 7);
    let capture_span = span(8, 12);
    let template_span = span(20, 21);
    let mut captures = CaptureSet::new();
    captures.insert_direct(
        "name".to_string(),
        TokenStream::captured_from_tokens(
            vec![Token {
                token_type: TokenType::Ident("main".to_string()),
                span: capture_span.clone(),
            }],
            expansion,
            invocation_span.clone(),
        ),
    );
    let template = MacroTemplate {
        fragments: vec![
            TemplateFragment::Capture {
                name: "name".to_string(),
            },
            TemplateFragment::Token(Token {
                token_type: TokenType::Equal,
                span: template_span.clone(),
            }),
        ],
    };

    let expanded = template
        .expand_with_origins(&captures, expansion, generated_source)
        .unwrap();
    let tokens = expanded.to_tokens_with_origins();

    match &tokens[0].1 {
        TokenOrigin::Captured {
            expansion: id,
            invocation_span: span,
            capture_span: captured,
        } => {
            assert_eq!(*id, expansion);
            assert_eq!(
                (span.start, span.end),
                (invocation_span.start, invocation_span.end)
            );
            assert_eq!(
                (captured.start, captured.end),
                (capture_span.start, capture_span.end)
            );
        }
        origin => panic!("expected captured origin, got {origin:?}"),
    }
    match &tokens[1].1 {
        TokenOrigin::Generated {
            expansion: id,
            generated_source: source,
            definition_span: span,
        } => {
            assert_eq!(*id, expansion);
            assert_eq!(*source, generated_source);
            assert_eq!(
                (span.start, span.end),
                (template_span.start, template_span.end)
            );
        }
        origin => panic!("expected generated origin, got {origin:?}"),
    }
}

#[test]
fn declarative_template_expands_repeated_capture_groups() {
    let mut captures = CaptureSet::new();
    captures.insert_repeated(
        "name".to_string(),
        vec![
            TokenStream::from_tokens(vec![Token::from(TokenType::Ident("one".to_string()))]),
            TokenStream::from_tokens(vec![Token::from(TokenType::Ident("two".to_string()))]),
        ],
    );
    let template = MacroTemplate {
        fragments: vec![TemplateFragment::Repetition {
            fragments: vec![
                TemplateFragment::Capture {
                    name: "name".to_string(),
                },
                TemplateFragment::Token(Token::from(TokenType::Equal)),
            ],
            line_indent: None,
        }],
    };

    let expanded = template.expand(&captures).unwrap().to_tokens();

    assert_eq!(expanded.len(), 4);
    assert!(matches!(
        expanded[0].token_type,
        TokenType::Ident(ref name) if name == "one"
    ));
    assert!(matches!(expanded[1].token_type, TokenType::Equal));
    assert!(matches!(
        expanded[2].token_type,
        TokenType::Ident(ref name) if name == "two"
    ));
    assert!(matches!(expanded[3].token_type, TokenType::Equal));
}

#[test]
fn declarative_template_repeats_direct_capture_with_repeated_capture_groups() {
    let mut captures = CaptureSet::new();
    captures.insert_direct(
        "ret".to_string(),
        TokenStream::from_tokens(vec![Token::from(TokenType::Ident("value".to_string()))]),
    );
    captures.insert_repeated(
        "name".to_string(),
        vec![
            TokenStream::from_tokens(vec![Token::from(TokenType::Ident("one".to_string()))]),
            TokenStream::from_tokens(vec![Token::from(TokenType::Ident("two".to_string()))]),
        ],
    );
    let template = MacroTemplate {
        fragments: vec![TemplateFragment::Repetition {
            fragments: vec![
                TemplateFragment::Capture {
                    name: "name".to_string(),
                },
                TemplateFragment::Token(Token::from(TokenType::Equal)),
                TemplateFragment::Token(Token::from(TokenType::Arrow)),
                TemplateFragment::Capture {
                    name: "ret".to_string(),
                },
            ],
            line_indent: None,
        }],
    };

    let expanded = template.expand(&captures).unwrap().to_tokens();

    assert_eq!(expanded.len(), 8);
    assert!(matches!(
        expanded[0].token_type,
        TokenType::Ident(ref name) if name == "one"
    ));
    assert!(matches!(expanded[1].token_type, TokenType::Equal));
    assert!(matches!(expanded[2].token_type, TokenType::Arrow));
    assert!(matches!(
        expanded[3].token_type,
        TokenType::Ident(ref name) if name == "value"
    ));
    assert!(matches!(
        expanded[4].token_type,
        TokenType::Ident(ref name) if name == "two"
    ));
    assert!(matches!(expanded[5].token_type, TokenType::Equal));
    assert!(matches!(expanded[6].token_type, TokenType::Arrow));
    assert!(matches!(
        expanded[7].token_type,
        TokenType::Ident(ref name) if name == "value"
    ));
}

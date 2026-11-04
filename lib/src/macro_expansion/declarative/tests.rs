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

fn expand_template_fragments(
    fragments: &[TemplateFragment],
    captures: &CaptureSet,
    repeated_index: Option<usize>,
) -> Result<TokenStream, String> {
    let mut trees = Vec::new();
    for fragment in fragments {
        match fragment {
            TemplateFragment::Capture { name } => {
                if let Some(index) = repeated_index {
                    if let Some(streams) = captures.get_repeated(name) {
                        let Some(stream) = streams.get(index) else {
                            return Err(format!("missing repeated macro capture '{}'", name));
                        };
                        trees.extend(stream.trees.clone());
                        continue;
                    }
                }

                let Some(streams) = captures.get_direct(name) else {
                    return Err(format!("missing macro capture '{}'", name));
                };
                for stream in streams {
                    trees.extend(stream.trees.clone());
                }
            }
            TemplateFragment::Token(token) => {
                trees.extend(TokenStream::from_tokens(vec![token.clone()]).trees);
            }
            TemplateFragment::Repetition {
                fragments: inner,
                line_indent,
            } => {
                let capture_names = capture_names_in_template(inner);
                let mut len = None;

                for name in &capture_names {
                    if let Some(streams) = captures.get_repeated(name) {
                        match len {
                            Some(len) if streams.len() != len => {
                                return Err(format!(
                                    "mismatched repeated macro capture lengths for '{}'",
                                    name
                                ));
                            }
                            None => len = Some(streams.len()),
                            _ => {}
                        }
                    };
                }

                let Some(len) = len else { continue };

                for index in 0..len {
                    let expanded = expand_template_fragments(inner, captures, Some(index))?;
                    trees.extend(expanded.trees);
                    if let (Some(indent), true) = (line_indent, index + 1 < len) {
                        trees.extend(TokenStream::from_tokens(line_break_tokens(indent)).trees);
                    }
                }
            }
        }
    }
    Ok(TokenStream { trees })
}

impl MacroTemplate {
    pub fn expand(&self, captures: &CaptureSet) -> Result<TokenStream, String> {
        expand_template_fragments(&self.fragments, captures, None)
    }
}

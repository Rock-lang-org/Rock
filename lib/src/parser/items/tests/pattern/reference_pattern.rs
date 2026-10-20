use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn reference_pattern_immutable() {
    let input = "&a";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, pattern) = pattern.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(
        pattern,
        Pattern {
            binding: None,
            kind: PatternKind::Reference {
                pattern: Box::new(Pattern {
                    binding: None,
                    kind: PatternKind::Ident(IdentPattern {
                        name: Ident { context: Default::default(),
                            name: "a".to_string(),
                            span: Span::test()
                        },
                        mut_: false,
                    })
                }),
                mutable: false,
            }
        }
    );

    assert_eq!(rest.len(), 0);
}

#[test]
fn reference_pattern_mutable() {
    let input = "&mut a";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, pattern) = pattern.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(
        pattern,
        Pattern {
            binding: None,
            kind: PatternKind::Reference {
                pattern: Box::new(Pattern {
                    binding: None,
                    kind: PatternKind::Ident(IdentPattern {
                        name: Ident { context: Default::default(),
                            name: "a".to_string(),
                            span: Span::test()
                        },
                        mut_: false,
                    })
                }),
                mutable: true,
            }
        }
    );

    assert_eq!(rest.len(), 0);
}

#[test]
fn reference_pattern_mutable_caret() {
    let input = "&^ a";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, pattern) = pattern.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(
        pattern,
        Pattern {
            binding: None,
            kind: PatternKind::Reference {
                pattern: Box::new(Pattern {
                    binding: None,
                    kind: PatternKind::Ident(IdentPattern {
                        name: Ident { context: Default::default(),
                            name: "a".to_string(),
                            span: Span::test()
                        },
                        mut_: false,
                    })
                }),
                mutable: true,
            }
        }
    );

    assert_eq!(rest.len(), 0);
}

#[test]
fn reference_pattern_nested_tuple() {
    let input = "&(a, b)";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, pattern) = pattern.process(ParseCtx::from(&tokens, &config)).unwrap();

    match pattern.kind {
        PatternKind::Reference {
            mutable,
            pattern: inner,
        } => {
            assert!(!mutable);
            match inner.kind {
                PatternKind::Tuple(patterns) => {
                    assert_eq!(patterns.len(), 2);
                }
                _ => panic!(
                    "Expected tuple pattern inside reference, got {:?}",
                    inner.kind
                ),
            }
        }
        _ => panic!("Expected reference pattern, got {:?}", pattern.kind),
    }

    assert_eq!(rest.len(), 0);
}

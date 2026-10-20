use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::tests::common::assert_formatted_eq;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn instance_pattern_fields_nested() {
    let input = "Player foo: (Ok 1), bar: toto";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, pattern) = pattern.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_formatted_eq(
        &pattern,
        &Pattern {
            binding: None,
            kind: PatternKind::Instance(InstancePattern {
                name: TypePath {
                    path: vec![IdentOrType::Type(ParseType::Type(ParseTypeInner { context: Default::default(),
                        name: "Player".to_string(),
                        generics: vec![],
                        span: Span::test(),
                    }))],
                },
                args: FieldsPatternOrArgumentsPattern::Fields(vec![
                    FieldPattern {
                        name: Ident { context: Default::default(),
                            name: "foo".to_string(),
                            span: Span::test(),
                        },
                        pattern: Pattern {
                            binding: None,
                            kind: PatternKind::Nested(Box::new(Pattern {
                                binding: None,
                                kind: PatternKind::Instance(InstancePattern {
                                    name: TypePath {
                                        path: vec![IdentOrType::Type(ParseType::Type(
                                            ParseTypeInner { context: Default::default(),
                                                name: "Ok".to_string(),
                                                generics: vec![],
                                                span: Span::test(),
                                            },
                                        ))],
                                    },
                                    args: FieldsPatternOrArgumentsPattern::Arguments(vec![
                                        Pattern {
                                            binding: None,
                                            kind: PatternKind::Literal(Literal {
                                                kind: LiteralKind::Number(1),
                                                span: Span::test(),
                                            }),
                                        },
                                    ]),
                                }),
                            })),
                        },
                    },
                    FieldPattern {
                        name: Ident { context: Default::default(),
                            name: "bar".to_string(),
                            span: Span::test(),
                        },
                        pattern: Pattern {
                            binding: None,
                            kind: PatternKind::Ident(IdentPattern {
                                name: Ident { context: Default::default(),
                                    name: "toto".to_string(),
                                    span: Span::test(),
                                },
                                mut_: false,
                            }),
                        },
                    },
                ]),
            }),
        },
    );

    assert_eq!(rest.len(), 0);
}

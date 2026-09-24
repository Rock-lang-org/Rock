use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::tests::common::assert_formatted_eq;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;
use std::path::PathBuf;

#[test]
fn test_parse_match() {
    let input = r#"match a
    a => 2
    (a, b) => a + b"#;
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = r#match.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_formatted_eq(
        &expression,
        &Match {
            do_syntax: None,
            expr: Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                operand: Operand::Ident(crate::ast::IdentifierPath {
                    path: vec![IdentOrType::Ident(Ident {
                        name: "a".to_string(),
                        span: Span {
                            start: 6,
                            end: 7,
                            file_path: PathBuf::from("/test.rk"),
                        },
                    })],
                }),
                secondaries: None,
                type_annotation: None,
            })),
            arms: vec![
                MatchArm {
                    pattern: Pattern {
                        binding: None,
                        kind: PatternKind::Ident(IdentPattern {
                            name: Ident {
                                name: "a".to_string(),
                                span: Span {
                                    start: 12,
                                    end: 13,
                                    file_path: PathBuf::from("/test.rk"),
                                },
                            },
                            mut_: false,
                        }),
                    },
                    condition: None,
                    body: Block {
                        statements: vec![Statement::Expression(Expression::UnaryExpr(
                            UnaryExpr::PrimaryExpr(PrimaryExpr {
                                operand: Operand::Literal(Literal {
                                    kind: LiteralKind::Number(2),
                                    span: Span {
                                        start: 17,
                                        end: 18,
                                        file_path: PathBuf::from("/test.rk"),
                                    },
                                }),
                                secondaries: None,
                                type_annotation: None,
                            }),
                        ))],
                    },
                },
                MatchArm {
                    pattern: Pattern {
                        binding: None,
                        kind: PatternKind::Tuple(vec![
                            Pattern {
                                binding: None,
                                kind: PatternKind::Ident(IdentPattern {
                                    name: Ident {
                                        name: "a".to_string(),
                                        span: Span {
                                            start: 24,
                                            end: 25,
                                            file_path: PathBuf::from("/test.rk"),
                                        },
                                    },
                                    mut_: false,
                                }),
                            },
                            Pattern {
                                binding: None,
                                kind: PatternKind::Ident(IdentPattern {
                                    name: Ident {
                                        name: "b".to_string(),
                                        span: Span {
                                            start: 27,
                                            end: 28,
                                            file_path: PathBuf::from("/test.rk"),
                                        },
                                    },
                                    mut_: false,
                                }),
                            },
                        ]),
                    },
                    condition: None,
                    body: Block {
                        statements: vec![Statement::Expression(Expression::BinopExpr(
                            UnaryExpr::PrimaryExpr(PrimaryExpr {
                                operand: Operand::Ident(crate::ast::IdentifierPath {
                                    path: vec![IdentOrType::Ident(Ident {
                                        name: "a".to_string(),
                                        span: Span::test(),
                                    })],
                                }),
                                secondaries: None,
                                type_annotation: None,
                            }),
                            Operator {
                                value: "+".to_string(),
                                span: Span::test(),
                            },
                            Box::new(Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                                operand: Operand::Ident(crate::ast::IdentifierPath {
                                    path: vec![IdentOrType::Ident(Ident {
                                        name: "b".to_string(),
                                        span: Span::test(),
                                    })],
                                }),
                                secondaries: None,
                                type_annotation: None,
                            }))),
                        ))],
                    },
                },
            ],
        },
    );

    assert_eq!(rest.len(), 0);
}

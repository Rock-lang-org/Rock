use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_match_with_condition() {
    let input = r#"match a
    (a, b) if a => a"#;
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = r#match.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(
        expression,
        Match {
            do_syntax: None,
            expr: Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                operand: Operand::Ident(crate::ast::IdentifierPath {
                    path: vec![IdentOrType::Ident(Ident {
                        name: "a".to_string(),
                        span: Span::test(),
                    })]
                }),
                secondaries: None,
                type_annotation: None,
            })),
            arms: vec![MatchArm {
                pattern: Pattern {
                    binding: None,
                    kind: PatternKind::Tuple(vec![
                        Pattern {
                            binding: None,
                            kind: PatternKind::Ident(IdentPattern {
                                name: Ident {
                                    name: "a".to_string(),
                                    span: Span::test(),
                                },
                                mut_: false,
                            })
                        },
                        Pattern {
                            binding: None,
                            kind: PatternKind::Ident(IdentPattern {
                                name: Ident {
                                    name: "b".to_string(),
                                    span: Span::test(),
                                },
                                mut_: false,
                            })
                        }
                    ])
                },
                condition: Some(Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                    operand: Operand::Ident(crate::ast::IdentifierPath {
                        path: vec![IdentOrType::Ident(Ident {
                            name: "a".to_string(),
                            span: Span::test(),
                        })]
                    }),
                    secondaries: None,
                    type_annotation: None,
                }))),
                body: Block {
                    statements: vec![Statement::Expression(Expression::UnaryExpr(
                        UnaryExpr::PrimaryExpr(PrimaryExpr {
                            operand: Operand::Ident(crate::ast::IdentifierPath {
                                path: vec![IdentOrType::Ident(Ident {
                                    name: "a".to_string(),
                                    span: Span::test(),
                                })]
                            }),
                            secondaries: None,
                            type_annotation: None,
                        })
                    ))]
                }
            }]
        }
    );

    assert_eq!(rest.len(), 0);
}

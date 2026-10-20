use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn nested_spaced_dot_should_close_fn_call() {
    let input = "foo a, (b .lol) .toto";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_eq!(
        expression,
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Ident(IdentifierPath {
                path: vec![IdentOrType::Ident(Ident { context: Default::default(),
                    name: "foo".to_string(),
                    span: Span::test(),
                })],
            }),
            secondaries: Some(vec![
                SecondaryExpr::Arguments(vec![
                    Argument {
                        arg: Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                            operand: Operand::Ident(IdentifierPath {
                                path: vec![IdentOrType::Ident(Ident { context: Default::default(),
                                    name: "a".to_string(),
                                    span: Span::test(),
                                })],
                            }),
                            secondaries: None,
                            type_annotation: None,
                        })),
                    },
                    Argument {
                        arg: Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                            operand: Operand::Expression(Box::new(Expression::UnaryExpr(
                                UnaryExpr::PrimaryExpr(PrimaryExpr {
                                    operand: Operand::Ident(IdentifierPath {
                                        path: vec![IdentOrType::Ident(Ident { context: Default::default(),
                                            name: "b".to_string(),
                                            span: Span::test(),
                                        })],
                                    }),
                                    secondaries: Some(vec![SecondaryExpr::Dot(
                                        IdentOrNumber::Ident(Ident { context: Default::default(),
                                            name: "lol".to_string(),
                                            span: Span::test(),
                                        })
                                    )]),
                                    type_annotation: None,
                                })
                            ))),
                            secondaries: None,
                            type_annotation: None,
                        })),
                    },
                ]),
                SecondaryExpr::Dot(IdentOrNumber::Ident(Ident { context: Default::default(),
                    name: "toto".to_string(),
                    span: Span::test(),
                })),
            ]),
            type_annotation: None,
        })),
    );
    assert_eq!(rest.len(), 0);
}

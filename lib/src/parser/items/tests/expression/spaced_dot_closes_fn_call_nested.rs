use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn spaced_dot_closes_fn_call_nested() {
    let input = "foo a, b a .bar .baz";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_eq!(
        expression,
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Ident(IdentifierPath {
                path: vec![IdentOrType::Ident(Ident {
                    name: "foo".to_string(),
                    span: Span::test(),
                })],
            }),
            secondaries: Some(vec![
                SecondaryExpr::Arguments(
                    vec![
                        Argument {
                            arg: Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                                operand: Operand::Ident(IdentifierPath {
                                    path: vec![IdentOrType::Ident(Ident {
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
                                operand: Operand::Ident(IdentifierPath {
                                    path: vec![IdentOrType::Ident(Ident {
                                        name: "b".to_string(),
                                        span: Span::test(),
                                    })],
                                }),
                                secondaries: Some(vec![SecondaryExpr::Arguments(
                                    vec![Argument {
                                        arg: Expression::UnaryExpr(UnaryExpr::PrimaryExpr(
                                            PrimaryExpr {
                                                operand: Operand::Ident(IdentifierPath {
                                                    path: vec![IdentOrType::Ident(Ident {
                                                        name: "a".to_string(),
                                                        span: Span::test(),
                                                    })],
                                                }),
                                                secondaries: None,
                                                type_annotation: None,
                                            },
                                        )),
                                    },],
                                    Span::new(
                                        "/test.rk".into(),
                                        input.rfind("a .bar").unwrap(),
                                        input.find(" .bar").unwrap()
                                    )
                                )]),
                                type_annotation: None,
                            })),
                        },
                    ],
                    Span::new("/test.rk".into(), 4, input.find(" .bar").unwrap())
                ),
                SecondaryExpr::Dot(IdentOrNumber::Ident(Ident {
                    name: "bar".to_string(),
                    span: Span::test(),
                })),
                SecondaryExpr::Dot(IdentOrNumber::Ident(Ident {
                    name: "baz".to_string(),
                    span: Span::test(),
                })),
            ]),
            type_annotation: None,
        })),
    );
    assert_eq!(rest.len(), 0);
}

use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn spaced_dot_closes_fn_call() {
    let input = "foo a, b .bar";
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
                                secondaries: None,
                                type_annotation: None,
                            })),
                        },
                    ],
                    Span::new("/test.rk".into(), 4, input.find(" .bar").unwrap())
                ),
                SecondaryExpr::Dot(IdentOrNumber::Ident(Ident {
                    name: "bar".to_string(),
                    span: Span::test(),
                }))
            ]),
            type_annotation: None,
        })),
    );
    assert_eq!(rest.len(), 0);
}

#[test]
fn spaced_dot_closes_fn_call_after_infix_argument() {
    let input = "foo x + 1 .bar";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = expression else {
        panic!("expected function call, got {expression:?}");
    };
    let Some(secondaries) = primary.secondaries else {
        panic!("expected call arguments and dot call");
    };
    let SecondaryExpr::Arguments(arguments, _) = &secondaries[0] else {
        panic!("expected call arguments, got {:?}", secondaries[0]);
    };

    assert!(matches!(arguments[0].arg, Expression::BinopExpr(_, _, _)));
    assert!(matches!(secondaries[1], SecondaryExpr::Dot(_)));
    assert_eq!(rest.len(), 0);
}

#[test]
fn spaced_dot_applies_to_complete_infix_expression() {
    let input = "x + 1 .bar";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = expression else {
        panic!("expected spaced-dot wrapper, got {expression:?}");
    };
    assert!(matches!(primary.operand, Operand::Expression(_)));
    assert!(matches!(
        primary.secondaries.as_deref(),
        Some([SecondaryExpr::Dot(_)])
    ));
    assert_eq!(rest.len(), 0);
}

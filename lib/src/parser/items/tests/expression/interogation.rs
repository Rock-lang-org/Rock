use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::tests::common::assert_formatted_eq;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

fn parse_expr(input: &str) -> Expression {
    let tokens = lex_test(input);
    let config = Config::default();
    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();
    assert_eq!(rest.len(), 0);
    expression
}

fn ident_expr(name: &str, secondaries: Option<Vec<SecondaryExpr>>) -> Expression {
    Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
        operand: Operand::Ident(IdentifierPath {
            path: vec![IdentOrType::Ident(Ident { context: Default::default(),
                name: name.to_string(),
                span: Span::test(),
            })],
        }),
        secondaries,
        type_annotation: None,
    }))
}

#[test]
fn interogation() {
    let input = "foo? bar, baz?";
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
                SecondaryExpr::Interogation,
                SecondaryExpr::Arguments(vec![
                    Argument {
                        arg: Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                            operand: Operand::Ident(IdentifierPath {
                                path: vec![IdentOrType::Ident(Ident { context: Default::default(),
                                    name: "bar".to_string(),
                                    span: Span::test(),
                                })],
                            }),
                            secondaries: None,
                            type_annotation: None,
                        }))
                    },
                    Argument {
                        arg: Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                            operand: Operand::Ident(IdentifierPath {
                                path: vec![IdentOrType::Ident(Ident { context: Default::default(),
                                    name: "baz".to_string(),
                                    span: Span::test(),
                                })],
                            }),
                            secondaries: None,
                            type_annotation: None,
                        }))
                    }
                ]),
                SecondaryExpr::Interogation,
            ]),

            type_annotation: None,
        })),
    );
    assert_eq!(rest.len(), 0);
}

#[test]
fn trailing_interogation_after_inline_call_applies_to_call() {
    assert_eq!(
        parse_expr("foo bar?"),
        ident_expr(
            "foo",
            Some(vec![
                SecondaryExpr::Arguments(vec![Argument {
                    arg: ident_expr("bar", None),
                }]),
                SecondaryExpr::Interogation,
            ]),
        ),
    );
}

#[test]
fn trailing_interogation_after_borrowed_argument_applies_to_call() {
    let expression = parse_expr("foo &bar?");
    let primary = match expression {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) => primary,
        other => panic!("expected call expression, got {other:#?}"),
    };
    let secondaries = primary.secondaries.expect("expected call secondaries");

    assert!(matches!(
        secondaries.as_slice(),
        [SecondaryExpr::Arguments(arguments), SecondaryExpr::Interogation]
            if matches!(
                arguments.as_slice(),
                [Argument {
                    arg: Expression::UnaryExpr(UnaryExpr::UnaryExpr(_, inner)),
                }] if matches!(**inner, UnaryExpr::PrimaryExpr(_))
            )
    ));
}

#[test]
fn trailing_interogation_after_bang_call_applies_to_call() {
    assert_eq!(
        parse_expr("foo!?"),
        ident_expr(
            "foo",
            Some(vec![
                SecondaryExpr::Arguments(vec![]),
                SecondaryExpr::Interogation,
            ]),
        ),
    );
}

#[test]
fn receiver_chain_and_call_interogation_compose() {
    assert_formatted_eq(
        &parse_expr("maybe?.get 0?"),
        &ident_expr(
            "maybe",
            Some(vec![
                SecondaryExpr::Interogation,
                SecondaryExpr::Dot(IdentOrNumber::Ident(Ident { context: Default::default(),
                    name: "get".to_string(),
                    span: Span::test(),
                })),
                SecondaryExpr::Arguments(vec![Argument {
                    arg: Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                        operand: Operand::Literal(Literal {
                            kind: LiteralKind::Number(0),
                            span: Span::test(),
                        }),
                        secondaries: None,
                        type_annotation: None,
                    })),
                }]),
                SecondaryExpr::Interogation,
            ]),
        ),
    );
}

#[test]
fn parenthesized_argument_interogation_stays_inside_argument() {
    assert_eq!(
        parse_expr("foo (bar?)"),
        ident_expr(
            "foo",
            Some(vec![SecondaryExpr::Arguments(vec![Argument {
                arg: Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                    operand: Operand::Expression(Box::new(ident_expr(
                        "bar",
                        Some(vec![SecondaryExpr::Interogation]),
                    ))),
                    secondaries: None,
                    type_annotation: None,
                })),
            }])]),
        ),
    );
}

#[test]
fn parenthesized_callee_interogation_stays_inside_callee() {
    assert_eq!(
        parse_expr("(make_fn?) arg"),
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Expression(Box::new(ident_expr(
                "make_fn",
                Some(vec![SecondaryExpr::Interogation]),
            ))),
            secondaries: Some(vec![SecondaryExpr::Arguments(vec![Argument {
                arg: ident_expr("arg", None),
            }])]),
            type_annotation: None,
        })),
    );
}

use crate::ast::*;
use crate::lexer::Span;
use crate::parser::items::tests::common::assert_formatted_eq;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;
use std::path::PathBuf;

#[test]
fn call_expression() {
    let input = "hello 1, 2, 3";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_formatted_eq(
        &expression,
        &Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Ident(IdentifierPath {
                path: vec![IdentOrType::Ident(Ident { context: Default::default(),
                    name: "hello".to_string(),
                    span: Span {
                        start: 0,
                        end: 5,
                        file_path: PathBuf::from("/test.rk"),
                    },
                })],
            }),
            secondaries: Some(vec![SecondaryExpr::Arguments(vec![
                Argument {
                    arg: Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                        operand: Operand::Literal(Literal {
                            kind: crate::ast::LiteralKind::Number(1),
                            span: Span {
                                start: 6,
                                end: 7,
                                file_path: PathBuf::from("/test.rk"),
                            },
                        }),
                        secondaries: None,
                        type_annotation: None,
                    })),
                },
                Argument {
                    arg: Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                        operand: Operand::Literal(Literal {
                            kind: crate::ast::LiteralKind::Number(2),
                            span: Span {
                                start: 9,
                                end: 10,
                                file_path: PathBuf::from("/test.rk"),
                            },
                        }),
                        secondaries: None,
                        type_annotation: None,
                    })),
                },
                Argument {
                    arg: Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
                        operand: Operand::Literal(Literal {
                            kind: crate::ast::LiteralKind::Number(3),
                            span: Span {
                                start: 12,
                                end: 13,
                                file_path: PathBuf::from("/test.rk"),
                            },
                        }),
                        secondaries: None,
                        type_annotation: None,
                    })),
                },
            ])]),
            type_annotation: None,
        })),
    );

    assert_eq!(rest.len(), 0);
}

#[test]
fn call_expression_accepts_prefix_deref_arguments() {
    let input = "eq *self, *other";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = expression else {
        panic!("expected call expression, got {expression:?}");
    };
    let Some(SecondaryExpr::Arguments(args)) = primary.secondaries.as_ref().and_then(|s| s.first())
    else {
        panic!("expected call arguments, got {primary:?}");
    };

    assert_eq!(args.len(), 2);
    for arg in args {
        let Expression::UnaryExpr(UnaryExpr::UnaryExpr(op, _)) = &arg.arg else {
            panic!("expected prefix deref argument, got {:?}", arg.arg);
        };
        assert_eq!(op.value, "*");
    }
    assert_eq!(rest.len(), 0);
}

#[test]
fn call_expression_accepts_mut_reference_first_argument() {
    let tokens = lex_test("recv &mut buff");
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = expression else {
        panic!("expected call expression, got {expression:?}");
    };
    let Some(SecondaryExpr::Arguments(args)) = primary.secondaries.as_ref().and_then(|s| s.first())
    else {
        panic!("expected call arguments, got {primary:?}");
    };
    let Expression::UnaryExpr(UnaryExpr::UnaryExpr(op, _)) = &args[0].arg else {
        panic!("expected mutable reference argument, got {:?}", args[0].arg);
    };

    assert_eq!(op.value, "&mut");
    assert_eq!(rest.len(), 0);
}

#[test]
fn call_expression_accepts_adjacent_shared_reference_argument() {
    let tokens = lex_test("read &stream");
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = expression else {
        panic!("expected call expression, got {expression:?}");
    };
    let Some(SecondaryExpr::Arguments(args)) = primary.secondaries.as_ref().and_then(|s| s.first())
    else {
        panic!("expected call arguments, got {primary:?}");
    };
    let Expression::UnaryExpr(UnaryExpr::UnaryExpr(op, _)) = &args[0].arg else {
        panic!("expected shared reference argument, got {:?}", args[0].arg);
    };

    assert_eq!(op.value, "&");
    assert_eq!(rest.len(), 0);
}

#[test]
fn cast_in_call_binds_to_argument() {
    let tokens = lex_test("convert 1 as I32");
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = expression else {
        panic!("expected call expression, got {expression:?}");
    };
    let Some(SecondaryExpr::Arguments(args)) = primary.secondaries.as_ref().and_then(|s| s.first())
    else {
        panic!("expected call arguments, got {primary:?}");
    };

    assert!(matches!(args[0].arg, Expression::CastExpr(_, _)));
    assert_eq!(rest.len(), 0);
}

#[test]
fn bare_ampersand_after_expression_remains_infix() {
    let tokens = lex_test("left & right");
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    let Expression::BinopExpr(_, op, _) = expression else {
        panic!("expected binary expression, got {expression:?}");
    };

    assert_eq!(op.value, "&");
    assert_eq!(rest.len(), 0);
}

#[test]
fn call_expression_accepts_argument_holes() {
    let tokens = lex_test("hello 1, _, 3");
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = expression else {
        panic!("expected call expression, got {expression:?}");
    };
    let Some(SecondaryExpr::Arguments(args)) = primary.secondaries.as_ref().and_then(|s| s.first())
    else {
        panic!("expected call arguments, got {primary:?}");
    };

    assert_eq!(args.len(), 3);
    assert!(matches!(
        &args[1].arg,
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::CallHole(_),
            secondaries: None,
            type_annotation: None,
        }))
    ));
    assert_eq!(rest.len(), 0);
}

#[test]
fn multiline_call_expression_accepts_argument_holes() {
    let tokens = lex_test("hello\n    _\n    3");
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = expression else {
        panic!("expected call expression, got {expression:?}");
    };
    let Some(SecondaryExpr::Arguments(args)) = primary.secondaries.as_ref().and_then(|s| s.first())
    else {
        panic!("expected call arguments, got {primary:?}");
    };

    assert_eq!(args.len(), 2);
    assert!(matches!(
        &args[0].arg,
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::CallHole(_),
            secondaries: None,
            type_annotation: None,
        }))
    ));
    assert_eq!(rest.len(), 0);
}

#[test]
fn call_hole_is_rejected_outside_an_argument_list() {
    let tokens = lex_test("_");
    let config = Config::default();

    assert!(expression
        .process(ParseCtx::from(&tokens, &config))
        .is_err());
}

#[test]
fn multiline_wildcard_lambda_is_not_parsed_as_a_call_hole() {
    let tokens = lex_test("fold\n    _ -> 0\n    value -> value");
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = expression else {
        panic!("expected call expression, got {expression:?}");
    };
    let Some(SecondaryExpr::Arguments(args)) = primary.secondaries.as_ref().and_then(|s| s.first())
    else {
        panic!("expected call arguments, got {primary:?}");
    };
    let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
        operand: Operand::LambdaDecl(lambda),
        ..
    })) = &args[0].arg
    else {
        panic!("expected wildcard lambda, got {:?}", args[0].arg);
    };

    assert!(matches!(lambda.parameters[0].kind, PatternKind::Wildcard));
    assert_eq!(args.len(), 2);
    assert_eq!(rest.len(), 0);
}

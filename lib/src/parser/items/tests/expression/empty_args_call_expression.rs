use crate::ast::*;
use crate::lexer::Span;
use crate::new_parser::items::*;
use crate::new_parser::*;
use crate::Config;
use std::path::PathBuf;

/// Test that `run()` (no space) is parsed as a function call with no arguments
#[test]
fn empty_args_call_expression() {
    let input = "run()";
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
                    name: "run".to_string(),
                    span: Span {
                        start: 0,
                        end: 3,
                        file_path: PathBuf::default(),
                    },
                })],
            }),
            secondaries: Some(vec![SecondaryExpr::Arguments(
                vec![],
                Span::new("/test.rk".into(), 3, input.len())
            )]),
            type_annotation: None,
        })),
    );

    assert_eq!(rest.len(), 0);
}

/// Test that `run ()` (with space) is parsed as a function call with `()` (Unit) as an argument
#[test]
fn spaced_unit_arg_call_expression() {
    let input = "run ()";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    // `run ()` should be parsed as `run` with a single argument: the unit value `()`
    match &expression {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) => {
            // Operand should be `run`
            match &primary.operand {
                Operand::Ident(path) => {
                    assert_eq!(path.path.len(), 1);
                    match &path.path[0] {
                        IdentOrType::Ident(ident) => assert_eq!(ident.name, "run"),
                        _ => panic!("Expected Ident, got Type"),
                    }
                }
                _ => panic!("Expected Ident operand"),
            }

            // Should have one secondary: Arguments with one argument (the unit value)
            let secondaries = primary.secondaries.as_ref().expect("Expected secondaries");
            assert_eq!(secondaries.len(), 1);
            match &secondaries[0] {
                SecondaryExpr::Arguments(args, _) => {
                    assert_eq!(
                        args.len(),
                        1,
                        "Expected exactly one argument (the unit value)"
                    );
                    // Verify the argument is a unit value (empty tuple)
                    match &args[0].arg {
                        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(arg_primary)) => {
                            match &arg_primary.operand {
                                Operand::Tuple(tuple) => {
                                    assert!(
                                        tuple.elements.is_empty(),
                                        "Expected empty tuple for unit value"
                                    );
                                }
                                _ => panic!(
                                    "Expected Tuple operand for unit value, got {:?}",
                                    arg_primary.operand
                                ),
                            }
                        }
                        _ => panic!("Expected UnaryExpr with PrimaryExpr for argument"),
                    }
                }
                _ => panic!("Expected Arguments secondary"),
            }
        }
        _ => panic!("Expected UnaryExpr with PrimaryExpr"),
    }

    assert_eq!(rest.len(), 0);
}

/// Test that `run(())` (stuck parens with unit inside) is parsed as a function call with `(())` as an argument
/// Note: `(())` is a parenthesized expression containing the unit value `()`
#[test]
fn stuck_parens_with_unit_arg() {
    let input = "run(())";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    // `run(())` should be parsed as `run` with a single argument: the parenthesized unit value `(())`
    match &expression {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) => {
            // Operand should be `run`
            match &primary.operand {
                Operand::Ident(path) => {
                    assert_eq!(path.path.len(), 1);
                    match &path.path[0] {
                        IdentOrType::Ident(ident) => assert_eq!(ident.name, "run"),
                        _ => panic!("Expected Ident, got Type"),
                    }
                }
                _ => panic!("Expected Ident operand"),
            }

            // Should have one secondary: Arguments with one argument
            let secondaries = primary.secondaries.as_ref().expect("Expected secondaries");
            assert_eq!(secondaries.len(), 1);
            match &secondaries[0] {
                SecondaryExpr::Arguments(args, _) => {
                    assert_eq!(args.len(), 1, "Expected exactly one argument");
                    // The argument `(())` is a parenthesized expression containing unit value
                    match &args[0].arg {
                        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(arg_primary)) => {
                            match &arg_primary.operand {
                                // Direct tuple (for `run ()` case)
                                Operand::Tuple(tuple) => {
                                    assert!(
                                        tuple.elements.is_empty(),
                                        "Expected empty tuple for unit value"
                                    );
                                }
                                // Parenthesized expression containing tuple (for `run(())` case)
                                Operand::Expression(inner_expr) => match inner_expr.as_ref() {
                                    Expression::UnaryExpr(UnaryExpr::PrimaryExpr(
                                        inner_primary,
                                    )) => match &inner_primary.operand {
                                        Operand::Tuple(tuple) => {
                                            assert!(
                                                tuple.elements.is_empty(),
                                                "Expected empty tuple for unit value"
                                            );
                                        }
                                        _ => {
                                            panic!("Expected Tuple inside parenthesized expression")
                                        }
                                    },
                                    _ => panic!(
                                        "Expected PrimaryExpr inside parenthesized expression"
                                    ),
                                },
                                _ => panic!(
                                    "Expected Tuple or Expression operand, got {:?}",
                                    arg_primary.operand
                                ),
                            }
                        }
                        _ => panic!("Expected UnaryExpr with PrimaryExpr for argument"),
                    }
                }
                _ => panic!("Expected Arguments secondary"),
            }
        }
        _ => panic!("Expected UnaryExpr with PrimaryExpr"),
    }

    assert_eq!(rest.len(), 0);
}

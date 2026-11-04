use crate::ast::*;
use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn unary_operator_expression() {
    let input = "-x";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    match expression {
        Expression::UnaryExpr(UnaryExpr::UnaryExpr(op, _)) => {
            assert_eq!(op.value, "-");
        }
        _ => panic!("Expected unary expression, got: {:?}", expression),
    }
    assert_eq!(rest.len(), 0);
}

#[test]
fn unary_operator_mut_reference_expression() {
    let config = Config::default();

    let tokens = lex_test("&mut x");
    let (rest, parsed) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    match parsed {
        Expression::UnaryExpr(UnaryExpr::UnaryExpr(op, _)) => {
            assert_eq!(op.value, "&mut");
        }
        _ => panic!("Expected mutable reference expression, got: {:?}", parsed),
    }
    assert_eq!(rest.len(), 0);

    let tokens = lex_test("&^ x");
    let (rest, parsed) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    match parsed {
        Expression::UnaryExpr(UnaryExpr::UnaryExpr(op, _)) => {
            assert_eq!(op.value, "&mut");
        }
        _ => panic!("Expected mutable reference expression, got: {:?}", parsed),
    }
    assert_eq!(rest.len(), 0);

    let tokens = lex_test("&x");
    let (rest, parsed) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    match parsed {
        Expression::UnaryExpr(UnaryExpr::UnaryExpr(op, _)) => {
            assert_eq!(op.value, "&");
        }
        _ => panic!("Expected shared reference expression, got: {:?}", parsed),
    }
    assert_eq!(rest.len(), 0);
}

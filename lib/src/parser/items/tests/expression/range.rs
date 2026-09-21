use crate::ast::{Expression, LambdaArrowKind, Operand, SecondaryExpr, UnaryExpr};
use crate::parser::engine::{ParseCtx, Parser};
use crate::parser::items::expression;
use crate::parser::lex_test;
use crate::Config;

fn parse_range(source: &str) -> crate::ast::RangeExpr {
    let tokens = lex_test(source);
    let config = Config::default();
    let (rest, expression) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();
    assert_eq!(rest.len(), 0, "range parser left trailing tokens");
    let Expression::Range(range) = expression else {
        panic!("expected range expression for {source:?}, got {expression:?}");
    };
    range
}

#[test]
fn parses_all_native_range_forms() {
    for (source, has_start, has_end, inclusive) in [
        ("1..4", true, true, false),
        ("1..=4", true, true, true),
        ("..4", false, true, false),
        ("..=4", false, true, true),
        ("1..", true, false, false),
        ("..", false, false, false),
    ] {
        let range = parse_range(source);
        assert_eq!(range.start.is_some(), has_start, "source: {source}");
        assert_eq!(range.end.is_some(), has_end, "source: {source}");
        assert_eq!(range.inclusive, inclusive, "source: {source}");
    }
}

#[test]
fn range_endpoints_accept_expressions() {
    let range = parse_range("start + 1..finish * 2");
    assert!(range.start.is_some());
    assert!(range.end.is_some());
}

#[test]
fn inclusive_range_requires_an_end() {
    let tokens = lex_test("1..=");
    let config = Config::default();
    assert!(expression
        .process(ParseCtx::from(&tokens, &config))
        .is_err());
}

#[test]
fn inline_callback_does_not_capture_preceding_call_arguments() {
    for source in [
        "for_each 1..=30, number !-> number.println!",
        "for_each values, value !-> value.println!",
        "for_each &values, value !-> value.println!",
        "for_each 1..=30, number !->\n    number\n        |> transform\n        |> print_value",
    ] {
        let tokens = lex_test(source);
        let config = Config::default();
        let (rest, parsed) = expression
            .process(ParseCtx::from(&tokens, &config))
            .unwrap();
        assert!(rest.is_empty(), "trailing tokens in {source:?}: {rest:?}");
        let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(call)) = parsed else {
            panic!("expected call in {source:?}, got {parsed:?}");
        };
        let arguments = call
            .secondaries
            .unwrap()
            .into_iter()
            .find_map(|secondary| match secondary {
                SecondaryExpr::Arguments(arguments) => Some(arguments),
                _ => None,
            })
            .unwrap();
        assert_eq!(arguments.len(), 2, "source: {source}");
        let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(callback)) = &arguments[1].arg else {
            panic!("expected callback in {source:?}");
        };
        let Operand::LambdaDecl(lambda) = &callback.operand else {
            panic!("expected lambda in {source:?}");
        };
        assert_eq!(lambda.parameters.len(), 1);
        assert_eq!(lambda.arrow_kind, LambdaArrowKind::Unit);
    }
}

#[test]
fn parenthesized_callback_keeps_multiple_parameters() {
    let tokens = lex_test("apply (left, right !-> left.println!)");
    let config = Config::default();
    let (rest, parsed) = expression
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();
    assert!(rest.is_empty());
    let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(call)) = parsed else {
        panic!("expected call");
    };
    let SecondaryExpr::Arguments(arguments) = &call.secondaries.as_ref().unwrap()[0] else {
        panic!("expected arguments");
    };
    assert_eq!(arguments.len(), 1);
    let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(argument)) = &arguments[0].arg else {
        panic!("expected parenthesized argument");
    };
    let Operand::Expression(inner) = &argument.operand else {
        panic!("expected grouping, got {:?}", argument.operand);
    };
    let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(callback)) = inner.as_ref() else {
        panic!("expected callback");
    };
    let Operand::LambdaDecl(lambda) = &callback.operand else {
        panic!("expected lambda");
    };
    assert_eq!(lambda.parameters.len(), 2);
}

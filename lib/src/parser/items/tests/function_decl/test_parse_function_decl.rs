use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::{ast::tree::LambdaArrowKind, Config};

#[test]
fn test_parse_function_decl() {
    let input = "myfn = a, b, c ->\n    statement\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, function_decl) = function_decl
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_eq!(function_decl.name.name, "myfn");
    assert_eq!(function_decl.lambda.parameters.len(), 3);
    assert_eq!(function_decl.lambda.span.start, 15);
    assert_eq!(function_decl.lambda.span.end, 17);
    assert_eq!(function_decl.lambda.body.statements.len(), 1);
    assert_eq!(rest.len(), 0);
}

#[test]
fn test_parse_curried_function_decl() {
    let input = "myfn = a, b ~> a + b\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, function_decl) = function_decl
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_eq!(function_decl.name.name, "myfn");
    assert_eq!(function_decl.lambda.parameters.len(), 2);
    assert_eq!(function_decl.lambda.arrow_kind, LambdaArrowKind::Curried);
    assert_eq!(function_decl.lambda.span.start, 12);
    assert_eq!(function_decl.lambda.span.end, 14);
    assert_eq!(function_decl.lambda.to_string().trim(), "a, b ~> a + b");
    assert_eq!(rest.len(), 0);
}

#[test]
fn test_parse_unit_function_decl() {
    let input = "myfn = value !-> value\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, function_decl) = function_decl
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_eq!(function_decl.lambda.arrow_kind, LambdaArrowKind::Unit);
    assert_eq!(function_decl.lambda.span.start, 13);
    assert_eq!(function_decl.lambda.span.end, 16);
    assert_eq!(function_decl.lambda.to_string().trim(), "value !-> value");
    assert_eq!(rest.len(), 0);
}

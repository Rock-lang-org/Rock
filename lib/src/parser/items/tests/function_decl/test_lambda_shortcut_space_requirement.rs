use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_lambda_shortcut_requires_space() {
    // Lambda shortcut with space should parse successfully
    let input = "myfn = (- 42)\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let result = function_decl.process(ParseCtx::from(&tokens, &config));
    assert!(result.is_ok());

    let (rest, function_decl) = result.unwrap();
    assert_eq!(function_decl.name.name, "myfn");
    assert_eq!(function_decl.lambda.parameters.len(), 1);
    assert_eq!(rest.len(), 0);
}

#[test]
fn test_stuck_operator_not_lambda_shortcut() {
    // Stuck operator (no space) should NOT be treated as lambda shortcut
    // Instead it should parse as a parenthesized expression
    let input = "myfn = -> (-42)\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let result = function_decl.process(ParseCtx::from(&tokens, &config));
    assert!(result.is_ok());

    let (rest, function_decl) = result.unwrap();
    assert_eq!(function_decl.name.name, "myfn");
    // Should have 0 parameters since it's a lambda with no params
    assert_eq!(function_decl.lambda.parameters.len(), 0);
    assert_eq!(rest.len(), 0);
}

#[test]
fn test_multiple_operators_with_space() {
    // Multiple operators with spaces should create nested lambda shortcuts
    let input = "myfn = -> -(- 42)\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let result = function_decl.process(ParseCtx::from(&tokens, &config));
    assert!(result.is_ok());
}

#[test]
fn test_multiple_operators_without_space() {
    // Multiple operators without spaces should parse as unary expressions
    let input = "myfn = -> -(-42)\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let result = function_decl.process(ParseCtx::from(&tokens, &config));
    assert!(result.is_ok());
}

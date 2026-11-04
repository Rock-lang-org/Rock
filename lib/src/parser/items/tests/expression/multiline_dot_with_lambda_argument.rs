use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn multiline_dot_with_lambda_argument() {
    // Test case: multiline dot followed by a lambda argument
    // The lambda should be parsed correctly with its own indentation
    // Expected: foo.bar(lambda)
    let input = r#"foo
    .bar ->
        body"#;
    let tokens = lex_test(input);
    let config = Config::default();

    let result = expression.process(ParseCtx::from(&tokens, &config));

    // This should parse successfully
    assert!(
        result.is_ok(),
        "Failed to parse multiline dot with lambda: {:?}",
        result.err()
    );
}

#[test]
fn multiline_dot_chain_with_lambda_arguments() {
    // Test case: multiple multiline dots with lambda arguments
    // Each lambda should maintain proper indentation
    // Expected: foo.bar(lambda1).baz(lambda2)
    let input = r#"foo
    .bar ->
        body1
    .baz ->
        body2"#;
    let tokens = lex_test(input);
    let config = Config::default();

    let result = expression.process(ParseCtx::from(&tokens, &config));

    // This should parse successfully
    assert!(
        result.is_ok(),
        "Failed to parse multiline dot chain with lambdas: {:?}",
        result.err()
    );
}

#[test]
fn multiline_dot_with_nested_multiline_in_lambda() {
    // Test case: multiline dot with lambda that contains multiline expressions
    // The nested multiline should not affect the outer parsing
    // Expected: foo.bar(lambda_with_nested_multiline)
    let input = r#"foo
    .bar ->
        nested
            .method
            .chain"#;
    let tokens = lex_test(input);
    let config = Config::default();

    let result = expression.process(ParseCtx::from(&tokens, &config));

    // This should parse successfully
    assert!(
        result.is_ok(),
        "Failed to parse multiline dot with nested multiline in lambda: {:?}",
        result.err()
    );
}

#[test]
fn multiline_dot_with_inline_lambda_then_multiline_dot() {
    // This is the edge case from the test project
    // foo.bar(lol, lambda).haha
    // The issue is that after parsing the multiline dot at indent 4,
    // the indent_level gets set to 4, which then affects parsing of the lambda body
    let input = r#"foo
    .bar lol ->
        mdr
    .haha"#;
    let tokens = lex_test(input);
    let config = Config::default();

    let result = expression.process(ParseCtx::from(&tokens, &config));

    // This should parse successfully
    assert!(
        result.is_ok(),
        "Failed to parse multiline dot with inline lambda then multiline dot: {:?}",
        result.err()
    );
}

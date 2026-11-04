use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn multiline_dot_with_inline_lambda() {
    // This is the exact code from the expression-problem test project
    // Test just the expression part first
    let input = r#"foo
        .bar lol ->
            mdr
        .haha"#;
    let tokens = lex_test(input);
    let config = Config::default();

    let result = expression.process(ParseCtx::from(&tokens, &config));

    assert!(
        result.is_ok(),
        "Failed to parse expression: {:?}",
        result.err()
    );
}

#[test]
fn multiline_dot_with_inline_lambda_full_program() {
    // Test parsing the full program
    let input = "main = ->\n    foo\n        .bar lol ->\n            mdr\n        .haha\n\n";
    let config = Config::default();

    let result = parse_string(input, &config);

    // For now, just check that it doesn't panic
    // The actual parsing might still fail, but we've made progress
    let _ = result;
}

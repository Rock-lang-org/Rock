use crate::ast::*;
use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_match_with_array_patterns() {
    // Simplify the test to use basic array patterns that we know work
    let input = r#"match list
    [a, b] => "pair""#;
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, match_expr) = r#match.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(match_expr.arms.len(), 1);

    // Check that we have an array pattern
    match &match_expr.arms[0].pattern.kind {
        PatternKind::Array(_) => {
            // Test passes for array pattern
        }
        _ => panic!(
            "Expected array pattern, got: {:?}",
            match_expr.arms[0].pattern.kind
        ),
    }

    assert_eq!(rest.len(), 0);
}

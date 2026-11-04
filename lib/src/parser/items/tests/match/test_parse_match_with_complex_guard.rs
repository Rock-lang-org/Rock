use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_match_with_complex_guard() {
    let input = r#"match point
    (x, y) if x > 0 && y > 0 => "first quadrant"
    (x, y) if x < 0 => "left side"
    _ => "other""#;
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, match_expr) = r#match.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(match_expr.arms.len(), 3);

    // First arm should have a complex guard condition
    assert!(match_expr.arms[0].condition.is_some());

    // Second arm should have a simple guard condition
    assert!(match_expr.arms[1].condition.is_some());

    // Third arm (wildcard) should have no condition
    assert!(match_expr.arms[2].condition.is_none());

    assert_eq!(rest.len(), 0);
}

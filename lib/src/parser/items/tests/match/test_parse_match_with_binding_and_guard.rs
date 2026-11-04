use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_match_with_binding_and_guard() {
    let input = r#"match data
    value @ (x, y) if x > 5 => process value
    _ => default"#;
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, match_expr) = r#match.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(match_expr.arms.len(), 2);

    // First arm should have a binding pattern with guard
    assert!(match_expr.arms[0].pattern.binding.is_some());
    assert_eq!(
        match_expr.arms[0].pattern.binding.as_ref().unwrap().name,
        "value"
    );
    assert!(match_expr.arms[0].condition.is_some());

    assert_eq!(rest.len(), 0);
}

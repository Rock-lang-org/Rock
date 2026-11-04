use crate::ast::*;
use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_match_with_literal_patterns() {
    let input = r#"match value
    0 => "zero"
    1 => "one"
    42 => "answer"
    _ => "other""#;
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, match_expr) = r#match.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(match_expr.arms.len(), 4);

    // Check that first three arms have literal patterns
    for (i, arm) in match_expr.arms.iter().take(3).enumerate() {
        match &arm.pattern.kind {
            PatternKind::Literal(_) => {
                // Test passes for literal patterns
            }
            _ => panic!(
                "Expected literal pattern at arm {}, got: {:?}",
                i, arm.pattern.kind
            ),
        }
    }

    assert_eq!(rest.len(), 0);
}

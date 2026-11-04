use crate::ast::*;
use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn wildcard_pattern() {
    let input = "_";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, pattern) = pattern.process(ParseCtx::from(&tokens, &config)).unwrap();

    match pattern.kind {
        PatternKind::Wildcard => {
            // Test passes
        }
        _ => panic!("Expected wildcard pattern, got: {:?}", pattern.kind),
    }

    assert_eq!(rest.len(), 0);
}

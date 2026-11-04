use crate::ast::*;
use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn binding_pattern() {
    let input = "value @ (x, y)";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, pattern) = pattern.process(ParseCtx::from(&tokens, &config)).unwrap();

    // Should have a binding
    assert!(pattern.binding.is_some());
    assert_eq!(pattern.binding.unwrap().name, "value");

    // Should have a tuple pattern
    match pattern.kind {
        PatternKind::Tuple(patterns) => {
            assert_eq!(patterns.len(), 2);
        }
        _ => panic!("Expected tuple pattern, got: {:?}", pattern.kind),
    }

    assert_eq!(rest.len(), 0);
}

use crate::ast::*;
use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn array_pattern_with_rest() {
    // Test a simpler array pattern first to see if array patterns work at all
    let input = "[a, b]";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest_tokens, pattern) = pattern.process(ParseCtx::from(&tokens, &config)).unwrap();

    match pattern.kind {
        PatternKind::Array(patterns) => {
            assert_eq!(patterns.len(), 2);
            // Both should be regular patterns
            for (i, expected_name) in ["a", "b"].iter().enumerate() {
                match &patterns[i] {
                    ArrayPattern::Pattern(p) => match &p.kind {
                        PatternKind::Ident(ident_pat) => {
                            assert_eq!(ident_pat.name.name, *expected_name);
                        }
                        _ => panic!("Expected ident pattern at position {}", i),
                    },
                    _ => panic!("Expected regular pattern at position {}", i),
                }
            }
        }
        _ => panic!("Expected array pattern, got: {:?}", pattern.kind),
    }

    assert_eq!(rest_tokens.len(), 0);
}

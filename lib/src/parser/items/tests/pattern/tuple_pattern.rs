use crate::ast::*;
use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn tuple_pattern() {
    let input = "(x, y, z)";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, pattern) = pattern.process(ParseCtx::from(&tokens, &config)).unwrap();

    match pattern.kind {
        PatternKind::Tuple(patterns) => {
            assert_eq!(patterns.len(), 3);
            let names = ["x", "y", "z"];
            for (i, expected_name) in names.iter().enumerate() {
                match &patterns[i].kind {
                    PatternKind::Ident(ident_pat) => {
                        assert_eq!(ident_pat.name.name, *expected_name);
                    }
                    _ => panic!("Expected ident pattern at position {}", i),
                }
            }
        }
        _ => panic!("Expected tuple pattern, got: {:?}", pattern.kind),
    }

    assert_eq!(rest.len(), 0);
}

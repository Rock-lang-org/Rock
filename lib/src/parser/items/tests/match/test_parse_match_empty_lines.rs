use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_match_empty_lines() {
    let input = r#"match a
    
    a => 2
    
    b => 3"#;
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, _expression) = r#match.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(rest.len(), 0);
}

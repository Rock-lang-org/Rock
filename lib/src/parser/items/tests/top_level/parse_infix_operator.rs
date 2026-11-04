use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn parse_infix_operator() {
    let input = "infix 5 |>\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, (precedence, name)) = infix_operator_decl
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_eq!(precedence, 5);
    assert_eq!(name, "|>".to_string());
    assert_eq!(rest.len(), 0);
}

#[test]
fn parse_caret_infix_operator() {
    let input = "infix 4 ^\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, (precedence, name)) = infix_operator_decl
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_eq!(precedence, 4);
    assert_eq!(name, "^".to_string());
    assert_eq!(rest.len(), 0);
}

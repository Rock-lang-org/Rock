use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_large_array() {
    let input = "[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, array) = array.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(array.elements.len(), 10);
    assert_eq!(rest.len(), 0);
}

use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_multiline_array_with_empty_lines() {
    let input = "[\n    1,\n\n    2,\n    \n    3\n]";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, array) = array.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(array.elements.len(), 3);
    assert_eq!(rest.len(), 0);
}

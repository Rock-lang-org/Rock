use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_mixed_multiline_array() {
    let input = "[\n    1, 2,\n    3, 4\n]";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, array) = array.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(array.elements.len(), 4);
    assert_eq!(rest.len(), 0);
}

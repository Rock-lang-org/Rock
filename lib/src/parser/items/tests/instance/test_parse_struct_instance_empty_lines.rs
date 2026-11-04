use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_struct_instance_empty_lines() {
    let input = "Test\n\n    a: 1\n\n    b: 2\n\n    c: a + 4";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, struct_instance) = instance.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(struct_instance.name.to_string(), "Test");
    assert_eq!(struct_instance.fields.len(), 3);
    assert_eq!(rest.len(), 0);
}

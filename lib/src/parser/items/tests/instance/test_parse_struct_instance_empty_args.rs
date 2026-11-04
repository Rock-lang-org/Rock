use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_struct_instance_empty_args() {
    let input = "Test";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, struct_instance) = instance.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(struct_instance.name.to_string(), "Test");
    assert_eq!(struct_instance.fields.len(), 0);
    assert_eq!(rest.len(), 0);
}

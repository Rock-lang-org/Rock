use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_impl_for() {
    let input = "impl Test for Test2\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, r#impl) = r#impl.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(r#impl.name.to_string(), "Test");
    assert_eq!(r#impl.for_.unwrap().to_string(), "Test2");
    assert_eq!(rest.len(), 0);
}

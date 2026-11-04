use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_impl_with_methods() {
    let input = "impl Test\n    new = -> lol\n    @add = -> a\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, r#impl) = r#impl.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(r#impl.name.to_string(), "Test");
    assert_eq!(r#impl.methods.len(), 2);
    assert_eq!(
        r#impl
            .methods
            .iter()
            .find(|(k, _v)| k.name == "new")
            .unwrap()
            .1
            .name
            .name,
        "new"
    );
    assert_eq!(rest.len(), 0);
}

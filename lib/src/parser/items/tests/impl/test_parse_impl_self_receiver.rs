use crate::ast::*;
use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_impl_allows_move_self_receiver() {
    let input = "impl Test\n    ~@bar = x -> x\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, parsed) = r#impl.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(parsed.methods.len(), 1);
    assert_eq!(
        parsed.methods.values().next().unwrap().self_receiver,
        Some(SelfReceiverMode::Move)
    );
    assert_eq!(rest.len(), 0);
}

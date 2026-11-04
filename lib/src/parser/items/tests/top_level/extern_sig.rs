use crate::parser::items::tests::common::lex_test_toplevel;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn extern_sig() {
    let input = "extern toto : Toto -> Tata\n";
    let tokens = lex_test_toplevel(input);
    let config = Config::default();

    let (rest, _top_level) = top_level.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(rest.len(), 0);
}

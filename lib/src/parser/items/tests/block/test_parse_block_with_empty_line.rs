use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_block_with_empty_line() {
    let input = "\n    statement\n\n    statement\n    \n    statement";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, block) = block.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(block.statements.len(), 3);
    assert_eq!(rest.len(), 0);
}

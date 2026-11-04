use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_block_with_indent() {
    let input = "\n    statement\n    statement";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, block) = block.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(block.statements.len(), 2);
    assert_eq!(rest.len(), 0);
}

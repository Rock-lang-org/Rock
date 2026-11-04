use crate::ast::*;
use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_function_decl_self_receiver_modes() {
    let config = Config::default();

    let cases = [
        ("@a = x -> x\n", Some(SelfReceiverMode::Shared)),
        ("^@a = x -> x\n", Some(SelfReceiverMode::Mut)),
        ("~@a = x -> x\n", Some(SelfReceiverMode::Move)),
    ];

    for (input, expected) in cases {
        let tokens = lex_test(input);
        let (rest, function_decl) = function_decl
            .process(ParseCtx::from(&tokens, &config))
            .unwrap();

        assert_eq!(function_decl.name.name, "a");
        assert_eq!(function_decl.self_receiver, expected);
        assert_eq!(rest.len(), 0);
    }
}

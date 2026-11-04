use crate::ast::*;
use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_trait_allows_mut_self_receiver() {
    let input = "trait Foo\n    ^@bar : Int\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, trait_decl) = r#trait.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(trait_decl.signatures.len(), 1);
    assert_eq!(
        trait_decl.signatures.values().next().unwrap().self_receiver,
        Some(SelfReceiverMode::Mut)
    );
    assert_eq!(rest.len(), 0);
}

#[test]
fn test_parse_trait_allows_move_self_receiver_signature() {
    let input = "trait Drop\n    ~@drop: ()\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, trait_decl) = r#trait.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(trait_decl.signatures.len(), 1);
    assert_eq!(
        trait_decl.signatures.values().next().unwrap().self_receiver,
        Some(SelfReceiverMode::Move)
    );
    assert_eq!(rest.len(), 0);
}

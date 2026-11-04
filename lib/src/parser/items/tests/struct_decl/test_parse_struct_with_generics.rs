use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_struct_with_generics() {
    let input = "struct Test T, U\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, struct_decl) = struct_decl
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_eq!(struct_decl.name.name, "Test");
    assert_eq!(struct_decl.generic_params.len(), 2);
    assert_eq!(struct_decl.generic_params[0].name.name, "T");
    assert_eq!(struct_decl.generic_params[1].name.name, "U");
    assert_eq!(rest.len(), 0);
}

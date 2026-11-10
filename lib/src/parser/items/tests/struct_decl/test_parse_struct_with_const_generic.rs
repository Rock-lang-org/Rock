use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_struct_with_usize_generic() {
    let input = "struct Buf T, N: usize\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, struct_decl) = struct_decl
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_eq!(struct_decl.generic_params.len(), 2);
    assert_eq!(struct_decl.generic_params[0].name.name, "T");
    assert!(struct_decl.generic_params[0].kind.is_none());
    assert_eq!(struct_decl.generic_params[1].name.name, "N");
    let kind = struct_decl.generic_params[1]
        .kind
        .as_ref()
        .expect("N must carry a kind");
    assert_eq!(kind.constructor.type_name(), "usize");
    assert!(kind.args.is_empty());
    assert_eq!(rest.len(), 0);
}

#[test]
fn test_parse_struct_with_parenthesized_usize_generic() {
    let input = "struct Buf T, (N: usize)\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, struct_decl) = struct_decl
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    assert_eq!(struct_decl.generic_params.len(), 2);
    let kind = struct_decl.generic_params[1]
        .kind
        .as_ref()
        .expect("N must carry a kind");
    assert_eq!(kind.constructor.type_name(), "usize");
    assert_eq!(rest.len(), 0);
}

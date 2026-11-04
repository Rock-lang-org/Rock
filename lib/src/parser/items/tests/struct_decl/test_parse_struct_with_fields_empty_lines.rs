use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_struct_with_fields_empty_lines() {
    let input = "struct Test\n\n    field: Type\n\n    field2: Type2\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, struct_decl) = struct_decl
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    let ty = struct_decl.name;

    assert_eq!(ty.name, "Test");
    assert_eq!(struct_decl.fields.len(), 2);
    assert_eq!(rest.len(), 0);
}

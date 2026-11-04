use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_struct_with_fields() {
    let input = "struct Test\n    field: Type\n    < field2: Type2\n";
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, struct_decl) = struct_decl
        .process(ParseCtx::from(&tokens, &config))
        .unwrap();

    let ty = struct_decl.name;
    let field = struct_decl
        .fields
        .iter()
        .find(|field| field.name.name == "field")
        .unwrap();
    let field2 = struct_decl
        .fields
        .iter()
        .find(|field| field.name.name == "field2")
        .unwrap();

    assert_eq!(ty.name, "Test");
    assert_eq!(struct_decl.fields.len(), 2);
    assert_eq!(field.ty.to_string(), "Type");
    assert!(!field.public);
    assert_eq!(field2.ty.to_string(), "Type2");
    assert!(field2.public);
    assert_eq!(rest.len(), 0);
}

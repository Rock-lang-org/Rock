use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_trait_with_associated_type() {
    let tokens = lex_test(
        r#"trait Index Idx
    type Output
    @index: Idx -> &Self::Output
"#,
    );
    let config = Config::default();

    let (rest, trait_decl) = r#trait.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(trait_decl.name.name, "Index");
    assert_eq!(trait_decl.generic_params.len(), 1);
    assert_eq!(trait_decl.associated_types.len(), 1);
    assert_eq!(trait_decl.associated_types[0].name.name, "Output");
    assert_eq!(trait_decl.signatures.len(), 1);
    assert_eq!(rest.len(), 0);
}

#[test]
fn test_parse_constructor_valued_associated_type() {
    let tokens = lex_test("trait Functor\n    type Family _\n");
    let config = Config::default();

    let (rest, trait_decl) = r#trait.process(ParseCtx::from(&tokens, &config)).unwrap();

    let associated_type = &trait_decl.associated_types[0];
    assert_eq!(associated_type.name.name, "Family");
    assert_eq!(associated_type.kind.as_ref().unwrap().args.len(), 1);
    assert_eq!(rest.len(), 0);
}

use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_impl_with_associated_type() {
    let tokens = lex_test(
        r#"impl Deref for Box T
    type Target = T
    @deref = -> @value
"#,
    );
    let config = Config::default();

    let (rest, impl_decl) = r#impl.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(impl_decl.name.type_name(), "Deref");
    assert_eq!(impl_decl.associated_types.len(), 1);
    assert_eq!(impl_decl.associated_types[0].name.name, "Target");
    assert_eq!(impl_decl.associated_types[0].ty.to_string(), "T");
    assert_eq!(impl_decl.methods.len(), 1);
    assert_eq!(rest.len(), 0);
}

#[test]
fn test_parse_constructor_valued_associated_type_definition() {
    let tokens = lex_test("impl Functor for Box T\n    type Family _ = Box\n");
    let config = Config::default();

    let (rest, impl_decl) = r#impl.process(ParseCtx::from(&tokens, &config)).unwrap();

    let associated_type = &impl_decl.associated_types[0];
    assert_eq!(associated_type.name.name, "Family");
    assert_eq!(associated_type.kind.as_ref().unwrap().args.len(), 1);
    assert_eq!(associated_type.ty.to_string(), "Box");
    assert_eq!(rest.len(), 0);
}

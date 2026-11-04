use crate::parser::items::tests::common::lex_test;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_trait_empty_lines() {
    let tokens = lex_test(
        r#"trait Foo
    
    bar = a -> a
    
    baz : Int
    
    @selfinject = a -> a
"#,
    );
    let config = Config::default();

    let (rest, trait_decl) = r#trait.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(trait_decl.name.name, "Foo");
    assert_eq!(trait_decl.methods.len(), 2);
    assert_eq!(trait_decl.signatures.len(), 1);
    assert_eq!(rest.len(), 0);
}

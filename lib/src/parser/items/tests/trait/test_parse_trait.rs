use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

#[test]
fn test_parse_trait() {
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

#[test]
fn trait_parses_constructor_supertrait_predicates() {
    let tokens = lex_test("trait Applicative for F _ where F: Functor\n");
    let config = Config::default();

    let (rest, trait_decl) = r#trait.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert!(rest.is_empty());
    assert_eq!(trait_decl.for_.as_ref().unwrap().name.name, "F");
    assert_eq!(trait_decl.where_clauses.len(), 1);
    assert_eq!(trait_decl.where_clauses[0].subject.type_name(), "F");
    assert_eq!(
        trait_decl.where_clauses[0]
            .trait_bound
            .as_ref()
            .unwrap()
            .type_name(),
        "Functor"
    );
}

#[test]
fn trait_parses_self_supertrait_predicates() {
    let tokens = lex_test("trait FnMut Args, Ret where Self: FnOnce Args, Ret\n");
    let config = Config::default();
    let (rest, declaration) = r#trait.process(ParseCtx::from(&tokens, &config)).unwrap();
    assert!(rest.is_empty());
    assert_eq!(declaration.generic_params.len(), 2);
    assert!(declaration.for_.is_none());
    assert_eq!(declaration.where_clauses.len(), 1);
    assert_eq!(declaration.where_clauses[0].subject.type_name(), "Self");
}

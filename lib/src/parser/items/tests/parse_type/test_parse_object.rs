use crate::ast::{ObjectQualifier, ParseType};
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

fn parse(input: &str) -> ParseType {
    let tokens = lex_test(input);
    let config = Config::default();
    let (rest, ty) = parse_type
        .process(ParseCtx::from(&tokens, &config))
        .expect("object type");
    assert!(rest.is_empty(), "unparsed tokens: {:?}", rest.tokens);
    ty
}

#[test]
fn object_qualifiers_preserve_instantiated_owner_and_binding_spans() {
    let source = "&(Source I64 { (Base I64)::Item = (Option I64), Send, (Other Bool) })";
    let ParseType::Reference { pointee, .. } = parse(source) else {
        panic!("reference")
    };
    let ParseType::Object(object) = pointee.as_ref() else {
        panic!("object")
    };
    assert_eq!(object.qualifiers.len(), 3);
    let ObjectQualifier::Binding {
        owner: Some(owner),
        member,
        ty,
    } = &object.qualifiers[0]
    else {
        panic!("binding")
    };
    assert_eq!(owner.type_name(), "Base");
    assert_eq!(ty.type_name(), "Option");
    assert_eq!(&source[member.span.start..member.span.end], "Item");
    assert!(matches!(
        object.qualifiers[2],
        ObjectQualifier::Trait(ParseType::Application(_))
    ));
}

#[test]
fn object_qualifiers_format_and_reparse() {
    let ty = parse("&mut Source { Item = (I64 -> Bool), Send, }");
    let formatted = ty.to_string();
    let reparsed = parse(&formatted);
    assert_eq!(reparsed.to_string(), formatted);
}

#[test]
fn object_qualifiers_reject_missing_values_and_unparenthesized_applications() {
    for source in [
        "Source {}",
        "Source { Item = }",
        "Source { Item = Option I64 }",
        "Source { Other I64 }",
    ] {
        let tokens = lex_test(source);
        let config = Config::default();
        assert!(
            parse_type
                .process(ParseCtx::from(&tokens, &config))
                .is_err(),
            "{source}"
        );
    }
}

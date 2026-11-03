use super::*;

// ===========================================================================
// Test: Simple function
// ===========================================================================

#[test]
fn simple_function_is_valid() {
    validate("main = -> 1\n");
}

#[test]
fn simple_function_structure() {
    let p = parse("main = -> 1\n");
    let f = first_fn(&p);
    assert_eq!(f.name.name, "main");
    assert_eq!(f.lambda.parameters.len(), 0);
    assert_eq!(f.lambda.body.statements.len(), 1);
    assert_eq!(f.self_receiver, None);
}

// ===========================================================================
// Test: Function with parameters
// ===========================================================================

#[test]
fn function_with_params() {
    let input = "add = a, b -> a + b\n";
    validate(input);

    let p = parse(input);
    let f = first_fn(&p);
    assert_eq!(f.name.name, "add");
    assert_eq!(f.lambda.parameters.len(), 2);

    // Both parameters should be ident patterns
    for param in &f.lambda.parameters {
        assert!(
            matches!(param.kind, PatternKind::Ident(_)),
            "Expected ident pattern, got {:?}",
            param.kind,
        );
    }
}

// ===========================================================================
// Test: Multiple top-level declarations
// ===========================================================================

#[test]
fn multiple_functions() {
    let input = r#"
foo = -> 1

bar = -> 2

baz = x -> x
"#;
    validate(input);

    let p = parse(input);
    let c = TopLevelCounter::count(&p);
    assert_eq!(c.functions, 3);
}

// ===========================================================================
// Test: Struct declaration
// ===========================================================================

#[test]
fn struct_declaration_valid() {
    let input = "struct Point\n    x: Int\n    y: Int\n";
    validate(input);

    let p = parse(input);
    let s = find_struct(&p, "Point");
    assert_eq!(s.fields.len(), 2);
    assert_eq!(s.fields[0].name.name, "x");
    assert_eq!(s.fields[1].name.name, "y");

    // Fields should have types
    for field in &s.fields {
        assert!(
            matches!(field.ty, ParseType::Type(_)),
            "Expected named type, got {:?}",
            field.ty,
        );
    }
}

#[test]
fn struct_with_default_field() {
    let input = "struct Point\n    x: Int\n    y: Int = 0\n";
    validate(input);

    let p = parse(input);
    let s = find_struct(&p, "Point");
    assert!(s.fields[0].default.is_none());
    assert!(s.fields[1].default.is_some());
}

#[test]
fn struct_with_generics() {
    let input = "struct Container T\n    value: T\n";
    validate(input);

    let p = parse(input);
    let s = find_struct(&p, "Container");
    assert_eq!(s.generic_params.len(), 1);
    assert_eq!(s.fields.len(), 1);
}

// ===========================================================================
// Test: Enum declaration
// ===========================================================================

#[test]
fn enum_declaration_valid() {
    let input = "enum Color\n    Red\n    Green\n    Blue\n";
    validate(input);

    let p = parse(input);
    let e = find_enum(&p, "Color");
    assert_eq!(e.variants.len(), 3);
    assert_eq!(e.variants[0].name.name, "Red");
    assert_eq!(e.variants[1].name.name, "Green");
    assert_eq!(e.variants[2].name.name, "Blue");
}

#[test]
fn enum_with_payload() {
    let input = "enum Result T, E\n    Ok T\n    Err E\n";
    validate(input);

    let p = parse(input);
    let e = find_enum(&p, "Result");
    assert_eq!(e.generic_params.len(), 2);
    assert_eq!(e.variants.len(), 2);

    // Each variant should have a type payload
    for variant in &e.variants {
        match &variant.fields {
            NamedFieldsOrTypesList::TypesList(types) => {
                assert!(
                    !types.is_empty(),
                    "Variant {} should have types",
                    variant.name.name,
                );
            }
            NamedFieldsOrTypesList::NamedFields(_) => {
                panic!("Expected TypesList for variant {}", variant.name.name);
            }
        }
    }
}

// ===========================================================================
// Test: Trait declaration
// ===========================================================================

#[test]
fn trait_declaration_valid() {
    let input = "trait Display\n    display: () -> String\n";
    validate(input);

    let p = parse(input);
    let t = find_trait(&p, "Display");
    assert_eq!(t.signatures.len(), 1);
    assert!(t.signatures.contains_key(&Ident {
        name: "display".to_string(),
        span: crate::lexer::Span::test(),
    }));
}

#[test]
fn trait_with_default_method() {
    let input = "trait Greet\n    greet = -> 42\n";
    validate(input);

    let p = parse(input);
    let t = find_trait(&p, "Greet");
    assert_eq!(t.methods.len(), 1);
}

#[test]
fn trait_with_associated_type_is_valid() {
    let input = r#"trait Deref
    type Target
    @deref: () -> &Self::Target
"#;
    validate(input);

    let p = parse(input);
    let t = find_trait(&p, "Deref");
    assert_eq!(t.associated_types.len(), 1);
    assert_eq!(t.associated_types[0].name.name, "Target");
}

// ===========================================================================
// Test: Type annotations
// ===========================================================================

#[test]
fn function_type_annotations() {
    let input = r#"add: Int -> Int -> Int
add = a, b -> a + b
"#;
    validate(input);

    let p = parse(input);
    let c = TopLevelCounter::count(&p);
    assert_eq!(c.functions, 1, "Should have 1 function decl");
}

#[test]
fn complex_type_annotations() {
    let input = r#"struct Container T
    value: T
    count: Int

main = ->
    c: Container Int = Container value: 42, count: 1
    c
"#;
    validate(input);
}

// ===========================================================================
// Test: Extern declarations
// ===========================================================================

#[test]
fn extern_declaration() {
    let input = "extern printf: *Char -> Int\n";
    let config = Config::default();
    let result = parse_string(input, &config);
    assert!(
        result.is_ok(),
        "Extern decl should parse: {:?}",
        result.err()
    );

    let p = result.unwrap();
    let c = TopLevelCounter::count(&p);
    assert_eq!(c.externs, 1);
}

use super::*;

// ===========================================================================
// Test: Impl block
// ===========================================================================

#[test]
fn impl_for_generic_type() {
    let input = r#"trait Show
    show: () -> String

struct Container T
    value: T

impl Show for Container T
    show = -> "Container"
"#;
    validate(input);

    let p = parse(input);
    let i = program_find_impl_for(&p, "Show", "Container");
    assert!(i.is_some(), "Should find impl Show for Container T");
    let i = i.unwrap();
    assert_eq!(i.name.generics().len(), 0, "Show trait has no generics");
    assert_eq!(
        i.for_.as_ref().unwrap().generics().len(),
        1,
        "Container T has one generic param"
    );

    // Check that the generic parameter is just "T"
    let gen = &i.for_.as_ref().unwrap().generics()[0];
    match gen {
        ParseType::Type(inner) => assert_eq!(inner.name, "T"),
        _ => panic!("Expected Type for generic parameter"),
    }
}

#[test]
fn impl_for_array_named_type_is_plain_named_impl_target() {
    let input = r#"trait Show
    show: () -> String

struct Array T
    < value: T

impl Show for Array T
    show = -> "Array"
"#;
    validate(input);

    let p = parse(input);
    let i = program_find_impl_for(&p, "Show", "Array");
    assert!(i.is_some(), "Should find impl Show for user type Array T");
    let i = i.unwrap();
    let for_type = i.for_.as_ref().expect("impl should have a target type");
    let ParseType::Application(application) = for_type else {
        panic!("expected Array type application, got {:?}", for_type);
    };
    assert!(matches!(
        application.constructor.as_ref(),
        ParseType::Type(inner) if inner.name == "Array"
    ));
    assert_eq!(application.args.len(), 1);
}

#[test]
fn impl_for_borrowed_slice_type_parses_reference_to_slice() {
    let input = r#"trait Show
    show: () -> String

impl Show for &[T]
    show = -> "slice"
"#;
    validate(input);

    let p = parse(input);
    let i = program_find_impl_for(&p, "Show", "&[T]")
        .expect("Should find impl Show for borrowed slice");
    let for_type = i.for_.as_ref().expect("impl should have a target type");
    match for_type {
        ParseType::Reference { is_mut, pointee } => {
            assert!(!is_mut);
            assert!(matches!(pointee.as_ref(), ParseType::Slice(_)));
        }
        other => panic!("expected borrowed slice type, got {:?}", other),
    }
}

#[test]
fn impl_with_associated_type_is_valid() {
    let input = r#"trait Deref
    type Target
    @deref: () -> &Self::Target

struct Box T
    value: T

impl Deref for Box T
    type Target = T
    @deref = -> @value
"#;
    validate(input);

    let p = parse(input);
    let i = program_find_impl_for(&p, "Deref", "Box").unwrap();
    assert_eq!(i.associated_types.len(), 1);
    assert_eq!(i.associated_types[0].name.name, "Target");
}

#[test]
fn impl_with_multiple_generic_params() {
    let input = r#"trait PairShow
    show_pair: () -> String

struct Pair A, B
    first: A
    second: B

impl PairShow for Pair A, B
    show_pair = -> "Pair"
"#;
    validate(input);

    let p = parse(input);
    let i = program_find_impl_for(&p, "PairShow", "Pair");
    assert!(i.is_some(), "Should find impl PairShow for Pair A, B");
    let i = i.unwrap();
    assert_eq!(
        i.for_.as_ref().unwrap().generics().len(),
        2,
        "Pair A, B has two generic params"
    );
}

#[test]
fn impl_with_where_clause() {
    let input = r#"trait Show
    show: () -> String

struct Container T
    value: T

impl Show for Container T where T: Show
    show = -> "Container"
"#;
    validate(input);

    let p = parse(input);
    let i = program_find_impl_for(&p, "Show", "Container");
    assert!(
        i.is_some(),
        "Should find impl Show for Container T with where clause"
    );
    let i = i.unwrap();
    assert_eq!(i.where_clauses.len(), 1, "Should have one where clause");
    assert_eq!(i.where_clauses[0].subject.type_name(), "T");
    assert_eq!(
        i.where_clauses[0]
            .trait_bound
            .as_ref()
            .expect("trait bound")
            .type_name(),
        "Show"
    );
}

#[test]
fn impl_with_multiple_where_clauses() {
    let input = r#"trait Show
    show: () -> String

struct Pair A, B
    first: A
    second: B

impl Show for Pair A, B where A: Show, B: Show
    show = -> "Pair"
"#;
    validate(input);

    let p = parse(input);
    let i = program_find_impl_for(&p, "Show", "Pair");
    assert!(
        i.is_some(),
        "Should find impl Show for Pair A, B with where clauses"
    );
    let i = i.unwrap();
    assert_eq!(i.where_clauses.len(), 2, "Should have two where clauses");
    assert_eq!(i.where_clauses[0].subject.type_name(), "A");
    assert_eq!(
        i.where_clauses[0]
            .trait_bound
            .as_ref()
            .expect("trait bound")
            .type_name(),
        "Show"
    );
    assert_eq!(i.where_clauses[1].subject.type_name(), "B");
    assert_eq!(
        i.where_clauses[1]
            .trait_bound
            .as_ref()
            .expect("trait bound")
            .type_name(),
        "Show"
    );
}

#[test]
fn impl_block_valid() {
    let input = r#"struct Foo
    x: Int

impl Foo
    get_x = -> @x
"#;
    validate(input);

    let p = parse(input);
    let i = find_impl(&p, "Foo");
    assert!(i.for_.is_none());
    assert_eq!(i.methods.len(), 1);
}

#[test]
fn impl_for_trait() {
    let input = r#"trait Display
    display: () -> String

struct Foo
    x: Int

impl Display for Foo
    display = -> "Foo"
"#;
    validate(input);

    let p = parse(input);
    // Find the impl-for
    let i = program_find_impl_for(&p, "Display", "Foo");
    assert!(i.is_some(), "Should find impl Display for Foo");
    let i = i.unwrap();
    assert_eq!(i.methods.len(), 1);
}

fn program_find_impl_for<'a>(
    program: &'a Program,
    trait_name: &str,
    for_name: &str,
) -> Option<&'a Impl> {
    program.module.top_levels.iter().find_map(|tl| match tl {
        TopLevel::Impl(i)
            if i.name.type_name() == trait_name
                && i.for_.as_ref().map(|f| f.type_name()) == Some(for_name.to_string()) =>
        {
            Some(i)
        }
        _ => None,
    })
}

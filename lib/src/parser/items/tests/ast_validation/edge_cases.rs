use super::*;

// ===========================================================================
// Test: Edge cases
// ===========================================================================

#[test]
fn empty_tuple() {
    // Unit value
    let input = "main = -> ()\n";
    let config = Config::default();
    assert!(parse_string(input, &config).is_ok());
}

#[test]
fn deeply_nested_expressions() {
    let input = "main = -> ((((1 + 2))))\n";
    validate(input);
}

#[test]
fn multiline_function_body() {
    let input = r#"main = ->
    a = 1
    b = 2
    c = 3
    d = a + b + c
    d
"#;
    validate(input);

    let p = parse(input);
    let f = first_fn(&p);
    assert_eq!(f.lambda.body.statements.len(), 5);
}

#[test]
fn empty_array() {
    let input = "main = -> []\n";
    let config = Config::default();
    assert!(parse_string(input, &config).is_ok());
}

#[test]
fn programs_separated_by_blank_lines() {
    let input = r#"

foo = -> 1


bar = -> 2


baz = -> 3

"#;
    validate(input);

    let p = parse(input);
    let c = TopLevelCounter::count(&p);
    assert_eq!(c.functions, 3);
}

#[test]
fn function_with_many_parameters() {
    let input = "f = a, b, c, d, e -> a\n";
    validate(input);

    let p = parse(input);
    let f = first_fn(&p);
    assert_eq!(f.lambda.parameters.len(), 5);
}

// ===========================================================================
// Test: Top-level counting for a complex program
// ===========================================================================

#[test]
fn complex_program_top_level_counts() {
    let input = r#"struct Point
    x: Int
    y: Int

struct Color
    r: Int
    g: Int
    b: Int

enum Shape
    Circle Int
    Rect Int, Int

trait Drawable
    draw: () -> Int

impl Drawable for Point
    draw = -> @x

impl Point
    new = x, y -> Point x: x, y: y

main = ->
    p = Point::new 1, 2
    p.draw!
"#;
    validate(input);

    let p = parse(input);
    let c = TopLevelCounter::count(&p);
    assert_eq!(c.structs, 2, "Should have 2 structs");
    assert_eq!(c.enums, 1, "Should have 1 enum");
    assert_eq!(c.traits, 1, "Should have 1 trait");
    assert_eq!(c.impls, 2, "Should have 2 impls");
    assert_eq!(c.functions, 1, "Should have 1 top-level function");
}

// ===========================================================================
// Test: Parse errors (should fail)
// ===========================================================================

#[test]
fn parse_error_bad_indent() {
    let input = "main = ->\na\n  2\n";
    let config = Config::default();
    assert!(parse_string(input, &config).is_err());
}

#[test]
fn parse_error_missing_body() {
    let input = "main = ->\n";
    let config = Config::default();
    assert!(parse_string(input, &config).is_err());
}

// ===========================================================================
// Test: Unsafe blocks
// ===========================================================================

#[test]
fn unsafe_block_valid() {
    let input = r#"main = ->
    x = unsafe
        42
    x
"#;
    validate(input);
}

// ===========================================================================
// Test: Double-dot (method cascade / copy-with)
// ===========================================================================

#[test]
fn double_dot_expression() {
    let input = r#"main = ->
    chained = foo..bar
    chained
"#;
    validate(input);
}

// ===========================================================================
// Test: Error propagation (?)
// ===========================================================================

#[test]
fn error_propagation() {
    let input = r#"main = ->
    x = foo?
    x
"#;
    validate(input);

    let p = parse(input);
    let f = first_fn(&p);
    // First statement is assignment
    if let Statement::Assignment(a) = &f.lambda.body.statements[0] {
        if let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(pe)) = &a.rhs {
            assert!(pe.secondaries.is_some());
            let secs = pe.secondaries.as_ref().unwrap();
            assert!(
                secs.iter()
                    .any(|s| matches!(s, SecondaryExpr::Interogation)),
                "Should have ? operator in secondaries",
            );
        } else {
            panic!("Expected primary expression for RHS");
        }
    } else {
        panic!("Expected assignment");
    }
}

// ===========================================================================
// Test: Infix operator declarations
// ===========================================================================

#[test]
fn infix_operator_declaration() {
    let input = r#"infix 5 |>

main = -> 1
"#;
    let config = Config::default();
    let result = parse_string(input, &config);
    assert!(
        result.is_ok(),
        "Infix decl should parse: {:?}",
        result.err()
    );

    let p = result.unwrap();
    let has_infix = p
        .module
        .top_levels
        .iter()
        .any(|tl| matches!(tl, TopLevel::InfixOperator(5, _)));
    assert!(has_infix, "Should have an infix operator declaration");
}

use super::*;

// ===========================================================================
// Test: Lambdas
// ===========================================================================

#[test]
fn inline_lambda() {
    let input = r#"main = ->
    double = x -> x + x
    double 5
"#;
    validate(input);
}

#[test]
fn lambda_as_argument() {
    let input = r#"apply = f, x -> f x

main = -> apply (x -> x + 1), 5
"#;
    validate(input);
}

#[test]
fn shorthand_functions() {
    let input = r#"main = ->
    plus_one = (+1)
    times_two = (*2)
    result = plus_one 5
    result
"#;
    validate(input);
}

// ===========================================================================
// Test: Struct instantiation
// ===========================================================================

#[test]
fn struct_instantiation_multiline() {
    let input = r#"struct Point
    x: Int
    y: Int
    z: Int

main = ->
    point = Point
        x: 10
        y: 20
        z: 30
    point
"#;
    validate(input);
}

#[test]
fn struct_instantiation_inline() {
    let input = r#"struct Point
    x: Int
    y: Int

main = ->
    point = Point x: 10, y: 20
    point
"#;
    validate(input);
}

// ===========================================================================
// Test: Pattern matching in assignments
// ===========================================================================

#[test]
fn struct_pattern_assignment() {
    let input = r#"struct Point
    x: Int
    y: Int

main = ->
    point = Point x: 10, y: 20
    Point x: px, y: py = point
    px
"#;
    validate(input);
}

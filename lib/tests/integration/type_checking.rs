use crate::support::*;

#[test]
fn test_integer_literal_defaults_to_i64() {
    compile_should_pass_without_std(
        r#"main = ->
    value = 42
    0
"#,
    );
}

#[test]
fn test_float_literal_defaults_to_f64() {
    compile_should_pass_without_std(
        r#"main = ->
    value = 4.2
    0
"#,
    );
}

#[test]
fn test_unconstrained_local_lambda_is_rejected() {
    compile_should_fail(
        r#"main = ->
    identity = value -> value
    0
"#,
        "ambiguous type",
    );
}

#[test]
fn test_generic_function_remains_polymorphic_under_strict_finalization() {
    compile_should_pass_without_std(
        r#"identity = value -> value

main = ->
    integer: I64 = identity 42
    boolean: Bool = identity true
    0
"#,
    );
}

#[test]
fn test_type_error_int_plus_string() {
    compile_should_fail(
        r#"
main = ->
    x = 42 + "hello"
    0
"#,
        "No implementation found for operator '+' on type I64",
    );
}

#[test]
fn test_type_error_float_plus_string() {
    compile_should_fail(
        r#"
main = ->
    x = 3.14 + "world"
    0
"#,
        "No implementation found for operator '+' on type F64",
    );
}

#[test]
fn test_type_error_annotation_mismatch() {
    compile_should_fail(
        r#"
main = ->
    x: I64 = "hello"
    0
"#,
        "Type annotation mismatch",
    );
}

#[test]
fn test_type_error_bool_plus_int() {
    compile_should_fail(
        r#"
main = ->
    x = true + 42
    0
"#,
        "No implementation found for operator '+' on type Bool",
    );
}

#[test]
fn test_as_cast_int_to_float() {
    // `x as F64` — integer to float
    let output = compile_and_run(
        r#"
main = ->
    x = 42
    f = x as F64
    f.println!
    0
"#,
    );
    assert_eq!(output.trim(), "42");
}

#[test]
fn test_as_cast_float_to_int() {
    // `f as I64` — float to int (truncates)
    let output = compile_and_run(
        r#"
main = ->
    f = 3.7
    i = f as I64
    i.println!
    0
"#,
    );
    assert_eq!(output.trim(), "3");
}

#[test]
fn test_as_cast_int_narrowing() {
    // `val as I32` — i64 narrowed to i32
    let output = compile_and_run(
        r#"
main = ->
    x = 1000
    y = x as I32
    y.println!
    0
"#,
    );
    assert_eq!(output.trim(), "1000");
}

#[test]
fn test_as_cast_u8_to_char() {
    // byte as Char — same as old ~U8ToChar
    let output = compile_and_run(
        r#"
main = ->
    bytes = [104 as U8]
    c = bytes[0] as Char
    c.println!
    0
"#,
    );
    assert_eq!(output.trim(), "h");
}

#[test]
fn test_char_show_literal() {
    let output = compile_and_run(
        r#"
main = ->
    'A'.println!
    0
"#,
    );
    assert_eq!(output.trim(), "A");
}

#[test]
fn test_as_cast_in_binop() {
    // `a + b as I64` — cast binds to RHS only (same-type cast verifies precedence)
    let output = compile_and_run(
        r#"
main = ->
    a = 10
    b = 5
    result = a + b as I64
    result.println!
    0
"#,
    );
    assert_eq!(output.trim(), "15");
}

#[test]
fn test_as_cast_ptr_to_int() {
    // Pointer-to-int cast: pointer value cast to I64 should be non-zero
    let output = compile_and_run(
        r#"
main = ->
    s = "hello"
    p = s.as_ptr!
    i = p as I64
    if i > 0
        "ok".println!
    0
"#,
    );
    assert_eq!(output.trim(), "ok");
}

#[test]
fn test_type_conversion_via_as() {
    // `to_float` and `to_int` now use `as` under the hood
    let output = compile_and_run(
        r#"
main = ->
    x = 7
    f = to_float x
    f.println!
    y = 2.9
    i = to_int y
    i.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "7");
    assert_eq!(lines[1], "2");
}

#[test]
fn test_type_alias_survives_collection_and_lowering() {
    let output = compile_and_run(
        r#"
type Number = Identity I64
type Identity T = T

identity: Number -> Number
identity = value -> value

main = ->
    identity 42 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_type_alias_cycle_is_rejected() {
    compile_should_fail(
        r#"
type Left = Right
type Right = Left

main = -> 0
"#,
        "type alias cycle",
    );
}

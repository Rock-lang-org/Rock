use crate::support::*;

#[test]
fn test_vec_raw_len_field_access_is_rejected_outside_impl() {
    compile_should_fail(
        r#"
main = ->
    mut v = Vec::new!
    v.push 1
    x = v.raw_len
    0
"#,
        "Field 'raw_len' of struct 'Vec' is private",
    );
}

#[test]
fn test_string_raw_len_field_access_is_rejected_outside_impl() {
    compile_should_fail(
        r#"
main = ->
    s = String::from "hello"
    x = s.raw_len
    0
"#,
        "Field 'raw_len' of struct 'String' is private",
    );
}

#[test]
fn test_public_struct_field_access_succeeds_outside_impl() {
    let output = compile_and_run(
        r#"
struct Point
    < x: I64
    < y: I64

main = ->
    p = Point
        x: 10
        y: 20
    p.x.println!
    p.y.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["10", "20"]);
}

#[test]
fn test_private_struct_field_access_is_rejected_outside_impl() {
    compile_should_fail(
        r#"
struct Point
    < x: I64
    y: I64

impl Point
    make = x, y ->
        Point
            x: x
            y: y

main = ->
    p = Point::make 10, 20
    p.y.println!
    0
"#,
        "Field 'y' of struct 'Point' is private",
    );
}

#[test]
fn test_private_struct_field_access_is_rejected_through_inferred_receiver() {
    compile_should_fail(
        r#"
struct Point
    < x: I64
    y: I64

impl Point
    make = x, y ->
        Point
            x: x
            y: y

get_y = p -> p.y

main = ->
    p = Point::make 10, 20
    get_y p .println!
    0
"#,
        "Field 'y' of struct 'Point' is private",
    );
}

#[test]
fn test_private_struct_field_access_succeeds_inside_own_impl() {
    let output = compile_and_run(
        r#"
struct Point
    < x: I64
    y: I64

impl Point
    make = x, y ->
        Point
            x: x
            y: y
    @sum = -> @x + @y

main = ->
    p = Point::make 10, 20
    p.sum! .println!
    0
"#,
    );

    assert_eq!(output.trim(), "30");
}

#[test]
fn test_private_struct_construction_is_rejected_outside_impl() {
    compile_should_fail(
        r#"
struct Point
    < x: I64
    y: I64

main = ->
    p = Point
        x: 10
        y: 20
    p.x.println!
    0
"#,
        "Cannot construct struct 'Point' because it has private fields",
    );
}

#[test]
fn test_private_struct_construction_succeeds_inside_own_impl() {
    let output = compile_and_run(
        r#"
struct Point
    < x: I64
    y: I64

impl Point
    make = x, y ->
        Point
            x: x
            y: y

main = ->
    p = Point::make 10, 20
    p.x.println!
    0
"#,
    );

    assert_eq!(output.trim(), "10");
}

#[test]
fn test_private_struct_pattern_is_rejected_outside_impl() {
    compile_should_fail(
        r#"
struct Point
    < x: I64
    y: I64

impl Point
    make = x, y ->
        Point
            x: x
            y: y

inspect = p ->
    match p
        Point y: _ => 0

main = ->
    p = Point::make 10, 20
    inspect p .println!
    0
"#,
        "Field 'y' of struct 'Point' is private",
    );
}

#[test]
fn test_private_struct_pattern_succeeds_inside_own_impl() {
    let output = compile_and_run(
        r#"
struct Point
    < x: I64
    y: I64

impl Point
    make = x, y ->
        Point
            x: x
            y: y
    @sum = ->
        match *self
            Point x: x, y: y => x + y

main = ->
    p = Point::make 10, 20
    p.sum! .println!
    0
"#,
    );

    assert_eq!(output.trim(), "30");
}

#[test]
fn test_struct_pattern_rejects_refutable_field_subpatterns() {
    compile_should_fail(
        r#"
struct Point
    < x: I64
    < y: I64

main = ->
    p = Point
        x: 10
        y: 20
    match p
        Point x: 10, y: y => y
"#,
        "Struct field patterns currently only support bindings and wildcards",
    );
}

#[test]
fn test_struct_pattern_rejects_mismatched_struct_type() {
    compile_should_fail(
        r#"
struct Point
    < x: I64

struct Other
    < x: I64

main = ->
    p = Point
        x: 10
    match p
        Other x: x => x
        Point x: x => x + 1
"#,
        "pattern type mismatch",
    );
}

#[test]
fn test_generic_struct_pattern_infers_scrutinee_type() {
    let output = compile_and_run(
        r#"
struct Box T
    < value: T

unwrap = box ->
    match box
        Box value: value => value + 1

main = ->
    unwrap (Box
        value: 10) .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "11");
}

#[test]
fn test_generic_struct_pattern_binds_non_i64_payload() {
    let output = compile_and_run(
        r#"
struct Box T
    < value: T

unwrap = box ->
    match box
        Box value: value => value

main = ->
    unwrap (Box
        value: "hello") .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "hello");
}

#[test]
fn test_trait_default_method_substitutes_trait_generic_struct_pattern_types() {
    let output = compile_and_run(
        r#"
struct Box T
    < value: T

struct Counter
    < seed: I64

trait UnwrapBox T
    @unwrap: Box T -> T
    @unwrap = boxed ->
        match boxed
            Box value: value => value

impl UnwrapBox I64 for Counter

main = ->
    receiver = Counter
        seed: 0
    boxed = Box
        value: 7
    unwrapped = receiver.unwrap boxed
    unwrapped.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "7");
}

#[test]
fn test_all_public_struct_construction_succeeds_outside_impl() {
    compile_should_pass(
        r#"
struct Point
    < x: I64
    < y: I64

main = ->
    p = Point
        x: 10
        y: 20
    p.x.println!
    0
"#,
    );
}

use std::sync::atomic::Ordering;

use crate::support::*;

#[test]
fn test_attached_postfix_call_applies_to_parenthesized_argument() {
    let output = compile_and_run(
        r#"
double = n -> n + n

main = !->
    n = double (Option::Some 21).unwrap_or 0
    n.println!
"#,
    );
    assert_eq!(output, "42\n");
}

#[test]
fn test_bind_callback_retries_outer_method_after_inner_method_resolves() {
    let output = compile_and_run(
        r#"
> std::io::fs::read_to_string

struct Turn
    < dir: I64

impl Turn
    new = letter ->
        Turn
            dir: if letter == 'R' then 1 else 0

parse = text ->
    mut turns = Vec::new!
    for item in text.split "\n"
        letter = item.char_at 0 .unwrap_or 'a'
        turns.push (Turn::new letter)
    Ok turns

load = -> read_to_string "" >>= parse

main = !->
    match load!
        Ok turns => turns.len!.println!
        Err _ => "read failed".println!
"#,
    );
    assert_eq!(output, "read failed\n");
}

const SHAPES: &str = r#"
struct Circle
    < r: I64

struct Square
    < s: I64

impl Circle
    @area: I64
    @area = -> self.r

impl Square
    @area: I64
    @area = -> self.s
"#;

fn compile_messages(source: &str) -> Vec<String> {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("test.rk");
    std::fs::write(&entry, source).unwrap();
    let result = rock_lib::compile(&test_config(entry, dir.clone()));
    let _ = std::fs::remove_dir_all(&dir);
    let diagnostics = result.expect_err("expected compilation to fail");
    diagnostics
        .0
        .iter()
        .map(|diagnostic| diagnostic.message.clone())
        .collect()
}

#[test]
fn test_caller_fixes_receiver_shared_by_two_impls() {
    let output = compile_and_run(&format!(
        "{SHAPES}
area_of = shape -> shape.area!

main = !->
    circle = Circle
        r: 3
    (area_of circle).println!
"
    ));
    assert_eq!(output, "3\n");
}

#[test]
fn test_caller_fixes_receiver_through_higher_order_call() {
    let output = compile_and_run(&format!(
        "{SHAPES}
area_of = shape -> shape.area!
apply = f, value -> f value

main = !->
    square = Square
        s: 4
    (apply area_of, square).println!
"
    ));
    assert_eq!(output, "4\n");
}

#[test]
fn test_unfixed_receiver_is_not_guessed_from_a_unique_method_name() {
    compile_should_fail(
        r#"
struct Circle
    < r: I64

impl Circle
    @area: I64
    @area = -> self.r

area_of = shape -> shape.area!

main = !-> ()
"#,
        "cannot infer the receiver type of 'area'",
    );
}

#[test]
fn test_receiver_used_with_two_types_is_reported() {
    compile_should_fail(
        &format!(
            "{SHAPES}
area_of = shape -> shape.area!

main = !->
    circle = Circle
        r: 3
    square = Square
        s: 4
    (area_of circle).println!
    (area_of square).println!
"
        ),
        "cannot infer the receiver type of 'area'",
    );
}

#[test]
fn test_slice_of_caller_fixed_receiver_reduces_to_str() {
    let output = compile_and_run(
        r#"
tail_len = s -> (&s[1..]).len!

tail_number = s ->
    rest = &s[1..]
    rest.parse_i64!.unwrap_or 0

main = !->
    (tail_len (String::from "abc")).println!
    (tail_number (String::from "R42")).println!
"#,
    );
    assert_eq!(output, "2\n42\n");
}

#[test]
fn test_slice_of_caller_fixed_receiver_satisfies_trait_bounds() {
    let output = compile_and_run(
        r#"
parse = content ->
    mut out = Vec::new!
    for item in content.split "\n"
        rest = &item[1..]
        value = match rest.try_into!
            Ok n => n
            Err _ => 0
        out.push value
    out

main = !->
    (parse (String::from "R12\nL3")).println!
"#,
    );
    assert_eq!(output, "[12, 3]\n");
}

#[test]
fn test_bind_callback_returning_wrong_carrier_is_the_reported_cause() {
    let messages = compile_messages(
        r#"
> std::io::fs::read_to_string

parse = content ->
    mut out = Vec::new!
    for line in content.split "\n"
        out.push line
    out

load = -> read_to_string "input" >>= parse

main = !->
    for line in load!
        line.println!
"#,
    );
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert!(
        messages[0].contains("does not implement trait `FnMut String, (Result _, IoError)`"),
        "{messages:?}"
    );
}

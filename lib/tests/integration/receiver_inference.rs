use std::sync::atomic::Ordering;

use crate::support::*;

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

use crate::support::*;

#[test]
fn test_vec_get_returns_optional_reference() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 10
    v.push 20

    match (v.get 0)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    match (v.get 5)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["10", "-1"]);
}

#[test]
fn test_vec_get_borrow_blocks_later_mutation() {
    compile_should_fail(
        r#"
main = ->
    mut v = Vec::new!
    v.push 1
    saved = v.get 0
    v.push 2
    match saved
        Option::Some val => *val .println!
        Option::None => 0.println!
    0
"#,
        "borrow",
    );
}

#[test]
fn vec_deref_does_not_return_reference_to_temporary_slice() {
    compile_should_fail(
        r#"
> std::collections::Vec

bad: Vec I64 -> &I64
bad = v -> &v.len!

main = ->
    mut v = Vec::new!
    v.push 1
    rr = bad v
    0
"#,
        "temporary",
    );
}

#[test]
fn test_vec_push() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 1
    v.push 2
    v.push 3
    v.len! .println!

    match (v.get 0)
        Option::Some val => *val .println!
        Option::None => 0.println!
    match (v.get 1)
        Option::Some val => *val .println!
        Option::None => 0.println!
    match (v.get 2)
        Option::Some val => *val .println!
        Option::None => 0.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "3");
    assert_eq!(lines[1], "1");
    assert_eq!(lines[2], "2");
    assert_eq!(lines[3], "3");
}

#[test]
fn test_std_vec_show_formats_owned_elements() {
    let output = compile_and_run(
        r#"
main = ->
    mut values: Vec String = Vec::new!
    values.push (String::from "alpha")
    values.push (String::from "two words")
    values.println!
    0
"#,
    );

    assert_eq!(output.trim(), "[alpha, two words]");
}

#[test]
fn test_std_vec_rejects_zero_sized_elements_on_push() {
    let (_output, success) = compile_and_run_with_status(
        r#"
struct Empty

main = ->
    mut v = Vec::new!
    v.push Empty
    0
"#,
    );

    assert!(!success, "Vec::push should reject zero-sized elements");
}

#[test]
fn test_std_vec_new_as_slice_uses_non_null_buffer() {
    let output = compile_and_run(
        r#"
main = ->
    v: Vec I64 = Vec::new!
    slice = v.as_slice!
    ((~ArrPtr (*slice)) as I64) != 0 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "true");
}

#[test]
fn test_std_vec_push_uses_non_null_buffer() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 1
    slice = v.as_slice!
    ((~ArrPtr (*slice)) as I64) != 0 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "true");
}

#[test]
fn test_std_vec_index_works() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 4
    v.push 7
    v[1].println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_std_vec_index_out_of_bounds_traps() {
    let (_output, success) = compile_and_run_with_status(
        r#"
main = ->
    mut v = Vec::new!
    v.push 10
    v[1].println!
    0
"#,
    );

    assert!(!success, "Vec index out of bounds should trap");
}

#[test]
fn test_std_vec_shared_binding_index_works() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 4
    v.push 7
    shared = v
    shared[1].println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_vec_as_slice_index_works_without_stored_slice_field() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 10
    v.push 20
    slice = v.as_slice!
    slice[1] .println!
    0
"#,
    );

    assert_eq!(output.trim(), "20");
}

#[test]
fn test_vec_as_slice_borrows_existing_buffer() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 11
    v.push 22
    slice = v.as_slice!
    slice[1] .println!
    0
"#,
    );

    assert_eq!(output.trim(), "22");
}

#[test]
fn test_vec_as_slice_borrow_blocks_later_mutation() {
    compile_should_fail(
        r#"
main = ->
    mut v = Vec::new!
    v.push 1
    slice = v.as_slice!
    v.push 2
    slice[0] .println!
    0
"#,
        "borrow",
    );
}

#[test]
fn test_vec_swap_remove_moves_owned_value() {
    let output = compile_and_run(
        r#"
main = ->
    mut values: Vec String = Vec::new!
    values.push (String::from "first")
    values.push (String::from "second")
    values.push (String::from "third")
    match values.swap_remove 1
        Option::Some value => value.println!
        Option::None => "missing".println!
    values.len!.println!
    values[1].println!
    0
"#,
    );

    assert_eq!(
        output.trim().lines().collect::<Vec<_>>(),
        ["second", "2", "third"]
    );
}

#[test]
fn test_vec_set_get() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 10
    v.push 20
    v.push 30
    v.set 1, 99
    match (v.get 0)
        Option::Some val => *val .println!
        Option::None => 0.println!
    match (v.get 1)
        Option::Some val => *val .println!
        Option::None => 0.println!
    match (v.get 2)
        Option::Some val => *val .println!
        Option::None => 0.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "99");
    assert_eq!(lines[2], "30");
}

#[test]
fn test_std_vec_drops_initialized_elements() {
    let output = compile_and_run(
        r#"
struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    mut v = Vec::new!
    v.push (Tracked
        value: 1)
    v.push (Tracked
        value: 2)
    v.push (Tracked
        value: 3)
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["3", "2", "1"]);
}

#[test]
fn test_std_vec_set_drops_replaced_element_once() {
    let output = compile_and_run(
        r#"
struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    mut v = Vec::new!
    v.push (Tracked
        value: 1)
    v.push (Tracked
        value: 2)
    v.set 0, (Tracked
        value: 9)
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["1", "2", "9"]);
}

#[test]
fn test_std_vec_set_out_of_bounds_drops_ignored_value_once() {
    let output = compile_and_run(
        r#"
struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    mut v = Vec::new!
    v.push (Tracked
        value: 1)
    v.set (-1), (Tracked
        value: 8)
    v.set 3, (Tracked
        value: 9)
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["8", "9", "1"]);
}

#[test]
fn test_std_vec_growth_drops_initialized_elements_once() {
    let output = compile_and_run(
        r#"
struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    mut v = Vec::new!
    v.push (Tracked
        value: 1)
    v.push (Tracked
        value: 2)
    v.push (Tracked
        value: 3)
    v.push (Tracked
        value: 4)
    v.push (Tracked
        value: 5)
    v.push (Tracked
        value: 6)
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["6", "5", "4", "3", "2", "1"]);
}

#[test]
fn test_vec_get_option() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 10
    v.push 20
    match (v.get 0)
        Option::Some val => *val .println!
        Option::None => "None".println!
    match (v.get 5)
        Option::Some val => *val .println!
        Option::None => "None".println!
    match (v.get 1)
        Option::Some val => *val .println!
        Option::None => 99.println!
    match (v.get 99)
        Option::Some val => *val .println!
        Option::None => 99.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "None");
    assert_eq!(lines[2], "20");
    assert_eq!(lines[3], "99");
}

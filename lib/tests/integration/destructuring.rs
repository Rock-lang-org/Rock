use crate::support::*;

#[test]
fn array_patterns_bind_elements_and_owned_rest() {
    let output = compile_and_run(
        r#"
main = !->
    [first, ..middle, last] = [1, 2, 3, 4]
    first.println!
    middle.println!
    last.println!
"#,
    );
    assert_eq!(output.trim(), "1\n[2, 3]\n4");
}

#[test]
fn array_patterns_borrow_elements_and_rest() {
    let output = compile_and_run(
        r#"
main = !->
    values = [1, 2, 3, 4]
    [first, ..middle, last] = &values
    (*first + *last).println!
    middle.println!
    values.println!
"#,
    );
    assert_eq!(output.trim(), "5\n[2, 3]\n[1, 2, 3, 4]");
}

#[test]
fn slice_patterns_check_lengths_and_borrow_rest() {
    let output = compile_and_run(
        r#"
describe: &[I64] -> I64
describe = values -> match values
    [] => 0
    [one] => *one
    [first, ..middle, last] => *first + middle.len! + *last

main = !->
    values: Vec I64 = [1, 2, 3, 4].into!
    (describe values.as_slice!).println!
    one: Vec I64 = [8].into!
    (describe one.as_slice!).println!
    empty: Vec I64 = Vec::new!
    (describe empty.as_slice!).println!
"#,
    );
    assert_eq!(output.trim(), "7\n8\n0");
}

#[test]
fn nested_refutable_patterns_check_every_field() {
    let output = compile_and_run(
        r#"
struct Data
    < values: (I64, Option I64)

main = !->
    data = Data
        values: (1, Option::Some 9)
    result = match &data
        Data values: (0, Option::Some n) => *n
        Data values: (1, Option::Some 8) => 80
        Data values: (1, Option::Some n) => *n
        _ => 0
    result.println!
"#,
    );
    assert_eq!(output.trim(), "9");
}

#[test]
fn array_patterns_in_parameters_and_loops() {
    let output = compile_and_run(
        r#"
sum: &[I64; 2] -> I64
sum = [a, b] -> *a + *b

main = !->
    values = [3, 4]
    (sum &values).println!
    for [a, b] in [[1, 2], [3, 4]]
        (a + b).println!
"#,
    );
    assert_eq!(output.trim(), "7\n3\n7");
}

#[test]
fn array_patterns_reject_multiple_rests() {
    compile_should_fail(
        r#"
main = !->
    [..a, ..b] = [1, 2]
    ()
"#,
        "only one rest pattern",
    );
}

#[test]
fn array_patterns_move_owned_strings_once() {
    let output = compile_and_run(
        r#"
main = !->
    [first, ..rest] = [(String::from "a"), (String::from "b"), (String::from "c")]
    first.println!
    rest.println!
"#,
    );
    assert_eq!(output.trim(), "a\n[b, c]");
}

#[test]
fn mutable_slice_pattern_references_update_storage() {
    let output = compile_and_run(
        r#"
change: &mut [I64] -> ()
change = values -> match values
    [only] => *only = 42
    _ => ()

main = !->
    mut values = [1]
    change &mut values
    values.println!
"#,
    );
    assert_eq!(output.trim(), "[42]");
}

#[test]
fn mutable_array_pattern_cannot_be_used_to_modify_shared_storage() {
    compile_should_fail(
        r#"
main = !->
    values = [1]
    [only] = &values
    *only = 42
"#,
        "borrow conflict on this value",
    );
}

#[test]
fn slice_patterns_with_fixed_elements_are_refutable_in_assignments() {
    compile_should_fail(
        r#"
main = !->
    values: Vec I64 = [1, 2].into!
    [first, ..rest] = values.as_slice!
    first.println!
"#,
        "refutable pattern in assignment",
    );
}

#[test]
fn mutable_slice_rest_binding_updates_original_storage() {
    let output = compile_and_run(
        r#"
change: &mut [I64] -> ()
change = values -> match values
    [..mut all] => all[0] = 42

main = !->
    mut values = [1, 2]
    change &mut values
    values.println!
"#,
    );
    assert_eq!(output.trim(), "[42, 2]");
}

#[test]
fn owned_array_pattern_drops_ignored_elements_exactly_once() {
    let output = compile_and_run(
        r#"
struct Tracked
    < id: I64

impl Drop for Tracked
    ~@drop = !-> self.id.println!

main = !->
    a = Tracked
        id: 1
    b = Tracked
        id: 2
    c = Tracked
        id: 3
    [first, ..] = [a, b, c]
    first.id.println!
"#,
    );
    let mut lines: Vec<_> = output.lines().collect();
    lines.sort();
    assert_eq!(lines, ["1", "1", "2", "3"]);
}

#[test]
fn array_patterns_allow_empty_rests_and_anonymous_rests() {
    let output = compile_and_run(
        r#"
main = !->
    [a, ..empty, b] = [3, 4]
    (a + b).println!
    empty.println!
    values = [8, 9]
    match &values
        [0, ..] => 0.println!
        [first, .., last] => (*first + *last).println!
        _ => 0.println!
"#,
    );
    assert_eq!(output.trim(), "7\n[]\n17");
}

#[test]
fn matching_through_nested_references_borrows_payloads() {
    let output = compile_and_run(
        r#"
main = !->
    value = Option::Some 42
    borrowed = &value
    result = match &borrowed
        Option::Some n => *n
        Option::None => 0
    result.println!
"#,
    );
    assert_eq!(output.trim(), "42");
}

// Assignment (`pattern = value`)

#[test]
fn destructuring_a_shared_reference_binds_element_references() {
    let output = compile_and_run(
        r#"
main = !->
    pair = (1, String::from "one")
    (number, word) = &pair
    (*number + 1).println!
    word.println!
    pair.1.println!
"#,
    );
    assert_eq!(output.trim(), "2\none\none");
}

#[test]
fn destructuring_a_mutable_reference_binds_mutable_element_references() {
    let output = compile_and_run(
        r#"
main = !->
    mut pair = (1, 2)
    (left, right) = &mut pair
    *left = *left + 10
    *right = *right + 20
    pair.0.println!
    pair.1.println!
"#,
    );
    assert_eq!(output.trim(), "11\n22");
}

#[test]
fn mutable_element_references_keep_the_tuple_borrowed() {
    compile_should_fail(
        r#"
main = !->
    mut pair = (1, 2)
    (left, _) = &mut pair
    pair.0.println!
    *left = 5
"#,
        "borrow conflict on this value",
    );
}

#[test]
fn destructuring_a_shared_reference_cannot_move_out() {
    compile_should_fail(
        r#"
main = !->
    pair = (String::from "a", 1)
    (word, _) = &pair
    owned: String = word
    owned.println!
"#,
        "Type mismatch",
    );
}

#[test]
fn nested_tuple_patterns_destructure_values_and_references() {
    let output = compile_and_run(
        r#"
main = !->
    triple = (1, (2, 3))
    (a, (b, c)) = triple
    (a + b + c).println!
    (x, (y, z)) = &triple
    (*x * *y * *z).println!
"#,
    );
    assert_eq!(output.trim(), "6\n6");
}

#[test]
fn struct_patterns_destructure_values_and_mutable_references() {
    let output = compile_and_run(
        r#"
struct Point
    < x: I64
    < y: I64

main = !->
    mut point = Point
        x: 1
        y: 2
    Point x: a, y: b = point
    (a + b).println!
    Point x: px, y: _ = &mut point
    *px = 40
    (point.x + point.y).println!
"#,
    );
    assert_eq!(output.trim(), "3\n42");
}

#[test]
fn struct_field_patterns_can_nest() {
    let output = compile_and_run(
        r#"
struct Segment
    < ends: (I64, I64)

main = !->
    segment = Segment
        ends: (3, 4)
    Segment ends: (from, to) = &segment
    (*from + *to).println!
"#,
    );
    assert_eq!(output.trim(), "7");
}

#[test]
fn explicit_reference_patterns_copy_the_referenced_values() {
    let output = compile_and_run(
        r#"
main = !->
    pair = (3, 4)
    &(a, b) = &pair
    (a + b).println!
"#,
    );
    assert_eq!(output.trim(), "7");
}

#[test]
fn refutable_patterns_are_rejected_in_assignments() {
    compile_should_fail(
        r#"
main = !->
    maybe = Option::Some 1
    Option::Some value = maybe
    value.println!
"#,
        "refutable pattern in assignment",
    );
}

// match

#[test]
fn matching_a_tuple_reference_binds_element_references() {
    let output = compile_and_run(
        r#"
main = !->
    pair = (2, String::from "two")
    match &pair
        (number, word) =>
            (*number + 1).println!
            word.println!
    pair.1.println!
"#,
    );
    assert_eq!(output.trim(), "3\ntwo\ntwo");
}

#[test]
fn matching_a_struct_reference_binds_field_references() {
    let output = compile_and_run(
        r#"
struct Point
    < x: I64
    < y: I64

main = !->
    point = Point
        x: 1
        y: 2
    match &point
        Point x: a, y: b => (*a + *b).println!
"#,
    );
    assert_eq!(output.trim(), "3");
}

#[test]
fn matching_an_enum_reference_borrows_its_payload() {
    let output = compile_and_run(
        r#"
main = !->
    maybe = Option::Some (String::from "kept")
    match &maybe
        Option::Some text => text.println!
        Option::None => ()
    maybe.println!
"#,
    );
    assert_eq!(output.trim(), "kept\nSome(kept)");
}

#[test]
fn matching_a_mutable_enum_reference_updates_its_payload() {
    let output = compile_and_run(
        r#"
main = !->
    mut maybe = Option::Some 1
    match &mut maybe
        Option::Some value => *value = *value + 41
        Option::None => ()
    maybe.println!
"#,
    );
    assert_eq!(output.trim(), "Some(42)");
}

#[test]
fn catch_all_arms_of_a_reference_match_reborrow_it() {
    let output = compile_and_run(
        r#"
describe: &Option I64 -> I64
describe = maybe -> match maybe
    Option::Some value => *value
    Option::None => 0

main = !->
    maybe = Option::Some 5
    match &maybe
        Option::None => "none".println!
        other => (describe other).println!
    maybe.println!
"#,
    );
    assert_eq!(output.trim(), "5\nSome(5)");
}

// Parameters and loops

#[test]
fn lambda_parameters_destructure_tuples() {
    let output = compile_and_run(
        r#"
main = !->
    add = (a, b) -> a + b
    (add (3, 4)).println!
"#,
    );
    assert_eq!(output.trim(), "7");
}

#[test]
fn function_parameters_destructure_values_and_references() {
    let output = compile_and_run(
        r#"
struct Point
    < x: I64
    < y: I64

sum: (I64, I64) -> I64
sum = (a, b) -> a + b

sum_ref: &(I64, I64) -> I64
sum_ref = (a, b) -> *a + *b

norm: &Point -> I64
norm = (Point x: x, y: y) -> *x * *x + *y * *y

main = !->
    pair = (3, 4)
    (sum pair).println!
    (sum_ref &pair).println!
    point = Point
        x: 3
        y: 4
    (norm &point).println!
"#,
    );
    assert_eq!(output.trim(), "7\n7\n25");
}

#[test]
fn for_loops_destructure_array_elements() {
    let output = compile_and_run(
        r#"
main = !->
    for (a, b) in [(1, 2), (3, 4)]
        (a + b).println!
"#,
    );
    assert_eq!(output.trim(), "3\n7");
}

#[test]
fn for_loops_destructure_borrowed_slice_elements() {
    let output = compile_and_run(
        r#"
main = !->
    values: Vec (I64, I64) = [(1, 2), (3, 4)].into!
    for (a, b) in values.as_slice!
        (*a + *b).println!
"#,
    );
    assert_eq!(output.trim(), "3\n7");
}

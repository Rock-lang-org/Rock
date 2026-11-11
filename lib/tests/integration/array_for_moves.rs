// `for x in array` over an owned array consumes it: each element moves into
// `x` exactly once and drops at the end of its pass; early exits drop only the
// not-yet-yielded elements.
//
// Bodies print `x.value` (1..4); `Noisy` drops print `value + 100` (101..104).
use crate::support::*;

const NOISY: &str = r#"
struct Noisy
    < value: I64
impl Drop for Noisy
    ~@drop = !->
        (self.value + 100).println!

mk: I64 -> Noisy
mk = v -> Noisy
    value: v
"#;

fn with_noisy(body: &str) -> String {
    format!("{NOISY}\n{body}")
}

#[test]
fn test_array_for_full_loop_drops_each_element_once_in_order() {
    let output = compile_and_run(&with_noisy(
        r#"
main = !->
    arr = [(mk 1), (mk 2), (mk 3)]
    for x in arr
        x.value.println!
    "done".println!
"#,
    ));
    assert_eq!(output, "1\n101\n2\n102\n3\n103\ndone\n");
}

#[test]
fn test_array_for_break_drops_remaining_elements_once() {
    let output = compile_and_run(&with_noisy(
        r#"
main = !->
    arr = [(mk 1), (mk 2), (mk 3)]
    for x in arr
        x.value.println!
        if x.value == 2
            break
    "done".println!
"#,
    ));
    assert_eq!(output, "1\n101\n2\n102\n103\ndone\n");
}

#[test]
fn test_array_for_early_return_drops_remaining_elements_once() {
    let output = compile_and_run(&with_noisy(
        r#"
scan: () -> I64
scan = ->
    arr = [(mk 1), (mk 2), (mk 3)]
    for x in arr
        x.value.println!
        if x.value == 1
            return 42
    0

main = !->
    scan!.println!
    "done".println!
"#,
    ));
    assert_eq!(output, "1\n101\n102\n103\n42\ndone\n");
}

#[test]
fn test_array_for_try_exit_drops_remaining_elements_once() {
    let output = compile_and_run(&with_noisy(
        r#"
check: I64 -> Result I64, I64
check = v ->
    if v == 2
        Result::Err 99
    else
        Result::Ok v

scan: () -> Result I64, I64
scan = ->
    arr = [(mk 1), (mk 2), (mk 3)]
    for x in arr
        r = check x.value
        v = r?
        (v + 1000).println!
    Result::Ok 0

main = !->
    match scan!
        Result::Ok _ => "ok".println!
        Result::Err e => e.println!
"#,
    ));
    assert_eq!(output, "1001\n101\n102\n103\n99\n");
}

#[test]
fn test_array_for_empty_body_just_consumes() {
    let output = compile_and_run(&with_noisy(
        r#"
main = !->
    arr = [(mk 1), (mk 2)]
    for x in arr
        ()
    "done".println!
"#,
    ));
    assert_eq!(output, "101\n102\ndone\n");
}

#[test]
fn test_array_for_nested_arrays_of_noisy() {
    let output = compile_and_run(&with_noisy(
        r#"
main = !->
    grid = [[(mk 1), (mk 2)], [(mk 3), (mk 4)]]
    for row in grid
        "row".println!
        for x in row
            x.value.println!
    "done".println!
"#,
    ));
    assert_eq!(
        output,
        "row\n1\n101\n2\n102\nrow\n3\n103\n4\n104\ndone\n"
    );
}

#[test]
fn test_array_for_nested_break_drops_inner_and_outer_remaining() {
    let output = compile_and_run(&with_noisy(
        r#"
main = !->
    grid = [[(mk 1), (mk 2)], [(mk 3), (mk 4)]]
    for row in grid
        for x in row
            x.value.println!
            break
    "done".println!
"#,
    ));
    assert_eq!(output, "1\n101\n102\n3\n103\n104\ndone\n");
}

#[test]
fn test_array_for_rejects_use_of_cleanup_array_after_loop() {
    compile_should_fail(
        &with_noisy(
            r#"
main = !->
    arr = [(mk 1), (mk 2)]
    for x in arr
        x.value.println!
    arr.len!.println!
"#,
        ),
        "moved value",
    );
}

#[test]
fn test_array_for_copy_element_array_loop_unchanged() {
    // Rock arrays are never Copy (TypeFacts::is_copy), so the source array is
    // moved into the loop even for Copy elements; only the loop itself is checked.
    let output = compile_and_run(
        r#"
main = !->
    arr = [1, 2, 3]
    mut sum = 0
    for x in arr
        sum = sum + x
    sum.println!
"#,
    );
    assert_eq!(output, "6\n");
}

#[test]
fn test_array_for_explicit_element_move_out_of_cleanup_array_still_rejected() {
    // `arr[i]` lowers through the index trait to a reference, so the user-level
    // diagnostic is the reference-move one; the raw projection rule is covered by
    // the borrowck unit tests.
    compile_should_fail(
        &with_noisy(
            r#"
main = !->
    arr = [(mk 1), (mk 2)]
    taken = arr[1]
    taken.value.println!
"#,
        ),
        "out of a reference",
    );
}

#[test]
fn test_array_for_vec_from_array_of_noisy_drops_each_once() {
    let output = compile_and_run(&with_noisy(
        r#"
main = !->
    values: Vec Noisy = Vec::from [(mk 1), (mk 2), (mk 3)]
    values.len!.println!
    "done".println!
"#,
    ));
    // Vec's Drop impl drops elements back to front.
    assert_eq!(output, "3\ndone\n103\n102\n101\n");
}

#[test]
fn test_array_for_into_vec_of_noisy_drops_each_once() {
    let output = compile_and_run(&with_noisy(
        r#"
main = !->
    values: Vec Noisy = [(mk 1), (mk 2)].into!
    values.len!.println!
    "done".println!
"#,
    ));
    assert_eq!(output, "2\ndone\n102\n101\n");
}

#[test]
fn test_array_for_strings_pushed_into_vec() {
    let output = compile_and_run(
        r#"
main = !->
    words = [(String::from "a"), (String::from "b"), (String::from "c")]
    mut out: Vec String = Vec::new!
    for w in words
        out.push w
    out.len!.println!
    out[0].println!
    out[2].println!
"#,
    );
    assert_eq!(output, "3\na\nc\n");
}

#[test]
fn test_array_for_vecs_pushed_into_outer_vec() {
    let output = compile_and_run(
        r#"
main = !->
    mut a: Vec I64 = Vec::new!
    a.push 1
    mut b: Vec I64 = Vec::new!
    b.push 2
    b.push 3
    arr = [a, b]
    mut out: Vec (Vec I64) = Vec::new!
    for v in arr
        out.push v
    out.len!.println!
    out[1].len!.println!
"#,
    );
    assert_eq!(output, "2\n2\n");
}

// Several early exits from one loop (`return` in the body, `break`) plus a
// `return` after the loop: each path drops every element exactly once.
#[test]
fn test_array_for_multiple_early_exits_and_post_loop_return() {
    let output = compile_and_run(&with_noisy(
        r#"
scan: (I64, I64) -> I64
scan = pair ->
    n = pair.0
    brk = pair.1
    arr = [(mk 1), (mk 2), (mk 3), (mk 4)]
    for x in arr
        x.value.println!
        if x.value == n
            return 10
        if x.value == brk
            break
    "after".println!
    if n == 100
        return 20
    30

main = !->
    (scan (1, 99)).println!
    (scan (0, 3)).println!
    (scan (100, 99)).println!
    (scan (2, 99)).println!
"#,
    ));
    assert_eq!(
        output,
        concat!(
            // return on the first element: binding then the whole tail
            "1\n101\n102\n103\n104\n10\n",
            // break on the third: binding then the last element, then the code after
            "1\n101\n2\n102\n3\n103\n104\nafter\n30\n",
            // full loop, then the post-loop return
            "1\n101\n2\n102\n3\n103\n4\n104\nafter\n20\n",
            // return on the second element
            "1\n101\n2\n102\n103\n104\n10\n",
        )
    );
}

// `?` inside the iterated expression exits before the array exists: nothing
// drops on the Err path; the Ok path drops each element once.
#[test]
fn test_array_for_try_in_iterated_expression() {
    let output = compile_and_run(&with_noisy(
        r#"
build: I64 -> Result [Noisy; 2], I64
build = v ->
    if v == 0
        Result::Err 99
    else
        Result::Ok [(mk 1), (mk 2)]

run: I64 -> Result I64, I64
run = v ->
    for x in (build v)?
        x.value.println!
    Result::Ok 0

main = !->
    match run 0
        Result::Ok _ => "ok".println!
        Result::Err e => e.println!
    match run 1
        Result::Ok _ => "ok".println!
        Result::Err e => e.println!
"#,
    ));
    assert_eq!(output, "99\n1\n101\n2\n102\nok\n");
}

#[test]
fn test_array_for_shared_tail_drops_enum_payload_with_parent_and_child_drop() {
    let output = compile_and_run(&with_noisy(
        r#"
struct Parent
    < value: I64
    < child: Noisy

impl Drop for Parent
    ~@drop = !->
        (self.value + 200).println!

enum Item
    Some Parent
    None

wrap: I64 -> Item
wrap = v -> Item::Some (Parent
    value: v
    child: mk v)

main = !->
    items = [(wrap 1), Item::None, (wrap 2), (wrap 3)]
    for item in items
        break
    "done".println!
"#,
    ));
    assert_eq!(output, "201\n101\n202\n102\n203\n103\ndone\n");
}

// Zero-length owned array: the loop body never runs and nothing drops.
#[test]
fn test_array_for_zero_length_array_runs_no_pass_and_drops_nothing() {
    let output = compile_and_run(&with_noisy(
        r#"
scan: () -> I64
scan = ->
    arr: [Noisy; 0] = []
    for x in arr
        x.value.println!
    "after".println!
    7

main = !->
    scan!.println!
    "done".println!
"#,
    ));
    assert_eq!(output, "after\n7\ndone\n");
}

// A loop that runs to completion followed by a return: the binding and the
// elements each drop exactly once.
#[test]
fn test_array_for_completed_loop_then_return_drops_once() {
    let output = compile_and_run(&with_noisy(
        r#"
scan: () -> I64
scan = ->
    arr = [(mk 1), (mk 2)]
    for x in arr
        x.value.println!
    return 5

main = !->
    scan!.println!
"#,
    ));
    assert_eq!(output, "1\n101\n2\n102\n5\n");
}

// A const-generic length reaching the for loop is concrete after
// monomorphization, including zero.
#[test]
fn test_array_for_generic_length_function_drops_each_once() {
    let output = compile_and_run(&with_noisy(
        r#"
walk: [Noisy; N] -> I64
walk = arr ->
    for x in arr
        x.value.println!
    9

main = !->
    (walk [(mk 1), (mk 2)]).println!
    e: [Noisy; 0] = []
    (walk e).println!
"#,
    ));
    assert_eq!(output, "1\n101\n2\n102\n9\n9\n");
}

use crate::support::*;

#[test]
fn bare_return_in_integer_function_is_rejected_at_the_return() {
    compile_should_fail(
        r#"
f = flag ->
    if flag
        return
    5

main = !->
    (f false).println!
"#,
        "Return type mismatch: this body returns an integer, but this `return` gives `()`",
    );
}

#[test]
fn early_return_value_must_match_the_function_result() {
    compile_should_fail(
        r#"
f = flag ->
    if flag
        return "text"
    5

main = !->
    (f false).println!
"#,
        "Return type mismatch: this body returns an integer, but this `return` gives `&Str`",
    );
}

#[test]
fn bare_return_must_match_a_non_literal_function_result() {
    compile_should_fail(
        r#"
f = flag ->
    if flag
        return
    "x".len!

main = !->
    (f false).println!
"#,
        "Return type mismatch: Type mismatch: () vs I64",
    );
}

#[test]
fn closure_return_must_match_the_closure_result() {
    compile_should_fail(
        r#"
main = !->
    pick = flag ->
        if flag
            return
        "x".len!
    (pick false).println!
"#,
        "Return type mismatch: Type mismatch: () vs I64",
    );
}

#[test]
fn returning_branch_does_not_shape_the_bound_value() {
    let output = compile_and_run(
        r#"
step = flag ->
    value = if flag
        return 0
    else
        3
    value + 1

main = !->
    (step false).println!
    (step true).println!
"#,
    );
    assert_eq!(output, "4\n0\n");
}

#[test]
fn bare_return_in_unit_bodies_still_compiles() {
    let output = compile_and_run(
        r#"
report = flag !->
    if flag
        return
    "reported".println!

main = !->
    report true
    report false
    each = flag !->
        if flag
            return
        "closure".println!
    each true
    each false
"#,
    );
    assert_eq!(output, "reported\nclosure\n");
}

#[test]
fn return_inside_for_loop_checks_the_enclosing_function() {
    compile_should_fail(
        r#"
first_big = items ->
    for item in items
        if item > 2
            return "big"
    0

main = !->
    (first_big [1, 2, 3]).println!
"#,
        "Return type mismatch",
    );
}

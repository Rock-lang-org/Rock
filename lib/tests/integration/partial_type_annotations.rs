use crate::support::*;

#[test]
fn test_vec_hole_infers_borrowed_filter_pipeline() {
    let output = compile_and_run(
        r#"
main = !->
    numbers: Vec _ = [1, 2, 3, 4].into!

    numbers.filter (value -> value % 2 == 0)
        <&> (* 10)
        |> (.println!)
"#,
    );
    assert_eq!(output.trim(), "[20, 40]");
}

#[test]
fn test_vec_hole_infers_sequence_pipeline() {
    let output = compile_and_run(
        r#"
main = !->
    values: Vec _ = [
        Option::Some 4
        Option::Some 5
    ].into!

    values
        |> sequence
        |> (.println!)
"#,
    );
    assert_eq!(output.trim(), "Some([4, 5])");
}

#[test]
fn test_vec_hole_annotation_infers_element_from_into() {
    let output = compile_and_run(
        r#"
main = !->
    values: Vec _ = [1, 2, 3].into!
    values.println!
"#,
    );
    assert_eq!(output.trim(), "[1, 2, 3]");
}

#[test]
fn test_bare_and_nested_hole_annotations_take_the_value_type() {
    let output = compile_and_run(
        r#"
main = !->
    x: _ = 5
    pair: (_, Bool) = (x, true)
    numbers: Vec I64 = [1, 2].into!
    copy: Vec _ = numbers
    pair.0.println!
    copy.println!
"#,
    );
    assert_eq!(output.trim(), "5\n[1, 2]");
}

#[test]
fn test_vec_hole_element_feeds_later_monadic_method_calls() {
    let output = compile_and_run(
        r#"
> std::concurrency::thread::spawn

main = !->
    handles: Vec _ = [(spawn (-> 1)), (spawn (-> 2))].into!
    joined = handles <&> (>>- (.join!))
    joined.println!
"#,
    );
    assert_eq!(output.trim(), "[Ok(1), Ok(2)]");
}

#[test]
fn test_expression_hole_annotation_is_inferred() {
    let output = compile_and_run(
        r#"
main = !->
    pair = (4, true): (_, Bool)
    pair.0.println!
"#,
    );
    assert_eq!(output.trim(), "4");
}

#[test]
fn test_hole_in_item_signature_is_rejected() {
    compile_should_fail(
        r#"
first: _ -> I64
first = value -> 0

main = !->
    ()
"#,
        "the type placeholder `_` is only allowed in local type annotations",
    );
}

#[test]
fn test_hole_annotation_still_reports_mismatches() {
    compile_should_fail(
        r#"
main = !->
    values: Vec _ = 5
    ()
"#,
        "Type annotation mismatch for 'values'",
    );
}

use crate::support::*;

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
fn destructuring_through_a_reference_parameter() {
    let output = compile_and_run(
        r#"
sum: &(I64, I64) -> I64
sum = pair ->
    (a, b) = pair
    *a + *b

main = !->
    pair = (3, 4)
    (sum &pair).println!
    (sum &pair).println!
"#,
    );
    assert_eq!(output.trim(), "7\n7");
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

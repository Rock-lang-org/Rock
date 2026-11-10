use crate::support::*;
use std::path::PathBuf;

fn borrow_diagnostic_messages(source: &str) -> Vec<String> {
    let dir = test_temp_dir(TEST_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
    std::fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    let source_path = dir.join("test.rk");
    std::fs::write(&source_path, source).unwrap();
    let mut config = test_config(source_path, dir.clone());
    config.no_link = true;
    rock_lib::compile(&config)
        .expect_err("expected compilation to fail")
        .0
        .iter()
        .map(|diagnostic| diagnostic.message.clone())
        .collect()
}

#[test]
fn test_move_out_of_borrowed_match_payload_names_type_once() {
    let messages = borrow_diagnostic_messages(
        r#"
enum Label
    Text String

impl Show for Label
    @show = ->
        match *self
            Label::Text text => text

main = !->
    (Label::Text (String::from "hi")).println!
"#,
    );

    assert_eq!(
        messages,
        ["Cannot move non-copy value of type 'String' out of a reference"]
    );
}

#[test]
fn test_move_whole_value_out_of_self_names_its_type() {
    let messages = borrow_diagnostic_messages(
        r#"
enum Label
    Text String

impl Show for Label
    @show = ->
        whole = *self
        match whole
            Label::Text text => text

main = !->
    (Label::Text (String::from "hi")).println!
"#,
    );

    assert_eq!(
        messages,
        ["Cannot move non-copy value of type 'Label' out of a reference"]
    );
}

#[test]
fn test_move_non_copy_value_out_of_shared_reference_is_rejected() {
    compile_should_fail(
        r#"
main = ->
    s = String::from "hello"
    r = &s
    moved = *r
    0
"#,
        "Cannot move non-copy value",
    );
}

#[test]
fn test_borrowed_enum_match_payload_binding_is_shared_reference() {
    let output = compile_and_run(
        r#"
show_option: &Option String -> I64
show_option = opt ->
    match *opt
        Option::Some value => value.len!
        Option::None => 0

main = ->
    value = Option::Some (String::from "hello")
    show_option &value .println!
    show_option &value .println!
    0
"#,
    );

    assert_eq!(output.trim(), "5\n5");
}

#[test]
fn test_borrowed_enum_match_payload_field_projection_uses_referent() {
    let output = compile_and_run(
        r#"
struct Boxed
    < text: String

show_boxed: &Option Boxed -> I64
show_boxed = opt ->
    match *opt
        Option::Some value => value.text.len!
        Option::None => 0

main = ->
    value = Option::Some (Boxed
        text: String::from "hello")
    show_boxed &value .println!
    show_boxed &value .println!
    0
"#,
    );

    assert_eq!(output.trim(), "5\n5");
}

#[test]
fn test_borrowed_enum_match_reference_payload_binding_uses_payload_type() {
    let output = compile_and_run(
        r#"
read_option_ref: &Option &I64 -> I64
read_option_ref = opt ->
    match *opt
        Option::Some value => *value
        Option::None => 0

main = ->
    number = 42
    value = Option::Some (&number)
    read_option_ref &value .println!
    read_option_ref &value .println!
    0
"#,
    );

    assert_eq!(output.trim(), "42\n42");
}

#[test]
fn test_copy_non_cleanup_struct_out_of_shared_reference_is_rejected() {
    compile_should_fail(
        r#"
struct Pair
    < x: I64

main = ->
    p = Pair
        x: 1
    r = &p
    copied = *r
    0
"#,
        "Cannot move non-copy value",
    );
}

// ── Borrow checker / MIR tests ──────────────────────────────────────────────

fn compile_example_should_fail(name: &str, expected_error: &str) {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let examples_dir = PathBuf::from(manifest_dir)
        .parent()
        .unwrap()
        .join("examples");
    let source_path = examples_dir.join(format!("{}.rk", name));
    let source = std::fs::read_to_string(&source_path)
        .unwrap_or_else(|_| panic!("Failed to read example: {}", name));
    compile_should_fail(&source, expected_error);
}

fn compile_example_should_pass(name: &str) {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let examples_dir = PathBuf::from(manifest_dir)
        .parent()
        .unwrap()
        .join("examples");
    let source_path = examples_dir.join(format!("{}.rk", name));
    let source = std::fs::read_to_string(&source_path)
        .unwrap_or_else(|_| panic!("Failed to read example: {}", name));
    compile_and_run(&source);
}

#[test]
fn test_borrow_use_after_move() {
    compile_example_should_fail("mir_tests/use_after_move", "borrow of moved value");
}

#[test]
fn test_borrow_double_move() {
    compile_example_should_fail("mir_tests/double_move", "use of moved value");
}

#[test]
fn test_borrow_uaf() {
    compile_example_should_fail("mir_tests/uaf", "borrow of moved value");
}

#[test]
fn test_borrow_mut_ref_requires_mutable_binding() {
    compile_should_fail(
        r#"
main = ->
    x = 1
    r = &mut x
    0
"#,
        "Cannot take a mutable reference to immutable binding",
    );
}

#[test]
fn test_borrow_mut_ref_not_copy() {
    compile_should_fail(
        r#"
main = ->
    mut x = 1
    r = &mut x
    y = r
    z = r
    0
"#,
        "use of moved value",
    );
}

#[test]
fn test_borrow_mut_ref_moves_across_function_call() {
    compile_should_fail(
        r#"
helper = r -> 0

main = ->
    mut x = 1
    r = &mut x
    helper r
    y = r
    0
"#,
        "use of moved value",
    );
}

#[test]
fn test_borrow_mut_ref_blocks_later_use() {
    compile_should_fail(
        r#"
main = ->
    mut x = 1
    r = &mut x
    x.println!
    r.println!
    0
"#,
        "borrow conflict",
    );
}

#[test]
fn test_borrow_mut_ref_blocks_assignment() {
    compile_should_fail(
        r#"
main = ->
    mut x = 1
    r = &mut x
    x = 2
    r.println!
    0
"#,
        "borrow conflict",
    );
}

#[test]
fn test_borrow_reborrow_allows_write_through_reborrow() {
    compile_should_pass(
        r#"
main = ->
    mut x = 1
    r = &mut x
    s = &mut *r
    *s = 2
    0
"#,
    );
}

#[test]
fn test_borrow_reborrow_blocks_owner_assignment_before_reborrow_use() {
    compile_should_fail(
        r#"
main = ->
    mut x = 1
    r = &mut x
    s = &mut *r
    x = 2
    *s = 3
    0
"#,
        "borrow conflict",
    );
}

#[test]
fn test_borrow_mut_ref_blocks_drop_at_scope_end() {
    compile_should_pass(
        r#"
main = ->
    mut x = String::from "hello"
    r = &mut x
    0
"#,
    );
}

#[test]
fn test_borrow_shared_ref_blocks_assignment() {
    compile_should_pass(
        r#"
main = ->
    mut x = 1
    r = &x
    x = 2
    0
"#,
    );
}

#[test]
fn test_borrow_shared_ref_blocks_move() {
    compile_should_pass(
        r#"
main = ->
    mut x: Vec I64 = Vec::new!
    r = &x
    y = x
    0
"#,
    );
}

#[test]
fn test_borrow_reference_copy_releases_with_owner() {
    compile_should_pass(
        r#"
main = ->
    mut x = 1
    r = &x
    s = r
    x = 2
    0
"#,
    );
}

#[test]
fn test_borrow_reference_copy_keeps_original_alias_live() {
    compile_should_fail(
        r#"
main = ->
    mut x = 1
    r = &x
    s = r
    x = 2
    t = r
    0
"#,
        "borrow",
    );
}

#[test]
fn test_borrow_shared_ref_blocks_assignment_rust_parity() {
    compile_example_should_fail("mir_tests/shared_borrow_then_assign", "borrow");
}

#[test]
fn test_borrow_shared_ref_blocks_move_rust_parity() {
    compile_example_should_fail("mir_tests/shared_borrow_then_move", "borrow");
}

#[test]
fn test_borrow_closure_shared_capture_blocks_mutation() {
    compile_example_should_fail("mir_tests/closure_shared_capture_blocks_mutation", "borrow");
}

#[test]
fn test_borrow_closure_copy_keeps_capture_loan_live() {
    compile_should_fail(
        r#"
main = ->
    mut x = 1
    f = -> x.println!
    g = f
    x = 2
    g!
    0
"#,
        "borrow",
    );
}

#[test]
fn test_borrow_closure_move_capture_moves_value() {
    compile_example_should_fail("mir_tests/closure_move_capture_moves_value", "moved");
}

#[test]
fn test_borrow_raw_pointer_from_borrow_preserves_reference_rules() {
    compile_example_should_fail("mir_tests/raw_pointer_from_borrow", "borrow");
}

#[test]
fn test_borrow_raw_pointer_copy_keeps_reference_rules() {
    compile_should_fail(
        r#"
main = ->
    mut x = 42
    r = &mut x
    ptr = r as *I64
    ptr2 = ptr
    y = x
    unsafe *ptr2 = 7
    0
"#,
        "borrow",
    );
}

#[test]
fn test_borrow_deref_mut_borrow_conflicts() {
    compile_should_fail(
        r#"
main = ->
    mut x = 1
    shared = &x
    mutable = &mut *shared
    0
"#,
        "borrow conflict",
    );
}

#[test]
fn test_borrow_mut_ref_branch_merge_blocks_later_use() {
    compile_should_fail(
        r#"
main = ->
    mut x = 1
    r = &mut x
    if x == 1
        r = &mut x
    else
        r = &mut x
    x = 2
    r.println!
    0
"#,
        "borrow conflict",
    );
}

#[test]
fn test_borrow_move_valid() {
    compile_example_should_pass("mir_tests/move_valid");
}

#[test]
fn test_borrow_loop() {
    compile_example_should_pass("mir_tests/loop_test");
}

#[test]
fn test_borrow_shared_borrow() {
    compile_example_should_pass("mir_tests/shared_borrow");
}

#[test]
fn test_borrow_conditional_move() {
    compile_example_should_pass("mir_tests/conditional_move");
}

#[test]
fn test_borrow_mut_and_shared() {
    compile_example_should_pass("mir_tests/mut_and_shared");
}

#[test]
fn test_reference_lifetime_cleanup() {
    compile_example_should_pass("mir_tests/reference_lifetime_cleanup");
}

#[test]
fn test_borrow_disjoint_fields() {
    compile_example_should_pass("mir_tests/field_borrowing");
}

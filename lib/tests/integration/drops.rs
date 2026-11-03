use crate::support::*;
use std::fs;

#[test]
fn test_drop_in_place_drops_raw_pointer_value() {
    let output = compile_and_run(
        r#"
extern malloc: I64 -> *U8
extern free: *U8 -> ()

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    ptr: *Tracked = (malloc 8) as *Tracked
    unsafe *ptr = Tracked
        value: 9
    unsafe drop_in_place ptr
    free (ptr as *U8)
    0
"#,
    );

    assert_eq!(output.trim(), "9");
}

#[test]
fn test_std_box_drops_inner_value() {
    let output = compile_and_run(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    boxed = Box::new (Tracked
        value: 7)
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_std_box_rejects_zero_sized_value() {
    let (_output, success) = compile_and_run_with_status(
        r#"
> std::mem::Box

struct Empty

main = ->
    boxed = Box::new Empty
    0
"#,
    );

    assert!(!success, "Box::new should reject zero-sized values");
}

#[test]
fn test_std_box_new_uses_non_null_pointer() {
    let output = compile_and_run(
        r#"
> std::mem::Box

main = ->
    boxed = Box::new 42
    (boxed.as_ptr! as I64) != 0 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "true");
}

#[test]
fn test_inherent_drop_method_is_not_automatic_cleanup() {
    let output = compile_and_run(
        r#"
struct NotDrop
    < value: I64

impl NotDrop
    @drop = ->
        self.value.println!
        return

main = ->
    value = NotDrop
        value: 99
    value.drop!
    0
"#,
    );

    assert_eq!(output.trim(), "99");
}

#[test]
fn test_std_box_drops_aggregate_field_value() {
    let output = compile_and_run(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < value: I64

struct Wrapper
    < tracked: Tracked

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    boxed = Box::new (Wrapper
        tracked: Tracked
            value: 8)
    0
"#,
    );

    assert_eq!(output.trim(), "8");
}

#[test]
fn test_drop_order_runs_owner_before_field_cleanup() {
    let output = compile_and_run(
        r#"
> std::mem::drop::Drop

struct Child
    < value: I64

struct Owner
    < child: Child

impl Drop for Child
    ~@drop = ->
        self.value.println!
        return

impl Drop for Owner
    ~@drop = ->
        self.child.value + 100 .println!
        return

main = ->
    owner = Owner
        child: Child
            value: 4
    0
"#,
    );

    assert_eq!(output.trim().lines().collect::<Vec<_>>(), vec!["104", "4"]);
}

#[test]
fn test_branch_move_drops_value_once_after_merge() {
    let output = compile_and_run(
        r#"
> std::mem::drop::Drop

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

branch = cond, value ->
    a = Tracked
        value: value
    if cond
        b = a
    0

main = ->
    branch true, 1
    branch false, 2
    0
"#,
    );

    assert_eq!(output.trim().lines().collect::<Vec<_>>(), vec!["1", "2"]);
}

#[test]
fn test_field_assignment_drops_old_field_before_overwrite() {
    let output = compile_and_run(
        r#"
> std::mem::drop::Drop

struct Tracked
    < value: I64

struct Wrapper
    < tracked: Tracked

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    mut wrapper = Wrapper
        tracked: Tracked
            value: 1
    wrapper.tracked = Tracked
        value: 2
    0
"#,
    );

    assert_eq!(output.trim().lines().collect::<Vec<_>>(), vec!["1", "2"]);
}

#[test]
fn test_tuple_array_and_enum_payload_cleanup_drop_elements() {
    let output = compile_and_run(
        r#"
> std::mem::drop::Drop

struct Tracked
    < value: I64

enum MaybeTracked
    Some Tracked
    None

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    tuple = (Tracked
        value: 3, Tracked
        value: 4)
    array = [Tracked
        value: 5, Tracked
        value: 6]
    some = MaybeTracked::Some (Tracked
        value: 7)
    none = MaybeTracked::None
    0
"#,
    );

    let mut lines: Vec<&str> = output.trim().lines().collect();
    lines.sort();
    assert_eq!(lines, vec!["3", "4", "5", "6", "7"]);
}

#[test]
fn test_expression_statement_temporary_is_dropped() {
    let output = compile_and_run(
        r#"
> std::mem::drop::Drop

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

make = value -> Tracked
    value: value

main = ->
    make 8
    0
"#,
    );

    assert_eq!(output.trim(), "8");
}

#[test]
fn test_borrowed_temporary_drops_when_call_expression_finishes() {
    let output = compile_and_run(
        r#"
> std::mem::drop::Drop

struct Holder
    < value: I64

impl Drop for Holder
    ~@drop = ->
        2.println!
        return

make_holder = ->
    Holder
        value: 1

use_holder: &Holder -> ()
use_holder = holder ->
    holder.value.println!
    return

main = ->
    use_holder (&(make_holder!))
    3.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["1", "2", "3"]);
}

#[test]
fn test_return_expression_temporary_drops_before_returning_to_caller() {
    let output = compile_and_run(
        r#"
> std::mem::drop::Drop

struct Holder
    < value: I64

impl Drop for Holder
    ~@drop = ->
        2.println!
        return

make_holder = ->
    Holder
        value: 1

read_holder: &Holder -> I64
read_holder = holder -> holder.value

make_value = ->
    return read_holder (&(make_holder!))

main = ->
    make_value! .println!
    3.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["2", "1", "3"]);
}

#[test]
fn test_owned_string_temporary_lives_through_ffi_call() {
    let output = compile_and_run(
        r#"
> std::ffi::libc::puts
> std::string::String

main = ->
    puts ((String::from "hello").as_ptr!)
    0
"#,
    );

    assert_eq!(output.trim(), "hello");
}

#[test]
fn test_std_has_no_keep_alive_lifetime_workarounds() {
    let std_files = ["src/convert.rk", "src/control/option.rk", "src/control/result.rk", "src/show.rk"];
    let offenders = std_files
        .iter()
        .filter_map(|file| {
            let path = std_path().join(file);
            let source = fs::read_to_string(&path).unwrap();
            source.contains("keep_alive").then_some(file.to_string())
        })
        .collect::<Vec<_>>();

    assert!(
        offenders.is_empty(),
        "std should rely on full-expression temporary lifetimes, found keep_alive in {offenders:?}"
    );
}

#[test]
fn test_unrelated_trait_named_drop_is_not_automatic_cleanup() {
    let output = compile_and_run(
        r#"
trait Drop
    ~@drop: ()

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    value = Tracked
        value: 9
    0
"#,
    );

    assert_eq!(output.trim(), "");
}

#[test]
fn test_unrelated_trait_named_drop_allows_field_moves() {
    let output = compile_and_run(
        r#"
trait Drop
    ~@drop: ()

struct Child
    < value: I64

struct Tracked
    < child: Child

impl Drop for Tracked
    ~@drop = ->
        self.child.value.println!
        return

take_child = tracked -> tracked.child

main = ->
    tracked = Tracked
        child: Child
            value: 9
    child = take_child tracked
    child.value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "9");
}

#[test]
fn test_std_box_move_drops_inner_value_once() {
    let (output, success) = compile_and_run_with_status(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    a = Box::new (Tracked
        value: 3)
    b = a
    0
"#,
    );

    assert!(success, "program failed with output {output:?}");
    assert_eq!(output.trim(), "3");
}

#[test]
fn test_returned_std_box_binding_drops_inner_value_once() {
    let (output, success) = compile_and_run_with_status(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

make_box: () -> Box Tracked
make_box = -> Box::new (Tracked
    value: 5)

main = ->
    box = make_box!
    0
"#,
    );

    assert!(success, "program should not double-free or abort");
    assert_eq!(output.trim(), "5");
}

#[test]
fn test_method_by_value_owned_arg_drops_once() {
    let (output, success) = compile_and_run_with_status(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < id: I64

impl Drop for Tracked
    ~@drop = ->
        self.id.println!
        return

struct Sink

impl Sink
    @take: Box Tracked -> ()
    @take = value ->
        return

main = ->
    sink = Sink
    sink.take (Box::new (Tracked
        id: 11))
    0
"#,
    );

    assert!(success, "program should not double-free or abort");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["11"]);
}

#[test]
fn test_partial_move_from_direct_drop_type_is_rejected() {
    compile_should_fail(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < id: I64

impl Drop for Tracked
    ~@drop = ->
        self.id.println!
        return

struct Owner
    < value: Box Tracked

impl Drop for Owner
    ~@drop = ->
        99.println!
        return

take_value = owner -> owner.value

main = ->
    owner = Owner
        value: Box::new (Tracked
            id: 1)
    boxed = take_value owner
    0
"#,
        "because it implements Drop",
    );
}

#[test]
fn test_nested_partial_move_from_direct_drop_type_is_rejected() {
    compile_should_fail(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < id: I64

impl Drop for Tracked
    ~@drop = ->
        self.id.println!
        return

struct Inner
    < value: Box Tracked

struct Owner
    < inner: Inner

impl Drop for Owner
    ~@drop = ->
        99.println!
        return

take_value = owner -> owner.inner.value

main = ->
    owner = Owner
        inner: Inner
            value: Box::new (Tracked
                id: 1)
    boxed = take_value owner
    0
"#,
        "because it implements Drop",
    );
}

#[test]
fn test_tuple_partial_move_from_direct_drop_type_is_rejected() {
    compile_should_fail(
        r#"
> std::mem::drop::Drop

struct Child
    < id: I64

struct Owner
    < pair: (Child, I64)

impl Drop for Owner
    ~@drop = ->
        99.println!
        return

make_child = ->
    Child
        id: 1

take_value = owner -> owner.pair.0

main = ->
    owner = Owner
        pair: (make_child!, 2)
    child = take_value owner
    0
"#,
        "because it implements Drop",
    );
}

#[test]
fn test_array_element_move_of_cleanup_value_is_rejected() {
    compile_should_fail(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < id: I64

impl Drop for Tracked
    ~@drop = ->
        self.id.println!
        return

take_first: [Box Tracked; 1] -> Tracked
take_first = values -> *(values[0])

main = ->
    values: [Box Tracked; 1] = [Box::new (Tracked
        id: 1)]
    boxed = take_first values
    0
"#,
        "borrow conflict",
    );
}

#[test]
fn test_std_box_reassignment_drops_old_and_new_values_once() {
    let output = compile_and_run(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    box = Box::new (Tracked
        value: 1)
    box = Box::new (Tracked
        value: 2)
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["1", "2"]);
}

#[test]
fn test_by_value_owned_parameter_is_dropped_when_unused() {
    let output = compile_and_run(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

consume_unused: Box Tracked -> ()
consume_unused = x ->
    return

main = ->
    consume_unused (Box::new (Tracked
        value: 10))
    0
"#,
    );

    assert_eq!(output.trim(), "10");
}

#[test]
fn test_by_value_owned_parameter_with_local_drop_is_dropped_when_unused() {
    let output = compile_and_run(
        r#"
> std::mem::drop::Drop

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

consume_unused: Tracked -> ()
consume_unused = x ->
    return

main = ->
    consume_unused (Tracked
        value: 11)
    0
"#,
    );

    assert_eq!(output.trim(), "11");
}

#[test]
fn test_assigning_moved_out_field_does_not_drop_stale_value() {
    let output = compile_and_run(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < value: I64

struct Wrapper
    < field: Box Tracked

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    mut wrapper = Wrapper
        field: Box::new (Tracked
            value: 1)
    moved = wrapper.field
    wrapper.field = Box::new (Tracked
        value: 2)
    0
"#,
    );

    let mut lines: Vec<&str> = output.trim().lines().collect();
    lines.sort();
    assert_eq!(lines, vec!["1", "2"]);
}

#[test]
fn test_enum_partial_payload_move_drops_unmoved_payload_field() {
    let output = compile_and_run(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < value: I64

enum Pair
    Both (Box Tracked) (Box Tracked)

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    pair = Pair::Both (Box::new (Tracked
        value: 1)), (Box::new (Tracked
        value: 2))
    match pair
        Pair::Both first, _ => 0
    0
"#,
    );

    let mut lines: Vec<&str> = output.trim().lines().collect();
    lines.sort();
    assert_eq!(lines, vec!["1", "2"]);
}

#[test]
fn test_parent_cleanup_skips_moved_field_but_drops_unmoved_sibling() {
    let output = compile_and_run(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < value: I64

struct Wrapper
    < first: Box Tracked
    < second: Box Tracked

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    wrapper = Wrapper
        first: Box::new (Tracked
            value: 1)
        second: Box::new (Tracked
            value: 2)
    moved = wrapper.first
    0
"#,
    );

    let mut lines: Vec<&str> = output.trim().lines().collect();
    lines.sort();
    assert_eq!(lines, vec!["1", "2"]);
}

#[test]
fn test_branch_skipped_field_move_drops_both_fields_after_merge() {
    let output = compile_and_run(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < value: I64

struct Wrapper
    < first: Box Tracked
    < second: Box Tracked

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    wrapper = Wrapper
        first: Box::new (Tracked
            value: 1)
        second: Box::new (Tracked
            value: 2)
    if false
        moved = wrapper.first
    0
"#,
    );

    let mut lines: Vec<&str> = output.trim().lines().collect();
    lines.sort();
    assert_eq!(lines, vec!["1", "2"]);
}

#[test]
fn test_branch_taken_field_move_drops_moved_field_once_and_sibling_after_merge() {
    let output = compile_and_run(
        r#"
> std::mem::Box
> std::mem::drop::Drop

struct Tracked
    < value: I64

struct Wrapper
    < first: Box Tracked
    < second: Box Tracked

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    wrapper = Wrapper
        first: Box::new (Tracked
            value: 1)
        second: Box::new (Tracked
            value: 2)
    if true
        moved = wrapper.first
    0
"#,
    );

    let mut lines: Vec<&str> = output.trim().lines().collect();
    lines.sort();
    assert_eq!(lines, vec!["1", "2"]);
}

use crate::support::*;

#[test]
fn nested_index_mut_updates_matrix_element() {
    let output = compile_and_run(
        r#"
> std::collections::Vec

main = ->
    mut first = Vec::new!
    first.push 1
    first.push 2
    mut second = Vec::new!
    second.push 3
    second.push 4
    mut matrix = Vec::new!
    matrix.push first
    matrix.push second
    matrix[1][0] = 9
    matrix[1][0].println!
    0
"#,
    );

    assert_eq!(output.trim(), "9");
}

#[test]
fn index_mut_then_field_updates_record() {
    let output = compile_and_run(
        r#"
struct Record
    < field: I64

main = ->
    mut records: [Record; 2] = [Record field: 1, Record field: 2]
    records[1].field = 9
    records[1].field.println!
    0
"#,
    );

    assert_eq!(output.trim(), "9");
}

#[test]
fn indexed_assignment_rhs_uses_index() {
    let output = compile_and_run(
        r#"
struct Cell
    < read_value: I64
    < write_value: I64

impl Index I64 for Cell
    type Output = I64
    @index = _ -> &@read_value

impl IndexMut I64 for Cell
    type Output = I64
    ^@index_mut = _ -> &mut @write_value

main = ->
    mut target = Cell
        read_value: 1
        write_value: 0
    source = Cell
        read_value: 7
        write_value: 2
    target[0] = source[0]
    target.write_value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn mutable_index_reference_uses_index_mut_authority() {
    let output = compile_and_run(
        r#"
struct Cell
    < read_value: I64
    < write_value: I64

impl Index I64 for Cell
    type Output = I64
    @index = _ -> &@read_value

impl IndexMut I64 for Cell
    type Output = I64
    ^@index_mut = _ -> &mut @write_value

main = ->
    mut target = Cell
        read_value: 1
        write_value: 2
    mutable = &mut target[0]
    *mutable = 9
    target.read_value.println!
    target.write_value.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["1", "9"]);
}

#[test]
fn shared_index_reference_uses_index_authority() {
    let output = compile_and_run(
        r#"
main = ->
    mut arr: [I64; 2] = [1, 2]
    shared = &arr[0]
    *shared .println!
    0
"#,
    );

    assert_eq!(output.trim(), "1");
}

#[test]
fn mutable_index_reference_preserves_distinct_index_aliases() {
    let output = compile_and_run(
        r#"
struct Cell
    < read_value: I64
    < write_value: I64

impl Index I64 for Cell
    type Output = I64
    @index = _ -> &@read_value

impl IndexMut I64 for Cell
    type Output = I64
    ^@index_mut = _ -> &mut @write_value

main = ->
    mut target = Cell
        read_value: 1
        write_value: 2
    mutable = &mut target[0]
    *mutable = 9
    shared = &target[0]
    *shared .println!
    target.write_value.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["1", "9"]);
}

#[test]
fn shared_index_borrow_blocks_index_mut_assignment() {
    compile_should_fail(
        r#"
main = ->
    mut arr: [I64; 2] = [1, 2]
    shared: &I64 = &arr[0]
    arr[0] = 9
    value = *shared
    value.println!
    0
"#,
        "borrow conflict on",
    );
}

#[test]
fn shared_range_borrow_blocks_overlapping_mutable_range() {
    compile_should_fail(
        r#"
main = ->
    mut values = [10, 20, 30, 40]
    shared: &[I64] = &values[..2]
    mut overlapping: &mut [I64] = &mut values[1..]
    overlapping[0] = 99
    shared[0].println!
    0
"#,
        "borrow conflict on",
    );
}

#[test]
fn mutable_index_reference_preserves_alias_rules() {
    compile_should_fail(
        r#"
main = ->
    mut arr: [I64; 2] = [1, 2]
    mutable = &mut arr[0]
    shared = &arr[0]
    *mutable .println!
    *shared .println!
    0
"#,
        "borrow conflict on",
    );
}

#[test]
fn aggregate_mut_reference_blocks_shared_access() {
    compile_should_fail(
        r#"
struct Holder
    < reference: &mut I64

main = ->
    mut x = 1
    holder = Holder
        reference: &mut x
    shared = &x
    *holder.reference .println!
    *shared .println!
    0
"#,
        "borrow conflict on",
    );
}

#[test]
fn aggregate_mut_reference_blocks_mutable_access() {
    compile_should_fail(
        r#"
struct Holder
    < reference: &mut I64

main = ->
    mut x = 1
    holder = Holder
        reference: &mut x
    other = &mut x
    *holder.reference .println!
    *other .println!
    0
"#,
        "borrow conflict on",
    );
}

#[test]
fn value_index_reads_release_before_later_mutation() {
    let output = compile_and_run(
        r#"
main = ->
    mut arr: [I64; 2] = [2, 1]
    if arr[0] > arr[1] then
        arr[0] = arr[1]
    arr[0] .println!
    0
"#,
    );

    assert_eq!(output.trim(), "1");
}

#[test]
fn test_array_read_without_index_provider_fails_before_mono() {
    compile_should_fail_with_exact_diagnostic(
        r#"
main = ->
    arr: [I64; 3] = [1, 2, 3]
    arr[0]
    0
"#,
        "Cannot use indexing because the Index language-item protocol is unavailable",
    );
}

#[test]
fn test_array_write_without_index_mut_provider_fails_before_mono() {
    compile_should_fail_with_exact_diagnostic(
        r#"
main = ->
    arr: [I64; 3] = [1, 2, 3]
    arr[0] = 4
    0
"#,
        "Cannot use mutable indexing because the IndexMut language-item protocol is unavailable",
    );
}

#[test]
fn test_slice_read_without_index_provider_fails_before_mono() {
    compile_should_fail_with_exact_diagnostic(
        r#"
read: &[I64] -> I64
read = slice -> slice[0]

main = -> 0
"#,
        "Cannot use indexing because the Index language-item protocol is unavailable",
    );
}

#[test]
fn test_slice_write_without_index_mut_provider_fails_before_mono() {
    compile_should_fail_with_exact_diagnostic(
        r#"
write: &mut [I64] -> ()
write = slice -> slice[0] = 4

main = -> 0
"#,
        "Cannot use mutable indexing because the IndexMut language-item protocol is unavailable",
    );
}

#[test]
fn test_pointer_read_without_index_provider_fails_before_mono() {
    compile_should_fail_with_exact_diagnostic(
        r#"
read: *I64 -> I64
read = ptr -> unsafe ptr[0]

main = -> 0
"#,
        "Cannot use indexing because the Index language-item protocol is unavailable",
    );
}

#[test]
fn test_pointer_write_without_index_mut_provider_fails_before_mono() {
    compile_should_fail_with_exact_diagnostic(
        r#"
write: *I64 -> ()
write = ptr -> unsafe ptr[0] = 4

main = -> 0
"#,
        "Cannot use mutable indexing because the IndexMut language-item protocol is unavailable",
    );
}

#[test]
fn test_foreign_index_impl_for_structural_array_is_rejected() {
    compile_should_fail(
        r#"
impl Index I64 for [Bool; 3]
    type Output = Bool
    @index = i ->
        ptr: *Bool = self as *Bool
        unsafe &(ptr[2 - i])

main = ->
    arr: [Bool; 3] = [false, false, true]
    arr[0].println!
    p = &arr[0]
    *p .println!
    0
"#,
        "orphan impl",
    );
}

#[test]
fn test_generic_use_of_foreign_index_impl_for_structural_array_is_rejected() {
    compile_should_fail(
        r#"
impl Index I64 for [Bool; 3]
    type Output = Bool
    @index = i ->
        ptr: *Bool = self as *Bool
        unsafe &(ptr[2 - i])

is_selected_at_two = value ->
    if value[2]
        1
    else
        0

main = ->
    arr: [Bool; 3] = [true, false, false]
    is_selected_at_two arr .println!
    0
"#,
        "orphan impl",
    );
}

#[test]
fn test_repeated_receiver_generic_impl_does_not_match_incompatible_receiver_args() {
    compile_should_fail(
        r#"
trait SameCode
    @code = -> 0

struct Pair A, B
    < left: A
    < right: B

impl SameCode for Pair T, T
    @code = -> 99

main = ->
    pair = Pair I64, Bool
        left: 1
        right: true
    pair.code!.println!
    0
"#,
        "Unknown field 'code' on struct 'Pair'",
    );
}

#[test]
fn test_explicit_generic_slice_impl_uses_self_body_and_dispatches() {
    let output = compile_and_run(
        r#"
trait SliceLen
    @slice_len = -> 0

impl SliceLen for &[T]
    @slice_len = ->
        ~ArrayLen (*self)

main = ->
    arr = [97 as U8, 98 as U8, 99 as U8]
    (&arr).slice_len!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "3");
}

#[test]
fn test_explicit_generic_slice_impl_method_as_value_falls_back_from_u8_slice() {
    let output = compile_and_run(
        r#"
trait SliceLen
    @slice_len = -> 0

impl SliceLen for &[T]
    @slice_len = ->
        ~ArrayLen (*self)

main = ->
    arr = [97 as U8, 98 as U8, 99 as U8]
    f = (&arr).slice_len
    f!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "3");
}

#[test]
fn test_array_indexing() {
    let output = compile_and_run(
        r#"
main = ->
    arr = [10, 20, 30, 40, 50]
    arr[0] .println!
    arr[2] .println!
    arr[4] .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "30");
    assert_eq!(lines[2], "50");
}

#[test]
fn test_array_for_loop() {
    let output = compile_and_run(
        r#"
main = ->
    arr = [1, 2, 3, 4, 5]
    sum = 0
    for x in arr
        sum = sum + x
    sum.println!
    0
"#,
    );
    assert_eq!(output.trim(), "15");
}

#[test]
fn test_array_index_assignment() {
    let output = compile_and_run(
        r#"
main = ->
    mut arr = [10, 20, 30]
    arr[1] .println!
    arr[1] = 99
    arr[1] .println!
    arr[0] .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "20"); // original arr[1]
    assert_eq!(lines[1], "99"); // modified arr[1]
    assert_eq!(lines[2], "10"); // arr[0] unchanged
}

#[test]
fn test_slice_indexing() {
    let output = compile_and_run(
        r#"
read_second: &[I64] -> I64
read_second = slice -> slice[1]

main = ->
    arr = [7, 8, 9]
    read_second &arr .println!
    0
"#,
    );

    assert_eq!(output.trim(), "8");
}

#[test]
fn test_native_ranges_borrow_slice_views() {
    let output = compile_and_run(
        r#"
inspect: &[I64] -> ()
inspect = slice !->
    (~ArrayLen slice).println!
    slice[0].println!

inspect_ranges: &[I64] -> ()
inspect_ranges = slice !->
    inspect (&slice[..2])
    inspect (&slice[1..])
    inspect (&slice[1..3])
    inspect (&slice[..=1])
    inspect (&slice[1..=2])
    inspect (&slice[..])
    selected = 1..3
    inspect (&slice[selected])

main = ->
    values = [10, 20, 30, 40]
    inspect_ranges (&values)
    inspect (&values[..2])
    0
"#,
    );

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        ["2", "10", "3", "20", "2", "20", "2", "10", "2", "20", "4", "10", "2", "20", "2", "10"]
    );
}

#[test]
fn test_inclusive_native_range_for_loop_includes_end() {
    let output = compile_and_run(
        r#"
main = ->
    mut total = 0
    for value in 1..=3
        total = total + value
    total.println!
    0
"#,
    );

    assert_eq!(output.trim(), "6");
}

#[test]
fn test_slice_index_assignment() {
    let output = compile_and_run(
        r#"
write_second: &mut [I64] -> I64
write_second = input ->
    mut slice = input
    slice[1] = 42
    0

main = ->
    mut values = [7, 8, 9]
    write_second (&mut values)
    values[1].println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_native_mutable_range_updates_original_slice() {
    let output = compile_and_run(
        r#"
update_middle: &mut [I64] -> ()
update_middle = slice !->
    mut middle = &mut slice[1..3]
    middle[0] = 77

main = ->
    mut values = [10, 20, 30, 40]
    update_middle (&mut values)
    values[1].println!
    values[2].println!
    0
"#,
    );

    assert_eq!(output.lines().collect::<Vec<_>>(), ["77", "30"]);
}

#[test]
fn test_native_ranges_index_vec_views() {
    let output = compile_and_run(
        r#"
inspect: &[I64] -> ()
inspect = slice !->
    (~ArrayLen slice).println!
    slice[0].println!

update_prefix: &mut (Vec I64) -> ()
update_prefix = values !->
    mut prefix: &mut [I64] = &mut (*values)[..2]
    prefix[0] = 55

main = ->
    mut values: Vec I64 = Vec::new!
    values.push 10
    values.push 20
    values.push 30
    inspect (&values[1..])
    update_prefix (&mut values)
    values[0].println!
    0
"#,
    );

    assert_eq!(output.lines().collect::<Vec<_>>(), ["2", "20", "55"]);
}

#[test]
fn test_native_range_out_of_bounds_traps() {
    let (stdout, success) = compile_and_run_with_status(
        r#"
main = ->
    values = [10, 20, 30]
    slice = &values[1..4]
    (~ArrayLen *slice).println!
    0
"#,
    );

    assert!(!success);
    assert!(
        stdout.contains("range out of bounds"),
        "stdout was {stdout:?}"
    );
}

#[test]
fn test_fixed_array_index_negative_traps() {
    let (stdout, success) = compile_and_run_with_status(
        r#"
main = ->
    arr = [7, 8, 9]
    arr[(0 - 1)].println!
    0
"#,
    );

    assert!(!success);
    assert!(
        stdout.contains("index out of bounds"),
        "stdout was {stdout:?}"
    );
}

#[test]
fn test_fixed_array_index_upper_bound_traps() {
    let (stdout, success) = compile_and_run_with_status(
        r#"
main = ->
    arr = [7, 8, 9]
    arr[3].println!
    0
"#,
    );

    assert!(!success);
    assert!(
        stdout.contains("index out of bounds"),
        "stdout was {stdout:?}"
    );
}

#[test]
fn test_slice_index_negative_traps() {
    let (stdout, success) = compile_and_run_with_status(
        r#"
read_negative: &[I64] -> I64
read_negative = slice -> slice[(0 - 1)]

main = ->
    arr = [7, 8, 9]
    read_negative &arr .println!
    0
"#,
    );

    assert!(!success);
    assert!(
        stdout.contains("index out of bounds"),
        "stdout was {stdout:?}"
    );
}

#[test]
fn test_slice_index_upper_bound_traps() {
    let (stdout, success) = compile_and_run_with_status(
        r#"
read_upper: &[I64] -> I64
read_upper = slice -> slice[3]

main = ->
    arr = [7, 8, 9]
    read_upper &arr .println!
    0
"#,
    );

    assert!(!success);
    assert!(
        stdout.contains("index out of bounds"),
        "stdout was {stdout:?}"
    );
}

#[test]
fn test_array_len() {
    let output = compile_and_run(
        r#"
main = ->
    arr = [10, 20, 30, 40, 50]
    ~ArrayLen arr .println!
    empty = [1]
    ~ArrayLen empty .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "5");
    assert_eq!(lines[1], "1");
}

#[test]
fn test_array_len_accepts_borrowed_slice() {
    let output = compile_and_run(
        r#"
sum_array: &[I64] -> I64
sum_array = arr ->
    total = 0
    i = 0
    len = ~ArrayLen arr
    while i < len
        total = total + arr[i]
        i = i + 1
    total

main = ->
    arr = [1, 2, 3, 4]
    sum_array &arr .println!
    0
"#,
    );

    assert_eq!(output.trim(), "10");
}

#[test]
fn test_unsupported_concrete_indexing_fails_during_lowering() {
    compile_should_fail(
        r#"
main = ->
    value = 42
    x = value[0]
    0
"#,
        "No implementation found for operator '[]' on type I64",
    );
}

#[test]
fn test_deref_receiver_without_index_support_fails_during_lowering() {
    compile_should_fail(
        r#"
trait Deref
    type Target
    @*: &Self::Target

struct Box T
    < value: T

impl Deref for Box T
    type Target = T
    @* = -> &@value

main = ->
    boxed = Box
        value: 7
    boxed[0]
    0
"#,
        "No implementation found for operator '[]' on type Box I64",
    );
}

#[test]
fn test_vec_as_slice_index_dispatches_to_slice() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 4
    v.push 7
    slice = v.as_slice!
    slice[1].println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_vec_indexing() {
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
fn test_vec_index_assignment() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 4
    v.push 7
    v[1] = 42
    v[1].println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_vec_index_out_of_bounds_traps() {
    let (stdout, success) = compile_and_run_with_status(
        r#"
main = ->
    mut v = Vec::new!
    v.push 10
    slice = v.as_slice!
    slice[1].println!
    0
"#,
    );

    assert!(!success);
    assert!(
        stdout.contains("index out of bounds"),
        "stdout was {stdout:?}"
    );
}

#[test]
fn test_borrowed_vec_can_index_explicit_slice() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 4
    v.push 7
    shared = &v
    slice = (*shared).as_slice!
    slice[1].println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_slice_index_of_slice_returns_inner_slice_ref() {
    let output = compile_and_run(
        r#"
main = ->
    rows = ["ab", "cd"]
    rows[1].println!
    0
"#,
    );

    assert_eq!(output.trim(), "cd");
}

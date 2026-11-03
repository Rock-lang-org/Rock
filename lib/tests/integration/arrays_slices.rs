use crate::support::*;

#[test]
fn test_removed_make_arr_intrinsic_is_rejected() {
    compile_should_fail(r#"
main = !->
    values = [1, 2]
    unsafe ~MakeArr values.as_ptr!, 2
"#, "MakeArr");
}

#[test]
fn test_mutable_raw_slice_borrow_requires_unsafe() {
    compile_should_fail(r#"
main = !->
    mut values = [1, 2]
    raw: *[I64] = ~PtrFromParts values.as_mut_ptr!, 2
    view = &mut *raw
    view[0].println!
"#, "requires an unsafe block");
}

#[test]
fn test_slice_methods_lengths_and_empty_views() {
    let output = compile_and_run(r#"
inspect: &[I64] -> ()
inspect = values !->
    values.len!.println!
    values.is_empty!.println!

main = !->
    mut values = [10, 20, 30]
    values.len!.println!
    values.is_empty!.println!
    inspect &values
    inspect &values[1..]
    inspect &values[3..3]
    view = &mut values[..]
    view.len!.println!
    view.is_empty!.println!
    ptr = view.as_mut_ptr!
    unsafe *ptr = 42
    view[0].println!
    view.len!.println!
    empty: [U8; 0] = []
    empty.len!.println!
    empty.is_empty!.println!
"#);
    assert_eq!(output, "3\nfalse\n3\nfalse\n2\nfalse\n0\ntrue\n3\nfalse\n42\n3\n0\ntrue\n");
}

#[test]
fn test_slice_methods_pointer_identity_and_mutation() {
    let output = compile_and_run(r#"
main = !->
    mut values = [10, 20, 30]
    original = values.as_ptr! as I64
    borrowed = (&values).as_ptr! as I64
    (original == borrowed).println!
    tail = &values[1..]
    offset = unsafe ~PtrOffset values.as_ptr!, 1
    ((tail.as_ptr! as I64) == (offset as I64)).println!
    ptr = values.as_mut_ptr!
    ((ptr as I64) == original).println!
    unsafe *ptr = 42
    values[0].println!
    values[1].println!
"#);
    assert_eq!(output, "true\ntrue\ntrue\n42\n20\n");
}

#[test]
fn test_slice_methods_do_not_move_non_copy_elements() {
    let output = compile_and_run(r#"
main = !->
    values = [(String::from "first"), (String::from "second")]
    values.len!.println!
    ptr = values.as_ptr!
    (unsafe &*ptr).println!
    values[1].println!
    values.len!.println!
"#);
    assert_eq!(output, "2\nfirst\nsecond\n2\n");
}

#[test]
fn test_slice_methods_borrowed_str_bytes_and_pointer() {
    let output = compile_and_run(r#"
main = !->
    text = "aé🦀"
    text.len!.println!
    text.is_empty!.println!
    "".is_empty!.println!
    bytes = text.as_bytes!
    bytes.len!.println!
    bytes[0].println!
    bytes[1].println!
    ((text.as_ptr! as I64) == (bytes.as_ptr! as I64)).println!
    owned = String::from text
    owned.as_str!.as_bytes!.len!.println!
    owned.println!
"#);
    assert_eq!(output, "7\nfalse\ntrue\n7\n97\n195\ntrue\n7\naé🦀\n");
}

#[test]
fn test_slice_methods_shared_slice_cannot_get_mut_pointer() {
    compile_should_fail(r#"
main = !->
    values = [1, 2]
    shared = &values[..]
    ptr = shared.as_mut_ptr!
    unsafe *ptr = 42
"#, "as_mut_ptr");
}

#[test]
fn test_slice_methods_immutable_array_cannot_get_mut_pointer() {
    compile_should_fail(r#"
main = !->
    values = [1, 2]
    ptr = values.as_mut_ptr!
    unsafe *ptr = 42
"#, "mutable receiver");
}

#[test]
fn test_slice_methods_byte_view_cannot_escape_local_string() {
    compile_should_fail(r#"
make_bytes: () -> &[U8]
make_bytes = ->
    text = String::from "local"
    text.as_str!.as_bytes!

main = !->
    make_bytes!.len!.println!
"#, "does not live long enough");
}

#[test]
fn test_string_literal_indexing_is_rejected() {
    compile_should_fail(
        r#"
main = ->
    s = "abc"
    s[1] .println!
    0
"#,
        "No implementation found for operator '[]'",
    );
}

#[test]
fn test_duplicate_generic_parameter_names_are_rejected() {
    compile_should_fail(
        r#"
struct Box T, T
    value: T

main = ->
    0
"#,
        "duplicate generic parameter 'T'",
    );
}

#[test]
fn test_borrowed_u8_slice_struct_field_indexes_as_byte() {
    let output = compile_and_run(
        r#"
struct Holder
    < data: &[U8]

second: &[U8] -> U8
second = s -> s[1]

main = ->
    bytes = [97 as U8, 98 as U8, 99 as U8]
    holder = Holder
        data: &bytes
    slice = holder.data
    (second slice) as I64 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "98");
}

#[test]
fn test_bare_slice_struct_field_is_rejected() {
    compile_should_fail(
        r#"
struct Holder
    < data: [U8]

main = -> 0
"#,
        "bare slice type [T] must be written behind a reference",
    );
}

#[test]
fn test_array_literal_keeps_fixed_array_type_until_coercion() {
    let output = compile_and_run(
        r#"
show_slice: &[I64] -> String
show_slice = s -> s.show!

main = ->
    arr = [1, 2, 3]
    show_slice &arr .println!
    0
"#,
    );

    assert_eq!(output.trim(), "[1, 2, 3]");
}

#[test]
fn test_show_impl_for_slice_resolves_through_builtin_receiver_type() {
    let output = compile_and_run(
        r#"
main = ->
    s = "abc"
    s.println!
    0
"#,
    );

    assert_eq!(output.trim(), "abc");
}

#[test]
fn test_array_is_available_as_user_defined_type_name() {
    let output = compile_and_run(
        r#"
struct Array
    < value: I64

main = ->
    a = Array
        value: 41
    a.value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "41");
}

#[test]
fn test_bare_slice_impl_target_is_rejected() {
    compile_should_fail(
        r#"
trait SliceLen
    @slice_len = -> 0

impl SliceLen for [T]
    @slice_len = -> ~ArrayLen self

main = -> 0
"#,
        "bare slice type [T] must be written behind a reference",
    );
}

#[test]
fn test_bare_str_parameter_is_rejected() {
    compile_should_fail(
        r#"
len: Str -> I64
len = s -> ~ArrayLen s

main = -> 0
"#,
        "bare string slice type Str must be written behind a reference",
    );
}

#[test]
fn test_borrowed_slice_impl_target_dispatches() {
    let output = compile_and_run(
        r#"
trait SliceLen
    @slice_len = -> 0

impl SliceLen for &[T]
    @slice_len = -> ~ArrayLen (*self)

main = ->
    arr = [1, 2, 3]
    (&arr).slice_len!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "3");
}

#[test]
fn test_str_trait_impl_uses_borrowed_str_not_u8_slice() {
    let output = compile_and_run(
        r#"
trait Kind
    @kind = -> 0

impl Kind for &Str
    @kind = -> 7

impl Kind for &[U8]
    @kind = -> 3

main = ->
    s = "abc"
    s.kind!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_u8_slice_trait_impl_does_not_apply_to_string_literal() {
    compile_should_fail(
        r#"
trait BytesOnly
    @bytes_only = -> 0

impl BytesOnly for &[U8]
    @bytes_only = -> 1

main = ->
    s = "abc"
    s.bytes_only!.println!
    0
"#,
        "bytes_only",
    );
}

#[test]
fn test_user_defined_array_does_not_receive_slice_methods() {
    compile_should_fail(
        r#"
struct Array
    < value: I64

main = ->
    a = Array
        value: 1
    a.println!
    0
"#,
        "Unknown field 'println' on struct 'Array'",
    );
}

#[test]
fn test_fixed_array_value_does_not_coerce_to_slice_parameter() {
    compile_should_fail(
        r#"
sum: &[I64] -> I64
sum = s ->
    (s[0] + s[1]) + s[2]

main = ->
    arr = [1, 2, 3]
    sum arr .println!
    0
"#,
        "Type mismatch",
    );
}

#[test]
fn test_fixed_array_value_does_not_coerce_to_slice_return() {
    compile_should_fail(
        r#"
make: () -> &[I64]
make = -> [1, 2, 3]

main = ->
    0
"#,
        "Type mismatch",
    );
}

#[test]
fn test_overapplied_non_unary_call_is_rejected() {
    compile_should_fail(
        r#"
add = x, y -> x + y

main = ->
    add 1, 2, 3 .println!
    0
"#,
        "Type mismatch",
    );
}

#[test]
fn test_fixed_array_borrow_coerces_to_slice_parameter() {
    let output = compile_and_run(
        r#"
sum: &[I64] -> I64
sum = s ->
    (s[0] + s[1]) + s[2]

main = ->
    arr = [1, 2, 3]
    sum &arr .println!
    0
"#,
    );

    assert_eq!(output.trim(), "6");
}

#[test]
fn test_array_literal_cast_seed_infers_remaining_integer_elements() {
    let output = compile_and_run(
        r#"
first_byte: &[U8] -> I64
first_byte = s -> s[0] as I64

main = ->
    buf = [7 as U8, 8, 9]
    first_byte &buf .println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_array_literal_binding_annotation_infers_integer_elements() {
    let output = compile_and_run(
        r#"
main = ->
    buf: [U8; 5] = [1, 2, 3, 4, 5]
    buf[4] as I64 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "5");
}

#[test]
fn test_repeat_array_initializer_is_evaluated_once() {
    let output = compile_and_run(
        r#"
next_byte: &mut I64 -> U8
next_byte = calls ->
    *calls = *calls + 1
    7 as U8

main = ->
    mut calls = 0
    buf: [U8; 256] = [next_byte (&mut calls); 256]
    buf[255] as I64 .println!
    calls.println!
    0
"#,
    );

    assert_eq!(output.trim().lines().collect::<Vec<_>>(), vec!["7", "1"]);
}

#[test]
fn test_repeat_array_binding_annotation_infers_integer_element() {
    let output = compile_and_run(
        r#"
main = ->
    buf: [U8; 256] = [0; 256]
    buf[255] as I64 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "0");
}

#[test]
fn test_repeat_array_rejects_non_copy_initializer() {
    compile_should_fail(
        r#"
main = ->
    values = [String::from "owned"; 2]
    values
    0
"#,
        "array repeat initializer must be Copy",
    );
}

#[test]
fn test_array_literal_mut_slice_use_infers_integer_elements() {
    let output = compile_and_run(
        r#"
write_first: &mut [U8] -> I64
write_first = s ->
    ptr = (~ArrPtr (*s)) as *U8
    unsafe *ptr = 42 as U8
    unsafe *ptr as I64

main = ->
    mut buf = [0, 0, 0, 0, 0]
    slice = &mut buf
    write_first slice .println!
    buf[0] as I64 .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["42", "42"]);
}

#[test]
fn test_array_literal_mut_slice_use_infers_integer_expressions() {
    let output = compile_and_run(
        r#"
first_byte: &mut [U8] -> I64
first_byte = s ->
    ptr = (~ArrPtr (*s)) as *U8
    unsafe *ptr as I64

main = ->
    value = 40
    mut buf = [value + 2, 0, 0]
    slice = &mut buf
    first_byte slice .println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_array_integer_literal_does_not_infer_as_bool() {
    compile_should_fail(
        r#"
main = ->
    buf: [Bool; 1] = [1]
    0
"#,
        "integer literal resolved to non-integer type `Bool`",
    );
}

#[test]
fn test_fixed_array_borrow_coercion_preserves_original_storage_alias() {
    let output = compile_and_run(
        r#"
slice_addr: &[I64] -> I64
slice_addr = s -> (~ArrPtr (*s)) as I64

first_elem_addr: &[I64] -> I64
first_elem_addr = s -> (&((*s)[0]) as *I64) as I64

main = ->
    arr = [1, 2, 3]
    base = slice_addr &arr
    first = first_elem_addr &arr
    if base == first
        1.println!
    else
        0.println!
    0
"#,
    );

    assert_eq!(output.trim(), "1");
}

#[test]
fn test_arr_ptr_rejects_fixed_array_values() {
    compile_should_fail(
        r#"
main = ->
    arr = [1, 2, 3]
    ptr = ~ArrPtr arr
    unsafe *ptr .println!
    0
"#,
        "ArrPtr expected slice",
    );
}

#[test]
fn test_fixed_array_show_uses_slice_impl_via_coercion() {
    let output = compile_and_run(
        r#"
main = ->
    arr = [1, 2, 3]
    arr.println!
    0
"#,
    );

    assert_eq!(output.trim(), "[1, 2, 3]");
}

#[test]
fn test_u8_buffer_show_prints_byte_values() {
    let output = compile_and_run(
        r#"
main = ->
    buf = [104 as U8, 101 as U8, 108 as U8]
    buf.println!
    0
"#,
    );

    assert_eq!(output.trim(), "[104, 101, 108]");
}

#[test]
fn test_explicit_fixed_array_trait_impl_dispatches_at_call_site() {
    let output = compile_and_run(
        r#"
trait FixedArrayCode
    @code = -> 0

impl FixedArrayCode for [I64; 3]
    @code = -> 33

main = ->
    arr: [I64; 3] = [1, 2, 3]
    arr.code!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "33");
}

#[test]
fn test_fixed_array_impl_body_can_use_self_as_array() {
    let output = compile_and_run(
        r#"
trait FixedArraySecond
    @second = -> 0

impl FixedArraySecond for [I64; 3]
    @second = -> self[1]

main = ->
    arr: [I64; 3] = [7, 8, 9]
    arr.second!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "8");
}

#[test]
fn test_generic_fixed_array_impl_method_uses_receiver_type_substitution() {
    let output = compile_and_run(
        r#"
trait EchoArray
    @echo = x -> x

impl EchoArray for [T; 3]
    @echo = x -> x

main = ->
    arr: [I64; 3] = [7, 8, 9]
    arr.echo 7 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_fixed_array_trait_bound_solver_finds_concrete_impl() {
    let output = compile_and_run(
        r#"
trait FixedArrayCode
    @code = -> 0

impl FixedArrayCode for [I64; 3]
    @code = -> 33

use_code: T -> I64 where T: FixedArrayCode
use_code = value -> value.code!

main = ->
    arr: [I64; 3] = [1, 2, 3]
    use_code arr .println!
    0
"#,
    );

    assert_eq!(output.trim(), "33");
}

#[test]
fn test_explicit_fixed_array_index_assignment_updates_storage() {
    let output = compile_and_run(
        r#"
main = ->
    mut arr: [I64; 3] = [1, 2, 3]
    arr[1] = 9
    arr[1].println!
    0
"#,
    );

    assert_eq!(output.trim(), "9");
}

#[test]
fn test_explicit_fixed_array_type_syntax_lowers_successfully() {
    let output = compile_and_run(
        r#"
get_first : [I64; 4] -> I64
get_first = arr ->
    arr[0]

main = ->
    arr: [I64; 4] = [10, 20, 30, 40]
    get_first arr .println!
    0
"#,
    );

    assert_eq!(output.trim(), "10");
}

#[test]
fn test_mut_fixed_array_borrow_coerces_to_mut_slice_parameter() {
    let output = compile_and_run(
        r#"
write_first: &mut [I64] -> I64
write_first = s ->
    ptr = (~ArrPtr (*s)) as *I64
    unsafe *ptr = 9
    unsafe *ptr

main = ->
    mut arr = [1, 2, 3]
    r = &mut arr
    write_first r .println!
    arr[0].println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "9");
    assert_eq!(lines[1], "9");
}

#[test]
fn test_mut_fixed_array_borrow_coerces_to_shared_slice_parameter() {
    let output = compile_and_run(
        r#"
sum: &[I64] -> I64
sum = s ->
    (s[0] + s[1]) + s[2]

main = ->
    mut arr = [1, 2, 3]
    r = &mut arr
    sum r .println!
    0
"#,
    );

    assert_eq!(output.trim(), "6");
}

#[test]
fn test_mut_fixed_array_borrow_dispatches_shared_slice_method() {
    let output = compile_and_run(
        r#"
show_mut: &mut [I64] -> String
show_mut = s -> s.show!

main = ->
    mut arr = [1, 2, 3]
    r = &mut arr
    show_mut r .println!
    0
"#,
    );

    assert_eq!(output.trim(), "[1, 2, 3]");
}

#[test]
fn test_mut_array_borrow_directly_dispatches_shared_slice_method() {
    let output = compile_and_run(
        r#"
main = ->
    mut arr = [1, 2, 3]
    r = &mut arr
    r.println!
    0
"#,
    );

    assert_eq!(output.trim(), "[1, 2, 3]");
}

#[test]
fn test_mut_array_borrow_dispatches_custom_shared_slice_method() {
    let output = compile_and_run(
        r#"
trait SliceLen
    @slice_len = -> 0

impl SliceLen for &[T]
    @slice_len = ->
        ~ArrayLen (*self)

main = ->
    mut arr = [1, 2, 3]
    r = &mut arr
    r.slice_len!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "3");
}

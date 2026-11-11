use crate::support::*;

#[test]
fn test_const_generics_struct_parses_and_reads_elements() {
    let output = compile_and_run(
        r#"
struct Buf T, N: usize
    < data: [T; N]

main = !->
    buf = Buf
        data: [10, 20, 30]
    buf.data[0].println!
    buf.data[2].println!
"#,
    );
    assert_eq!(output.trim(), "10\n30");
}

#[test]
fn test_const_generics_impl_over_arrays_of_any_length() {
    let output = compile_and_run(
        r#"
trait Head
    @head = -> 0

impl Head for [I64; N]
    @head = -> self[0]

main = !->
    a: [I64; 2] = [7, 8]
    b: [I64; 4] = [9, 1, 2, 3]
    a.head!.println!
    b.head!.println!
"#,
    );
    assert_eq!(output.trim(), "7\n9");
}

#[test]
fn test_const_generics_vec_from_array_serves_several_lengths() {
    let output = compile_and_run(
        r#"
main = !->
    three: Vec I64 = Vec::from [1, 2, 3]
    five: Vec I64 = Vec::from [4, 5, 6, 7, 8]
    three.len!.println!
    three[2].println!
    five.len!.println!
    five[4].println!
"#,
    );
    assert_eq!(output.trim(), "3\n3\n5\n8");
}

#[test]
fn test_const_generics_into_vec_from_array() {
    let output = compile_and_run(
        r#"
main = !->
    values: Vec I64 = [1, 2, 3].into!
    values.len!.println!
    values[0].println!
"#,
    );
    assert_eq!(output.trim(), "3\n1");
}

#[test]
fn test_const_generics_generic_function_over_array_length() {
    let output = compile_and_run(
        r#"
first: [T; N] -> T
first = arr -> arr[0]

main = !->
    a: [I64; 3] = [5, 6, 7]
    first a .println!
"#,
    );
    assert_eq!(output.trim(), "5");
}

#[test]
fn test_const_generics_function_monomorphized_for_two_lengths() {
    let output = compile_and_run(
        r#"
first: [T; N] -> T
first = arr -> arr[0]

main = !->
    first [1, 2] .println!
    first [30, 40, 50, 60] .println!
"#,
    );
    assert_eq!(output.trim(), "1\n30");
}

#[test]
fn test_const_generics_function_without_std_exit_status() {
    let status = compile_and_run_without_std(
        r#"
pick: [T; N] -> I64
pick = arr -> 4

main = ->
    pick [4, 9, 11]
"#,
    );
    assert_eq!(status, 4);
}

#[test]
fn test_const_generics_length_mismatch_is_rejected() {
    compile_should_fail(
        r#"
take_three: [I64; 3] -> I64
take_three = arr -> arr[0]

main = !->
    arr: [I64; 2] = [1, 2]
    take_three arr .println!
"#,
        "Type mismatch: [I64; 2] vs [I64; 3]",
    );
}

#[test]
fn test_const_generics_unknown_length_name_is_rejected() {
    compile_should_fail(
        r#"
struct Buf T
    data: [T; Size]

main = !->
    ()
"#,
        "array length 'Size' must name a `usize` const generic parameter",
    );
}

#[test]
fn test_const_generics_name_used_as_type_and_length_is_rejected() {
    compile_should_fail(
        r#"
struct Buf N
    data: [N; N]
    other: N

main = !->
    ()
"#,
        "array length 'N' must name a `usize` const generic parameter",
    );
}

#[test]
fn test_const_generics_length_param_as_value_in_generic_function() {
    let output = compile_and_run(
        r#"
len_of: [T; N] -> I64
len_of = arr -> N

main = !->
    len_of [1, 2, 3] .println!
    len_of [1, 2, 3, 4, 5] .println!
"#,
    );
    assert_eq!(output.trim(), "3\n5");
}

#[test]
fn test_const_generics_length_param_as_value_in_impl_over_arrays() {
    let output = compile_and_run(
        r#"
trait Count
    @count = -> 0

impl Count for [I64; N]
    @count = -> N

main = !->
    a: [I64; 2] = [7, 8]
    b: [I64; 4] = [9, 1, 2, 3]
    a.count!.println!
    b.count!.println!
"#,
    );
    assert_eq!(output.trim(), "2\n4");
}

#[test]
fn test_const_generics_length_param_as_value_in_struct_method() {
    let output = compile_and_run(
        r#"
struct Buf T, N: usize
    < data: [T; N]

impl Buf T, N
    @len = -> N

main = !->
    buf = Buf
        data: [10, 20, 30]
    buf.len!.println!
"#,
    );
    assert_eq!(output.trim(), "3");
}

#[test]
fn test_const_generics_literal_type_argument_accepts_matching_array() {
    let output = compile_and_run(
        r#"
struct Buf T, N: usize
    < data: [T; N]

main = !->
    b: Buf I64, 3 = Buf
        data: [1, 2, 3]
    b.data[2].println!
"#,
    );
    assert_eq!(output.trim(), "3");
}

#[test]
fn test_const_generics_literal_type_argument_length_mismatch_is_rejected() {
    compile_should_fail(
        r#"
struct Buf T, N: usize
    < data: [T; N]

main = !->
    b: Buf I64, 3 = Buf
        data: [1, 2]
    b.data[0].println!
"#,
        "Type mismatch",
    );
}

#[test]
fn test_const_generics_number_for_type_parameter_is_rejected() {
    compile_should_fail(
        r#"
struct Box T
    < value: T

main = !->
    b: Box 3 = Box
        value: 1
"#,
        "found a `usize` constant",
    );
}

#[test]
fn test_const_generics_type_for_usize_parameter_is_rejected() {
    compile_should_fail(
        r#"
struct Buf T, N: usize
    < data: [T; N]

main = !->
    b: Buf I64, I64 = Buf
        data: [1, 2]
"#,
        "expected a `usize` constant",
    );
}

// Struct and enum fields only see the declared parameter list; an implicit parameter would be
// orphaned from it.
#[test]
fn test_const_generics_undeclared_length_in_struct_field_is_rejected() {
    compile_should_fail(
        r#"
struct Buf T
    data: [T; M]

main = !->
    1.println!
"#,
        "array length 'M' must name a `usize` const generic parameter",
    );
}

#[test]
fn test_const_generics_undeclared_length_with_literal_len_in_struct_field_is_rejected() {
    compile_should_fail(
        r#"
struct Buf T
    < data: [M; 3]

main = !->
    ()
"#,
        "unknown type 'M'",
    );
}

#[test]
fn test_undeclared_type_in_struct_field_is_rejected() {
    compile_should_fail(
        r#"
struct Buf T
    < data: M

main = !->
    ()
"#,
        "unknown type 'M'",
    );
}

#[test]
fn test_undeclared_type_in_struct_without_param_list_is_rejected() {
    compile_should_fail(
        r#"
struct Buf
    < data: M

main = !->
    ()
"#,
        "unknown type 'M'",
    );
}

#[test]
fn test_undeclared_type_in_enum_variant_is_rejected() {
    compile_should_fail(
        r#"
enum Choice T
    One M
    Two

main = !->
    ()
"#,
        "unknown type 'M'",
    );
}

#[test]
fn test_const_generics_impl_trait_argument_usize_for_type_parameter_is_rejected() {
    compile_should_fail(
        r#"
struct Buf T, N: usize
    < data: [T; N]

trait Tk A
    @tk = -> 0

impl Tk N for Buf T, N
    @tk = -> 1

main = !->
    ()
"#,
        "kind mismatch: expected a type of kind Type, found a `usize` constant",
    );
}

#[test]
fn test_const_generics_impl_trait_argument_type_for_usize_parameter_is_rejected() {
    compile_should_fail(
        r#"
struct Buf T
    < data: T

trait Sized2 N: usize
    @size = -> 0

impl Sized2 I64 for Buf T
    @size = -> 1

main = !->
    ()
"#,
        "kind mismatch: expected a `usize` constant such as `3`, found a type of kind Type",
    );
}

#[test]
fn test_const_generics_impl_trait_argument_literal_for_usize_parameter_is_accepted() {
    let output = compile_and_run(
        r#"
struct Buf T
    < data: T

trait Sized2 N: usize
    @size = -> 0

impl Sized2 3 for Buf T
    @size = -> 3

main = !->
    b = Buf
        data: 1
    b.size!.println!
"#,
    );
    assert_eq!(output.trim(), "3");
}

#[test]
fn test_const_generics_parenthesized_path_head_with_literal_argument() {
    let output = compile_and_run(
        r#"
struct Buf T, N: usize
    < data: [T; N]

impl Buf T, N
    new = d -> Buf
        data: d

main = !->
    b = (Buf I64, 3)::new [1, 2, 3]
    b.data[2].println!
"#,
    );
    assert_eq!(output.trim(), "3");
}

#[test]
fn test_const_generics_type_alias_over_array_length() {
    let output = compile_and_run(
        r#"
type Row T, N: usize = [T; N]

first: Row I64, 3 -> I64
first = arr -> arr[0]

main = !->
    a: Row I64, 3 = [4, 5, 6]
    first a .println!
"#,
    );
    assert_eq!(output.trim(), "4");
}

#[test]
fn test_const_generics_trait_impl_array_length_in_trait_args() {
    let output = compile_and_run(
        r#"
struct Buf T, N: usize
    < data: [T; N]

trait Convert U
    @convert: U -> U

impl Convert [T; N] for Buf T, N
    @convert = a -> a

main = !->
    b = Buf
        data: [7, 8, 9]
    arr: [I64; 3] = b.convert [1, 8, 3]
    arr[1].println!
"#,
    );
    assert_eq!(output.trim(), "8");
}

#[test]
fn test_const_generics_generic_method_returning_array_through_substitution() {
    let output = compile_and_run(
        r#"
struct Buf T, N: usize
    < data: [T; N]

impl Buf T, N
    @items: [T; N] -> [T; N]
    @items = a -> a

main = !->
    b = Buf
        data: [1, 2, 3, 4]
    arr: [I64; 4] = b.items [1, 2, 3, 4]
    arr[3].println!
"#,
    );
    assert_eq!(output.trim(), "4");
}

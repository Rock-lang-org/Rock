use crate::support::*;

#[test]
fn test_std_into_uses_from_blanket_impl() {
    let output = compile_and_run(
        r#"
struct Small
    < value: I64

struct Big
    < value: I64

impl From Small for Big
    from = small ->
        Big
            value: small.value + 1

main = ->
    small = Small
        value: 41
    big: Big = small.into!
    big.value.println!
    0
"#,
    );
    assert_eq!(output.trim(), "42");
}

#[test]
fn test_static_trait_overloads_infer_arguments_without_std() {
    let status = compile_and_run_without_std(
        r#"
trait Convert T
    convert: T -> Self

struct Value
    < value: I64

impl Convert I64 for Value
    convert = input -> Value
        value: input

impl Convert Bool for Value
    convert = input -> Value
        value: if input
            42
        else
            0

main = ->
    convert_integer: I64 -> Value = Value::convert
    integer = convert_integer 7
    boolean = Value::convert true
    ~I64Add integer.value, boolean.value
"#,
    );
    assert_eq!(status, 49);
}

#[test]
fn test_std_string_from_conversions() {
    let output = compile_and_run(
        r#"
convert: T -> U where U: From T
convert = value -> U::from value

main = !->
    text = String::from "hello"
    text.println!
    text.len!.println!
    42.show!.println!
    (-7).show!.println!
    0.show!.println!
    String::from 3.5 .println!
    String::from 'R' .println!
    String::from "" .len!.println!
    bytes = [82 as U8, 111 as U8, 99 as U8, 107 as U8]
    String::from (&bytes) .println!
    generic_text: String = convert "generic"
    generic_number: String = convert 3.5
    generic_text.println!
    generic_number.println!
    converted: String = 'R'.into!
    converted.println!
    owned = String::from text
    owned.println!
"#,
    );
    assert_eq!(
        output,
        "hello\n5\n42\n-7\n0\n3.5\nR\n0\nRock\ngeneric\n3.5\nR\nhello\n"
    );
}

#[test]
fn test_std_string_from_rejects_unsupported_source() {
    compile_should_fail(
        r#"
main = !->
    String::from true .println!
"#,
        "does not implement trait",
    );
}

#[test]
fn test_std_string_from_rejects_integer_sources() {
    for ty in ["I64", "U64"] {
        compile_should_fail(&format!("main = !->\n    value = 1 as {ty}\n    String::from value .println!\n"), "does not implement trait");
    }
}

#[test]
fn test_std_fixed_u8_arrays_convert_to_string_for_any_length() {
    let output = compile_and_run(
        r#"
main = ->
    short = [82 as U8, 111 as U8, 99 as U8, 107 as U8]
    short_string: String = String::from (&short)
    short_string.println!

    long = [72 as U8, 101 as U8, 108 as U8, 108 as U8, 111 as U8, 44 as U8, 32 as U8, 82 as U8, 111 as U8, 99 as U8, 107 as U8]
    long_string: String = String::from (&long)
    long_string.println!

    from_bytes = [70 as U8, 114 as U8, 111 as U8, 109 as U8]
    from_string: String = String::from (&from_bytes)
    from_string.println!

    empty: [U8; 0] = []
    empty_string: String = String::from (&empty)
    empty_string.len!.println!
    0
"#,
    );

    assert_eq!(output, "Rock\nHello, Rock\nFrom\n0\n");
}

#[test]
fn test_array_print() {
    let output = compile_and_run(
        r#"
main = ->
    // Test 1: Simple integer array
    arr = [1, 2, 3, 4, 5]
    arr.println!

    // Test 2: Array with different values
    numbers = [10, 20, 30]
    numbers.println!

    // Test 3: Empty array
    empty: [I64; 0] = []
    empty.println!

    // Test 4: Single element
    single = [42]
    single.println!

    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "[1, 2, 3, 4, 5]");
    assert_eq!(lines[1], "[10, 20, 30]");
    assert_eq!(lines[2], "[]");
    assert_eq!(lines[3], "[42]");
}

#[test]
fn test_int_to_string() {
    let output = compile_and_run(
        r#"
main = ->
    s = int_to_string 42
    s.println!
    num_str = int_to_string 100
    result = (String::from "Value: ").concat (num_str.clone!)
    result.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "42");
    assert_eq!(lines[1], "Value: 100");
}

#[test]
fn test_std_conversions_return_owned_strings() {
    let (output, success) = compile_and_run_with_status(
        r#"
main = ->
    i = int_to_string 12345
    i.println!
    i.len!.println!

    neg = int_to_string (-7)
    neg.println!
    neg.len!.println!

    f = float_to_string 3.5
    f.println!
    f.len! > 0 .println!

    string_to_int "42" .println!
    string_to_float "2.5" .println!
    0
"#,
    );
    assert!(success, "program failed with stdout: {output:?}");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "12345");
    assert_eq!(lines[1], "5");
    assert_eq!(lines[2], "-7");
    assert_eq!(lines[3], "2");
    assert_eq!(lines[4], "3.5");
    assert_eq!(lines[5], "true");
    assert_eq!(lines[6], "42");
    assert_eq!(lines[7], "2.5");
}

#[test]
fn test_std_string_helpers_return_owned_values() {
    let (output, success) = compile_and_run_with_status(
        r#"
main = ->
    joined: String = string_concat "Rock", "Lang"
    joined.println!
    joined.len!.println!

    bytes = [82 as U8, 111 as U8, 99 as U8, 107 as U8]
    slice_in = &bytes
    sub: Vec U8 = byte_substr slice_in, 1, 2
    sub.len!.println!
    slice = sub.as_slice!
    slice[0] as I64 .println!
    slice[1] as I64 .println!
    0
"#,
    );
    assert!(success, "program failed with stdout: {output:?}");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["RockLang", "8", "2", "111", "99"]);
}

#[test]
fn test_std_string_add_operators() {
    let src = r#"
main = ->
    suffix = String::from "!"
    prefix = "Hello, " + "Rock"
    a = prefix + " language"
    b = "Greeting: " + a
    c = b + suffix
    c.println!
    0
"#;

    let output = compile_and_run(src);
    assert_eq!(output, "Greeting: Hello, Rock language!\n");
}

#[test]
fn test_std_repeated_primitive_show_println_drops_temporaries() {
    let (output, success) = compile_and_run_with_status(
        r#"
main = ->
    i = 0
    while i < 5
        (100 + i).println!
        (i as F64).println!
        true.println!
        'x'.println!
        i = i + 1
    0
"#,
    );
    assert!(success, "program failed with stdout: {output:?}");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines.len(), 20);
    assert_eq!(lines[0], "100");
    assert_eq!(lines[1], "0");
    assert_eq!(lines[2], "true");
    assert_eq!(lines[3], "x");
    assert_eq!(lines[16], "104");
    assert_eq!(lines[18], "true");
    assert_eq!(lines[19], "x");
}

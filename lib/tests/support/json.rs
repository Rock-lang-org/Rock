use super::compile_and_run_at_opt;

fn check_output(source: &str, expected: &str) {
    for optimization in [0, 3] {
        assert_eq!(compile_and_run_at_opt(source, optimization), expected);
    }
}

#[test]
fn json_round_trips_values_and_preserves_numbers_and_object_members() {
    let cases = [
        ("null", "null"),
        (" true\r\n\t", "true"),
        ("false", "false"),
        ("[]", "[]"),
        ("{}", "{}"),
        ("\"\"", "\"\""),
        ("\"hello é中🦀\"", "\"hello é中🦀\""),
        (r#"{"foo": "bar"}"#, r#"{"foo":"bar"}"#),
        (
            r#" { "a" : [null, true, false, {"b": -1.25e+3}], "a": 2 } "#,
            r#"{"a":[null,true,false,{"b":-1.25e+3}],"a":2}"#,
        ),
        ("-0", "-0"),
        ("0.00", "0.00"),
        ("1E-0002", "1E-0002"),
        ("9007199254740993", "9007199254740993"),
        ("18446744073709551616", "18446744073709551616"),
        ("-9223372036854775809", "-9223372036854775809"),
        ("1e9999999999999999999999", "1e9999999999999999999999"),
    ];
    let mut source = String::from(
        r#"
> std::json::parse
> std::json::stringify

check = input !->
    match parse input
        Result::Ok value =>
            match stringify &value
                Result::Ok text => text.println!
                Result::Err error => error.println!
        Result::Err error => error.println!

main = !->
"#,
    );
    let mut expected = String::new();
    for (input, output) in cases {
        source.push_str(&format!("    check {input:?}\n"));
        expected.push_str(output);
        expected.push('\n');
    }
    check_output(&source, &expected);
}

#[test]
fn json_decodes_unicode_and_escapes_strings_and_keys() {
    check_output(
        r#"
> std::json::parse
> std::json::stringify
> std::json::Value

check = input !->
    match parse input
        Result::Ok value =>
            match stringify &value
                Result::Ok text => text.println!
                Result::Err error => error.println!
        Result::Err error => error.println!

main = !->
    check "\"\\\"\\\\\\/\\b\\f\\n\\r\\t\\u0000\\u001f\""
    check "\"\\u00e9\\u4E2D\\uD83E\\uDD80\""
    check "\"\\ud800\\udc00\\uDBFF\\uDFFF\""
    check "{\"a\\n\\\"\\u0000\":\"value\"}"
    match parse "\"a\\u0000b\""
        Result::Ok (Value::String text) =>
            text.len!.println!
            text.as_bytes![1].println!
            text.as_bytes![2].println!
        _ => "wrong value".println!
"#,
        concat!(
            "\"\\\"\\\\/\\b\\f\\n\\r\\t\\u0000\\u001f\"\n",
            "\"é中🦀\"\n",
            "\"\u{10000}\u{10ffff}\"\n",
            "{\"a\\n\\\"\\u0000\":\"value\"}\n",
            "3\n0\n98\n",
        ),
    );
}

#[test]
fn json_rejects_malformed_documents() {
    let cases = [
        "",
        " ",
        "nul",
        "Null",
        "true false",
        "nullx",
        "+1",
        "01",
        "-01",
        "-",
        "1.",
        ".1",
        "1e",
        "1e+",
        "1e-",
        "1e1.2",
        "NaN",
        "Infinity",
        "0x10",
        "[",
        "{",
        "[1,]",
        "[,1]",
        "[1 2]",
        "[1}",
        "{\"a\":1,}",
        "{a:1}",
        "{\"a\" 1}",
        "{\"a\":}",
        "{\"a\":1 \"b\":2}",
        "\"",
        "\"abc",
        "\"a\nb\"",
        "\"a\tb\"",
        "\"\\",
        "\"\\x00\"",
        "\"\\u\"",
        "\"\\u123\"",
        "\"\\u12xz\"",
        "\"\\ud800\"",
        "\"\\udc00\"",
        "\"\\ud800x\"",
        "\"\\ud800\\n\"",
        "\"\\ud800\\u0041\"",
        "\"\\ud800\\ud800\"",
        "/* comment */null",
        "[true] trailing",
        "{\"a\":1}{}",
        "[1][2]",
    ];
    let mut source = String::from(
        r#"
> std::json::parse

check = input !->
    match parse input
        Result::Ok _ => "accepted invalid JSON".println!
        Result::Err error =>
            (error.offset >= 0 && error.offset <= input.len!).println!

main = !->
"#,
    );
    for input in cases {
        source.push_str(&format!("    check {input:?}\n"));
    }
    check_output(&source, &"true\n".repeat(cases.len()));
}

#[test]
fn json_reports_byte_offsets() {
    check_output(
        r#"
> std::json::parse

check = input !->
    match parse input
        Result::Ok _ => "expected error".println!
        Result::Err error => error.println!

main = !->
    check "{\"é\" 1}"
    check "[1,]"
    check "null x"
    check "\"\\q\""
    check "1e+"
"#,
        concat!(
            "expected ':' at byte 6\n",
            "expected JSON value at byte 3\n",
            "trailing characters at byte 5\n",
            "invalid escape at byte 2\n",
            "expected exponent digit at byte 3\n",
        ),
    );
}

#[test]
fn json_builds_values_and_validates_number_construction() {
    check_output(
        r#"
> std::json::Value
> std::json::Number
> std::json::stringify

check_number = text !->
    match Number::parse text
        Result::Ok number => number.as_str!.println!
        Result::Err _ => "invalid".println!

main = !->
    mut members: Vec (String, Value) = Vec::new!
    members.push ((String::from "foo"), (Value::String (String::from "bar")))
    mut items: Vec Value = Vec::new!
    items.push (Value::Number (Number::from_i64 (-9223372036854775807 - 1)))
    items.push (Value::Number (Number::from_i64 9223372036854775807))
    items.push Value::Null
    items.push (Value::Bool false)
    members.push ((String::from "values"), (Value::Array items))
    value = Value::Object members
    match stringify &value
        Result::Ok text => text.println!
        Result::Err error => error.println!
    check_number "1.25e+1000"
    check_number "0"
    check_number "01"
    check_number " 1"
    check_number "1 "
    check_number "1,2"
    check_number "NaN"
    check_number "1e"
"#,
        concat!(
            "{\"foo\":\"bar\",\"values\":[-9223372036854775808,9223372036854775807,null,false]}\n",
            "1.25e+1000\n0\ninvalid\ninvalid\ninvalid\ninvalid\ninvalid\ninvalid\n",
        ),
    );
}

#[test]
fn json_handles_every_control_byte_without_truncation() {
    check_output(
        r#"
> std::json::Value
> std::json::parse
> std::json::stringify

main = !->
    mut bytes: Vec U8 = Vec::new!
    for i in 0..32
        bytes.push (i as U8)
    text = String::from bytes.as_slice!
    original = Value::String text.clone!
    match stringify &original
        Result::Ok encoded =>
            match parse encoded.as_str!
                Result::Ok (Value::String decoded) => (decoded == text).println!
                _ => "decode failed".println!
        Result::Err error => error.println!
    for i in 0..32
        control = String::from (i as Char)
        raw = (String::from "\"").concat control .concat (String::from "\"")
        match parse raw.as_str!
            Result::Ok _ => "accepted control byte".println!
            Result::Err error => (error.offset == 1).println!
    bom = (String::from (65279 as Char)).concat (String::from "null")
    match parse bom.as_str!
        Result::Ok _ => "accepted BOM".println!
        Result::Err error => (error.offset == 0).println!
"#,
        &"true\n".repeat(34),
    );
}

#[test]
fn json_limits_array_and_object_nesting_on_parse_and_write() {
    check_output(
        r#"
> std::json::Value
> std::json::parse
> std::json::stringify

nested_text = count, object ->
    mut bytes: Vec U8 = Vec::new!
    prefix = if object then "{\"a\":" else "["
    suffix = if object then '}' else ']'
    for _ in 0..count
        for byte in prefix.as_bytes!
            bytes.push *byte
    bytes.push ('0' as U8)
    for _ in 0..count
        bytes.push (suffix as U8)
    String::from bytes.as_slice!

check_parse = count, object !->
    text = nested_text count, object
    match parse text.as_str!
        Result::Ok value =>
            match stringify &value
                Result::Ok encoded => (encoded == text).println!
                Result::Err error => error.println!
        Result::Err error => error.message.println!

check_write = object !->
    mut value = Value::Null
    mut i = 0
    while i < 129
        if object
            mut members: Vec (String, Value) = Vec::new!
            members.push ((String::from "a"), value)
            value = Value::Object members
        else
            mut items: Vec Value = Vec::new!
            items.push value
            value = Value::Array items
        i = i + 1
    match stringify &value
        Result::Ok _ => "accepted too much nesting".println!
        Result::Err error => error.message.println!

main = !->
    check_parse 128, false
    check_parse 129, false
    check_parse 128, true
    check_parse 129, true
    check_write false
    check_write true
"#,
        concat!(
            "true\nnesting limit exceeded\n",
            "true\nnesting limit exceeded\n",
            "nesting limit exceeded\nnesting limit exceeded\n",
        ),
    );
}

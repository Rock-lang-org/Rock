use crate::support::*;

#[test]
fn test_utf8_strings_and_range_indexing() {
    let output = compile_and_run(
        r#"
main = !->
    text = "aé中🦀z"
    text.len!.println!
    (&text[1..3]).println!
    (&text[3..=5]).println!
    (&text[6..10]).println!
    (&text[..1]).println!
    (&text[10..]).println!
    (&text[..]).println!
    (&text[11..11]).len!.println!
    owned = String::from text
    (&owned[1..6]).println!
    owned.char_at 3 .println!
    str_char_at text, 5 .println!
    str_char_at text, -1 .println!
    String::from 'é' .println!
    String::from '中' .println!
    String::from '🦀' .println!
    ('🦀' as I64).println!
"#,
    );
    assert_eq!(
        output,
        "11\né\n中\n🦀\na\nz\naé中🦀z\n0\né中\nSome(🦀)\nNone\nNone\né\n中\n🦀\n129408\n"
    );
}

#[test]
fn test_utf8_validation() {
    for bytes in [
        vec![0x80],
        vec![0xc0, 0x80],
        vec![0xc2],
        vec![0xe0, 0x80, 0x80],
        vec![0xed, 0xa0, 0x80],
        vec![0xf0, 0x80, 0x80, 0x80],
        vec![0xf4, 0x90, 0x80, 0x80],
        vec![0xf5, 0x80, 0x80, 0x80],
        vec![0xc2, 0x41],
        vec![0xff],
    ] {
        let values = bytes
            .iter()
            .map(|b| format!("{b} as U8"))
            .collect::<Vec<_>>()
            .join(", ");
        let source = format!(
            "main = !->\n    bytes = [{values}]\n    String::from_utf8 (&bytes) .println!\n"
        );
        assert_eq!(compile_and_run(&source), "None\n", "{bytes:?}");
    }
    assert_eq!(
        compile_and_run(
            r#"
main = !->
    bytes = [240 as U8, 159 as U8, 166 as U8, 128 as U8]
    String::from_utf8 (&bytes) .println!
    empty: [U8; 0] = []
    String::from_utf8 (&empty) .println!
"#
        ),
        "Some(🦀)\nSome()\n"
    );
}

#[test]
fn test_utf8_invalid_ranges_are_checked() {
    for range in [
        "2..3",
        "1..2",
        "2..2",
        "-1..1",
        "3..1",
        "0..5",
        "..=4",
        "..=9223372036854775807",
    ] {
        for receiver in ["\"aéz\"", "String::from \"aéz\""] {
            let source =
                format!("main = !->\n    text = {receiver}\n    (&text[{range}]).println!\n");
            let (output, success) = compile_and_run_with_status(&source);
            assert!(!success, "{receiver}[{range}] unexpectedly succeeded");
            assert!(
                output.contains("out of bounds") || output.contains("UTF-8 boundary"),
                "{output}"
            );
        }
    }
}

#[test]
fn test_utf8_char_width_escapes_and_nul() {
    let output = compile_and_run(
        r#"
echo: Char -> Char
echo = value -> value
main = !->
    chars = ['é', '中', '🦀']
    chars[2].println!
    echo '中' .println!
    ('\n' as I64).println!
    ('\'' as I64).println!
    ('\\' as I64).println!
    text = String::from "é\0🦀"
    text.len!.println!
    text.println!
    str_char_at "é", 1 .println!
    String::from (1114111 as Char) .len!.println!
    match '\n'
        '\n' => 1.println!
        _ => 0.println!
"#,
    );
    assert_eq!(output, "🦀\n中\n10\n39\n92\n7\né\0🦀\nSome(́)\n4\n1\n");
}

#[test]
fn test_utf8_invalid_char_casts_trap_before_narrowing() {
    for value in [-1_i64, 0xd800, 0xdfff, 0x110000, 0x100000041] {
        let source = format!("main = !->\n    value = {value}\n    (value as Char).println!\n");
        assert!(
            !compile_and_run_with_status(&source).1,
            "{value} must not form a Char"
        );
    }
}

#[test]
fn test_utf8_invalid_byte_conversion_is_checked() {
    let (output, success) = compile_and_run_with_status(
        r#"
main = !->
    bytes = [255 as U8]
    String::from (&bytes) .println!
"#,
    );
    assert!(!success);
    assert_eq!(output, "invalid UTF-8\n");
}

#[test]
fn test_utf8_scalar_boundaries_roundtrip() {
    let scalars = [
        0_u32, 0x7f, 0x80, 0x7ff, 0x800, 0xd7ff, 0xe000, 0xffff, 0x10000, 0x10ffff,
    ];
    let mut source = String::from("main = !->\n");
    let mut expected = String::new();
    for (i, scalar) in scalars.into_iter().enumerate() {
        source.push_str(&format!(
            "    text{i} = String::from ({scalar} as Char)\n    is_utf8 text{i}.as_bytes! .println!\n    text{i}.len!.println!\n    decoded{i} = text{i}.char_at 0 .unwrap_or 'x'\n    (decoded{i} as I64).println!\n"
        ));
        expected.push_str(&format!(
            "true\n{}\n{scalar}\n",
            char::from_u32(scalar).unwrap().len_utf8()
        ));
    }
    assert_eq!(compile_and_run(&source), expected);
}

#[test]
fn test_utf8_char_literals_require_one_scalar() {
    for literal in ["''", "'ab'", "'é'", "'\\q'"] {
        compile_should_fail(
            &format!("main = !->\n    {literal}.println!\n"),
            "character literal must contain exactly one Unicode scalar",
        );
    }
}

#[test]
fn test_utf8_strings_do_not_allow_byte_mutation_or_integer_indexing() {
    compile_should_fail(
        r#"
main = !->
    text = String::from "é"
    text[0].println!
"#,
        "No implementation found for operator '[]'",
    );
    compile_should_fail(
        r#"
main = !->
    mut text = String::from "é"
    text[0..2] = "hi"
"#,
        "No mutable indexing implementation found",
    );
}

#[test]
fn test_utf8_string_slice_cannot_outlive_owner() {
    compile_should_fail(
        r#"
bad: () -> &Str
bad = ->
    text = String::from "é🦀"
    &text[2..]

main = !-> bad!.println!
"#,
        "Cannot return reference",
    );
}

#[test]
fn test_str_indexing() {
    compile_should_fail(
        r#"
main = ->
    s = "hello"
    s[0].println!
    0
"#,
        "No implementation found for operator '[]'",
    );
}

#[test]
fn test_str_literal_index_out_of_bounds_is_rejected_as_str_indexing() {
    compile_should_fail(
        r#"
main = ->
    c = "hello"[10]
    c.println!
    0
"#,
        "No implementation found for operator '[]'",
    );
}

#[test]
fn test_str_variable_indexing_is_rejected() {
    compile_should_fail(
        r#"
main = ->
    s = "hello"
    i = 3
    s[i].println!
    0
"#,
        "No implementation found for operator '[]'",
    );
}

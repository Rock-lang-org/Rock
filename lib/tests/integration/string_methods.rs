use crate::support::*;

#[test]
fn test_string_in_place_building() {
    let output = compile_and_run(
        r#"
main = !->
    mut s = String::with_capacity 2
    s.push_str "ab"
    s.push 'é'
    s.push_str "cdefghij"
    s.println!
    s.len!.println!
    (s.capacity! >= s.len!).println!
    s.pop!.println!
    s.pop!.println!
    s.truncate 4
    s.println!
    s.pop!.println!
    s.insert_str 1, "XY"
    s.println!
    s.insert_str 0, "<"
    end = s.len!
    s.insert_str end, ">"
    s.println!
    s.clear!
    s.len!.println!
    s.pop!.println!
"#,
    );
    assert_eq!(
        output,
        "abécdefghij\n12\ntrue\nSome(j)\nSome(i)\nabé\nSome(é)\naXYb\n<aXYb>\n0\nNone\n"
    );
}

#[test]
fn test_string_searching() {
    let output = compile_and_run(
        r#"
main = !->
    text = "one two one"
    (text.contains "two").println!
    (text.contains "six").println!
    (text.find "one").println!
    (text.rfind "one").println!
    (text.find "six").println!
    (text.starts_with "one").println!
    (text.ends_with "two").println!
    (text.matches_count "one").println!
    ("aaaa".matches_count "aa").println!
    owned = String::from text
    (owned.find "two").println!
    (owned.ends_with "one").println!
"#,
    );
    assert_eq!(
        output,
        "true\nfalse\nSome(0)\nSome(8)\nNone\ntrue\nfalse\n2\n2\nSome(4)\ntrue\n"
    );
}

#[test]
fn test_string_splitting_and_join() {
    let output = compile_and_run(
        r#"
main = !->
    parts = "a,b,,c".split ","
    parts.len!.println!
    (parts.join "|").println!
    ("a,b,c".splitn 2, ",").join "|" .println!
    ("".split ",").len!.println!
    match "key=value=x".split_once "="
        Option::Some pair =>
            pair.0.println!
            pair.1.println!
        Option::None => "none".println!
    match "key=value=x".rsplit_once "="
        Option::Some pair =>
            pair.0.println!
            pair.1.println!
        Option::None => "none".println!
    match "x".split_once "="
        Option::Some _ => "some".println!
        Option::None => "none".println!
    ("l1\r\nl2\n\nl4\n".lines!).join "|" .println!
    ("  hello \t wörld\n ".split_whitespace!).join "|" .println!
"#,
    );
    assert_eq!(
        output,
        "4\na|b||c\na|b,c\n1\nkey\nvalue=x\nkey=value\nx\nnone\nl1|l2||l4\nhello|wörld\n"
    );
}

#[test]
fn test_string_trimming_and_slices() {
    let output = compile_and_run(
        r#"
main = !->
    text = " \t hé \n"
    ("[" + text.trim! + "]").println!
    ("[" + text.trim_start! + "]").println!
    ("[" + text.trim_end! + "]").println!
    ("   ".trim!).len!.println!
    ("prefix.rk".strip_prefix "prefix").println!
    ("prefix.rk".strip_suffix ".rk").println!
    ("prefix.rk".strip_prefix "x").println!
    ("héllo".substr 0, 3).println!
    ("héllo".substr 1, 1).println!
    ("héllo".substr 4, 9).println!
"#,
    );
    assert_eq!(
        output,
        "[hé]\n[hé \n]\n[ \t hé]\n0\nSome(.rk)\nSome(prefix)\nNone\nSome(hé)\nNone\nNone\n"
    );
}

#[test]
fn test_string_building_conversions() {
    let output = compile_and_run(
        r#"
main = !->
    ("a-b-c".replace "-", "+").println!
    ("a-b-c".replacen "-", "", 1).println!
    ("abc".replace "", "x").println!
    ("ab".repeat 3).println!
    ("ab".repeat 0).len!.println!
    ("Hé, Rock!".to_uppercase!).println!
    ("Hé, Rock!".to_lowercase!).println!
    ("RoCk".eq_ignore_ascii_case "rOcK").println!
    ("Rock".eq_ignore_ascii_case "Roc").println!
    ("aé中🦀".char_count!).println!
    s: String = "x".to_string!
    s.println!
"#,
    );
    assert_eq!(
        output,
        "a+b+c\nab-c\nabc\nababab\n0\nHé, ROCK!\nhé, rock!\ntrue\nfalse\n4\nx\n"
    );
}

#[test]
fn test_string_parsing() {
    let output = compile_and_run(
        r#"
main = !->
    ("42".parse_i64!).println!
    ("-17".parse_i64!).println!
    ("+5".parse_i64!).println!
    ("9223372036854775807".parse_i64!).println!
    ("-9223372036854775808".parse_i64!).println!
    ("9223372036854775808".parse_i64!).println!
    ("".parse_i64!).println!
    ("-".parse_i64!).println!
    ("12a".parse_i64!).println!
    ("2.5".parse_f64!).println!
    ("-1e3".parse_f64!).println!
    (".5".parse_f64!).println!
    ("1e".parse_f64!).println!
    (".".parse_f64!).println!
    ("true".parse_bool!).println!
    ("false".parse_bool!).println!
    ("yes".parse_bool!).println!
"#,
    );
    assert_eq!(
        output,
        "Some(42)\nSome(-17)\nSome(5)\nSome(9223372036854775807)\nSome(-9223372036854775808)\nNone\nNone\nNone\nNone\nSome(2.5)\nSome(-1000)\nSome(0.5)\nNone\nNone\nSome(true)\nSome(false)\nNone\n"
    );
}

#[test]
fn test_char_helpers() {
    let output = compile_and_run(
        r#"
main = !->
    ' '.is_whitespace!.println!
    '　'.is_whitespace!.println!
    'a'.is_whitespace!.println!
    '7'.is_digit!.println!
    'x'.is_alphabetic!.println!
    '_'.is_alphanumeric!.println!
    'Q'.is_uppercase!.println!
    'q'.is_lowercase!.println!
    'q'.to_ascii_uppercase!.println!
    'Q'.to_ascii_lowercase!.println!
    ('f'.to_digit 16).println!
    ('9'.to_digit 8).println!
    '🦀'.len_utf8!.println!
"#,
    );
    assert_eq!(
        output,
        "true\ntrue\nfalse\ntrue\ntrue\nfalse\ntrue\ntrue\nQ\nq\nSome(15)\nNone\n4\n"
    );
}

#[test]
fn test_string_ordering_and_hashing() {
    let output = compile_and_run(
        r#"
main = !->
    a = String::from "apple"
    b = String::from "banana"
    (a < b).println!
    (b <= a).println!
    (a.hash! == "apple".hash!).println!
    mut counts: HashMap String, I64 = HashMap::new!
    counts.insert (String::from "k"), 3
    (counts.contains_key &(String::from "k")).println!
    (counts.contains_key &(String::from "z")).println!
"#,
    );
    assert_eq!(output, "true\nfalse\ntrue\ntrue\nfalse\n");
}

#[test]
fn test_string_methods_book_examples() {
    let chapter = include_str!("../../../docs/src/std/collections.md");
    let section = chapter
        .split_once("## String methods")
        .unwrap()
        .1
        .split_once("## `Vec T`")
        .unwrap()
        .0;
    let listings: Vec<&str> = section
        .split("```rock\n")
        .skip(1)
        .map(|block| block.split_once("```").unwrap().0)
        .collect();
    assert_eq!(listings.len(), 2);
    assert_eq!(
        compile_and_run(listings[0]),
        "name = Rock\ntrue\nSome(5)\nname\nROCK\nred, green, blue\n4\na+b+c\nSome(42)\nNone\n"
    );
    assert_eq!(
        compile_and_run(listings[1]),
        "Hello, Rock!\nSome(!)\nHello\n0\n"
    );
}

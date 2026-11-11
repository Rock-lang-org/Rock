use crate::support::*;

#[test]
fn test_std_option_methods() {
    let output = compile_and_run(
        r#"
inc = x -> x + 1

to_even = x ->
    if x % 2 == 0
        Option::Some x
    else
        Option::None

main = ->
    some_map = Option::Some 41
    none_map = Option::None
    some_and_then = Option::Some 41
    none_fold = Option::None
    nested = Option::Some (Option::Some 9)
    some_fold = Option::Some 41
    some_ok_or: Result I64, I64 = Option::Some 12 .ok_or 5
    none_ok_or: Result I64, I64 = Option::None .ok_or 7
    some_folded: I64 = some_fold.fold
        0
        x -> x + 1
    none_folded: I64 = none_fold.fold
        7
        x -> x + 1

    (some_map.map inc).unwrap_or 0 .println!
    (none_map.map inc).unwrap_or 0 .println!
    (some_and_then.and_then to_even).unwrap_or 0 .println!
    ((Option::Some 8).and_then to_even).unwrap_or 0 .println!
    (nested.flatten!).unwrap_or 0 .println!
    some_folded.println!
    none_folded.println!
    some_ok_or.fold (x -> x), (x -> x) .println!
    none_ok_or.fold (x -> x), (x -> x) .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["42", "0", "0", "8", "9", "42", "7", "12", "7"]);
}

#[test]
fn test_std_option_inspect_borrows_payload_and_returns_option() {
    let output = compile_and_run(
        r#"
print_ref: &I64 -> I32
print_ref = x -> *x .println!

main = ->
    value = (Option::Some 4).inspect print_ref
    value.unwrap_or 0 .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["4", "4"]);
}

#[test]
fn test_std_option_inspect_owned_string_borrows_and_returns_owned_option() {
    let output = compile_and_run(
        r#"
print_len: &String -> I32
print_len = value -> value.len! .println!

main = ->
    value = Option::Some (String::from "hello")
    returned = value.inspect print_len
    returned.println!
    0
"#,
    );

    assert_eq!(output.trim(), "5\nSome(hello)");
}

#[test]
fn test_std_option_inspect_owned_string_consumes_receiver() {
    compile_should_fail(
        r#"
print_len: &String -> I32
print_len = value -> value.len! .println!

main = ->
    value = Option::Some (String::from "hello")
    returned = value.inspect print_len
    returned.println!
    value.println!
    0
"#,
        "borrow of moved value",
    );
}

#[test]
fn test_std_result_methods() {
    let output = compile_and_run(
        r#"
inc = x -> x + 1
tag = err -> err + 5
id_i64: I64 -> I64
id_i64 = x -> x
inc1 = x -> x + 1

parse = x ->
    if x > 0
        Result::Ok x
    else
        Result::Err 3

main = ->
    ok_map: Result I64, I64 = Result::Ok 41
    err_map: Result I64, I64 = Result::Err 4
    err_map_err: Result I64, I64 = Result::Err 4
    ok_and_then: Result I64, I64 = Result::Ok 41
    err_or: Result I64, I64 = Result::Err 4
    nested_ok: Result (Result I64, I64), I64 = Result::Ok (Result::Ok 9)
    nested_err: Result (Result I64, I64), I64 = Result::Ok (Result::Err 6)
    ok_fold: Result I64, I64 = Result::Ok 41
    err_fold: Result I64, I64 = Result::Err 4
    (ok_map.map inc).unwrap_or 0 .println!
    (err_map.map inc).unwrap_or 0 .println!
    (err_map_err.map_err tag).fold id_i64, id_i64 .println!
    (ok_and_then.and_then parse).unwrap_or 0 .println!
    ((Result::Ok (-1)).and_then parse).unwrap_or 0 .println!
    (err_or.or (Result::Ok 9)).unwrap_or 0 .println!
    (nested_ok.flatten!).unwrap_or 0 .println!
    (nested_err.flatten!).fold id_i64, id_i64 .println!
    (ok_fold.fold id_i64, inc1) .println!
    (err_fold.fold id_i64, id_i64) .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(
        lines,
        vec!["42", "0", "9", "41", "0", "9", "9", "6", "42", "4"]
    );
}

#[test]
fn test_std_option_result_fp_operators() {
    let output = compile_and_run(
        r#"
inc = x -> x + 1

keep_even_opt = x ->
    if x % 2 == 0
        Option::Some x
    else
        Option::None

keep_even_res = x ->
    if x % 2 == 0
        Result::Ok x
    else
        Result::Err 1

main = ->
    some = Option::Some 4
    none: Option I64 = Option::None
    fallback_none: Option I64 = Option::None
    some_for_result: Option I64 = Option::Some 12
    none_for_result: Option I64 = Option::None
    wrapped_inc: Option (I64 -> I64) = Option::Some inc
    ok: Result I64, I64 = Result::Ok 4
    err: Result I64, I64 = Result::Err 7
    mapped_ok: Result I64, I64 = Result::Ok 4
    mapped_ok_reverse: Result I64, I64 = Result::Ok 4
    fallback_err: Result I64, I64 = Result::Err 7
    wrapped_res_inc: Result (I64 -> I64), I64 = Result::Ok inc

    (some >>= keep_even_opt).unwrap_or 0 .println!
    (none >>= keep_even_opt).unwrap_or 0 .println!
    (inc <$> Option::Some 4).unwrap_or 0 .println!
    ((Option::Some 4) <&> inc).unwrap_or 0 .println!
    (wrapped_inc <*> Option::Some 4).unwrap_or 0 .println!
    (fallback_none <|> Option::Some 9).unwrap_or 0 .println!
    (some_for_result !> 5).fold (x -> x), (x -> x) .println!
    (none_for_result !> 7).fold (x -> x), (x -> x) .println!

    (ok >>= keep_even_res).unwrap_or 0 .println!
    (err >>= keep_even_res).unwrap_or 0 .println!
    (inc <$> mapped_ok).unwrap_or 0 .println!
    (mapped_ok_reverse <&> inc).unwrap_or 0 .println!
    (wrapped_res_inc <*> Result::Ok 4).unwrap_or 0 .println!
    (fallback_err <|> Result::Ok 9).unwrap_or 0 .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(
        lines,
        vec!["4", "0", "5", "5", "5", "9", "12", "7", "4", "0", "5", "5", "5", "9"]
    );
}

#[test]
fn test_std_result_bifunctor_and_error_operator() {
    let output = compile_and_run(
        r#"
> std::functional::first
> std::functional::second

inc = value -> value + 1
double = value -> value * 2
add_ten = value -> value + 10
triple = value -> value * 3
double_after_inc = value -> double (inc value)
triple_after_add_ten = value -> triple (add_ten value)
identity = value -> value

main = ->
    mapped_ok: Result I64, I64 = bimap inc, add_ten, (Result::Ok 4)
    mapped_err: Result I64, I64 = bimap inc, add_ten, (Result::Err 7)
    first_ok: Result I64, I64 = first inc, (Result::Ok 4)
    second_err: Result I64, I64 = second add_ten, (Result::Err 7)
    operator_err: Result I64, I64 = (Result::Err 7) <!> add_ten

    identity_ok: Result I64, I64 = bimap identity, identity, (Result::Ok 4)
    identity_err: Result I64, I64 = bimap identity, identity, (Result::Err 7)

    composed_ok_l: Result I64, I64 = bimap double_after_inc, triple_after_add_ten, (Result::Ok 4)
    composed_ok_r: Result I64, I64 = bimap double, triple, (bimap inc, add_ten, (Result::Ok 4))
    composed_err_l: Result I64, I64 = bimap double_after_inc, triple_after_add_ten, (Result::Err 7)
    composed_err_r: Result I64, I64 = bimap double, triple, (bimap inc, add_ten, (Result::Err 7))

    mapped_ok.println!
    mapped_err.println!
    first_ok.println!
    second_err.println!
    operator_err.println!
    identity_ok.println!
    identity_err.println!
    composed_ok_l.println!
    composed_ok_r.println!
    composed_err_l.println!
    composed_err_r.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(
        lines,
        vec![
            "Ok(5)", "Err(17)", "Ok(5)", "Err(17)", "Err(17)", "Ok(4)", "Err(7)", "Ok(10)",
            "Ok(10)", "Err(51)", "Err(51)",
        ]
    );
}

#[test]
fn test_std_show_default_println() {
    let src = r#"
struct Named
    < value: I64

impl Show for Named
    @show = -> "Named(" + (int_to_string self.value) + ")"

main = ->
    n = Named
        value: 7
    result: Result I64, I64 = Result::Ok 4
    n.println!
    Option::Some 3 .println!
    result.println!
    0
"#;

    let output = compile_and_run(src);
    assert_eq!(output, "Named(7)\nSome(3)\nOk(4)\n");
}

#[test]
fn test_std_option_result_show_prints_payloads() {
    let output = compile_and_run(
        r#"
> std::control::Option
> std::control::Result
> std::io::IoError

main = ->
    none: Option I64 = Option::None
    ok_41: Result I64, I64 = Result::Ok 41
    err_4: Result I64, I64 = Result::Err 4
    ok_5: Result I64, I64 = Result::Ok 5
    (Option::Some 3).println!
    none.println!
    ok_41.println!
    err_4.println!
    ok_5.println!
    err_io_a: Result I64, IoError = Result::Err (IoError::Os (0 as I32))
    err_io_b: Result I64, IoError = Result::Err (IoError::Os (0 as I32))
    err_io_a.println!
    err_io_b.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(
        lines,
        vec![
            "Some(3)",
            "None",
            "Ok(41)",
            "Err(4)",
            "Ok(5)",
            "Err(IoError(Os: 0))",
            "Err(IoError(Os: 0))"
        ]
    );
}

#[test]
fn test_std_option_show_owned_string_is_non_consuming() {
    let output = compile_and_run(
        r#"
main = ->
    value = Option::Some (String::from "hello")
    value.println!
    value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "Some(hello)\nSome(hello)");
}

#[test]
fn test_std_result_show_owned_string_is_non_consuming() {
    let output = compile_and_run(
        r#"
main = ->
    ok: Result String, String = Result::Ok (String::from "hello")
    err: Result String, String = Result::Err (String::from "oops")
    ok.println!
    ok.println!
    err.println!
    err.println!
    0
"#,
    );

    assert_eq!(output.trim(), "Ok(hello)\nOk(hello)\nErr(oops)\nErr(oops)");
}

#[test]
fn test_std_option_map_owned_string_consumes_receiver() {
    compile_should_fail(
        r#"
main = ->
    value = Option::Some (String::from "hello")
    mapped = value.map (s -> s.len!)
    mapped.unwrap_or 0 .println!
    again = value.show!
    again.println!
    0
"#,
        "borrow of moved value",
    );
}

#[test]
fn test_std_result_map_owned_string_consumes_receiver() {
    compile_should_fail(
        r#"
main = ->
    value: Result String, String = Result::Ok (String::from "hello")
    mapped = value.map (s -> s.len!)
    mapped.unwrap_or 0 .println!
    again = value.show!
    again.println!
    0
"#,
        "borrow of moved value",
    );
}

#[test]
fn test_std_result_map_err_owned_string_consumes_receiver() {
    compile_should_fail(
        r#"
main = ->
    value: Result String, String = Result::Err (String::from "oops")
    mapped = value.map_err (s -> s.len!)
    (mapped.fold (n -> n), (s -> s.len!)) .println!
    again = value.show!
    again.println!
    0
"#,
        "borrow of moved value",
    );
}

#[test]
fn test_std_option_bind_owned_string_forwards_move_receiver() {
    let output = compile_and_run(
        r#"
bind: String -> Option I64
bind = s -> Option::Some (s.len!)

main = ->
    value = Option::Some (String::from "hello")
    mapped = value >>= bind
    mapped.unwrap_or 0 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "5");
}

#[test]
fn test_std_result_bind_owned_string_forwards_move_receiver() {
    let output = compile_and_run(
        r#"
bind: String -> Result I64, String
bind = s -> Result::Ok (s.len!)

main = ->
    value: Result String, String = Result::Ok (String::from "hello")
    mapped = value >>= bind
    mapped.unwrap_or 0 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "5");
}

#[test]
fn test_result_method_sections_preserve_unresolved_payload_until_materialization() {
    let output = compile_and_run(
        r#"
> std::control::Result

struct Source
    < value: I64

struct Sink
    < value: I64

impl Source
    @next: Result Sink, I64
    @next = -> Result::Ok (Sink
        value: self.value)

impl Sink
    @finish: Result I64, I64
    @finish = -> Result::Ok self.value

main = ->
    source = Source
        value: 42
    initial: Result Source, I64 = Result::Ok source
    match (initial >>= (.next!) >>= (.finish!))
        Result::Ok result =>
            result.println!
            0
        Result::Err _ => 1
"#,
    );

    assert_eq!(output.trim(), "42");
}

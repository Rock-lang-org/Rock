use crate::support::*;
use std::fs;
use std::process::Command;
use std::sync::atomic::Ordering;

#[test]
fn test_hello_world() {
    let output = compile_example("hello");
    assert_eq!(output.trim(), "Hello, World!");
}

#[test]
fn test_std_program_args_are_owned_and_available_outside_main() {
    let output = compile_and_run_with_args(
        r#"
> std::env::args

read_args = -> args!

main = ->
    first = read_args!
    second = read_args!
    first.len!.println!
    first[1].println!
    second[2].println!
    0
"#,
        &["alpha", "two words"],
    );

    assert_eq!(output, "3\nalpha\ntwo words\n");
}

#[test]
fn test_arithmetic() {
    let output = compile_example("arithmetic");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "30");
    assert_eq!(lines[1], "205");
}

#[test]
fn test_functions() {
    let output = compile_example("functions");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "7");
    assert_eq!(lines[1], "25");
    assert_eq!(lines[2], "25");
}

#[test]
fn test_if_else() {
    let output = compile_example("if_else");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "5");
    assert_eq!(lines[1], "10");
    assert_eq!(lines[2], "42");
}

#[test]
fn test_structs() {
    let output = compile_example("structs");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "20");
    assert_eq!(lines[2], "30");
}

#[test]
fn test_while_loop() {
    let output = compile_example("while_loop");
    let lines: Vec<&str> = output.trim().lines().collect();
    for i in 0..10 {
        assert_eq!(lines[i], i.to_string());
    }
    assert_eq!(lines[10], "done");
}

#[test]
fn test_for_loop() {
    let output = compile_example("for_loop");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "20");
    assert_eq!(lines[2], "30");
    assert_eq!(lines[3], "40");
    assert_eq!(lines[4], "50");
    assert_eq!(lines[5], "done");
}

#[test]
fn test_recursion() {
    let output = compile_example("recursion");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "3628800"); // 10!
    assert_eq!(lines[1], "55"); // fib(10)
}

#[test]
fn test_match_expr() {
    let output = compile_example("match_expr");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "0");
    assert_eq!(lines[1], "10");
    assert_eq!(lines[2], "20");
    assert_eq!(lines[3], "99");
}

#[test]
fn test_nested_control_flow() {
    let output = compile_example("nested");
    let lines: Vec<&str> = output.trim().lines().collect();
    // fizzbuzz for 1-20
    let expected = [
        "3", "3", "1", "3", "2", "1", "3", "3", "1", "2", "3", "1", "3", "3", "0", "3", "3", "1",
        "3", "2",
    ];
    for (i, exp) in expected.iter().enumerate() {
        assert_eq!(lines[i], *exp, "Mismatch at position {}", i);
    }
}

#[test]
fn test_mut_ref_assign() {
    let output = compile_example("mir_tests/mut_ref_assign");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "2");
}

#[test]
fn test_std_math() {
    let output = compile_and_run(
        r#"
main = ->
    std::ffi::libc::sqrt 144.0 .println!
    std::ffi::libc::sin 0.0 .println!
    std::ffi::libc::cos 0.0 .println!
    std::ffi::libc::pow 2.0, 10.0 .println!
    std::ffi::libc::ceil 3.2 .println!
    std::ffi::libc::floor 3.8 .println!
    "hello".len!.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "12"); // sqrt(144)
    assert_eq!(lines[1], "0"); // sin(0)
    assert_eq!(lines[2], "1"); // cos(0)
    assert_eq!(lines[3], "1024"); // pow(2,10)
    assert_eq!(lines[4], "4"); // ceil(3.2)
    assert_eq!(lines[5], "3"); // floor(3.8)
    assert_eq!(lines[6], "5"); // strlen("hello")
}

#[test]
fn test_std_prelude_extern_is_available_without_explicit_import() {
    let output = compile_and_run(
        r#"
main = ->
    sqrt 144.0 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "12");
}

#[test]
fn test_function_name_can_match_c_extern_symbol() {
    let output = compile_and_run(
        r#"
> std::io::net::TcpListener

main = ->
    listen! .println!
    0

listen = -> 42
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_local_binding_shadows_top_level_function() {
    let output = compile_and_run(
        r#"
offset = x -> x + 2

uses_name_locally = ->
    offset = 40
    offset + 1

main = !->
    uses_name_locally!.println!
    (offset 1).println!
"#,
    );

    assert_eq!(output.trim().lines().collect::<Vec<_>>(), ["41", "3"]);
}

#[test]
fn test_inline_program() {
    let output = compile_and_run(
        r#"
max = a, b ->
    if a > b then a else b

min = a, b ->
    if a < b then a else b

main = ->
    max 10, 20 .println!
    min 10, 20 .println!
    max 5, 3 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "20");
    assert_eq!(lines[1], "10");
    assert_eq!(lines[2], "5");
}

#[test]
fn test_inline_nested_functions() {
    let output = compile_and_run(
        r#"
double = x -> x * 2
triple = x -> x * 3

main = ->
    double 5 .println!
    triple 5 .println!
    double (triple 3) .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "15");
    assert_eq!(lines[2], "18");
}

#[test]
fn unit_arrow_evaluates_body_and_discards_named_function_and_lambda_results() {
    let output = compile_and_run(
        r#"
discard = value !->
    7.println!
    value

emit_value = ->
    9.println!
    42

main = !->
    discard 99
    f = value !->
        8.println!
        value
    f 9
    run_effect = !-> emit_value!
    run_effect!
    emit_value!
"#,
    );

    assert_eq!(
        output.trim().lines().collect::<Vec<_>>(),
        ["7", "8", "9", "9"]
    );
}

#[test]
fn unit_arrow_main_exits_zero_without_std() {
    for source in [
        "main = !-> 42\n",
        "main = !-> (42, 7)\n",
        "main = !-> return\n",
    ] {
        assert_eq!(compile_and_run_without_std(source), 0);
    }
    assert_eq!(compile_and_run_without_std("main = -> 7\n"), 7);
}

#[test]
fn test_main_exit_status_at_all_optimization_levels() {
    let cases = [
        (
            "std_effect",
            "main = !-> true.println!\n",
            true,
            true,
            0,
            "true\n",
        ),
        (
            "raw_effect",
            "extern putchar: I32 -> I32\nmain = !-> putchar (65 as I32)\n",
            false,
            true,
            0,
            "A",
        ),
        ("discard_integer", "main = !-> 42\n", false, true, 0, ""),
        ("explicit_unit", "main = -> ()\n", false, true, 0, ""),
        (
            "early_cleanup",
            r#"
struct Cleanup
    < value: I64

impl Drop for Cleanup
    ~@drop = !-> self.value.println!

main = !->
    cleanup = Cleanup
        value: 17
    true.println!
    return
"#,
            true,
            true,
            0,
            "true\n17\n",
        ),
        ("integer_i64", "main = -> 7\n", false, false, 7, ""),
        ("integer_i32", "main = -> 13 as I32\n", false, false, 13, ""),
    ];
    for opt_level in 0..=3 {
        for (name, source, with_std, unit_return, exit_code, stdout) in cases {
            let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
            let dir = test_temp_dir(id);
            fs::create_dir_all(&dir).unwrap();
            let _cleanup = TestDirCleanup(dir.clone());
            let entry = dir.join(format!("{name}.rk"));
            fs::write(&entry, source).unwrap();
            let mut config = test_config(entry, dir.clone());
            config.opt_level = opt_level;
            config.emit_llvm = true;
            if !with_std {
                config.extern_artifacts.clear();
                config.no_std = true;
                config.no_prelude = true;
            }
            rock_lib::compile(&config)
                .unwrap_or_else(|error| panic!("{name} at -O {opt_level}: {error:?}"));
            let output = run_test_command(&mut Command::new(dir.join(name)));
            assert_eq!(
                output.status.code(),
                Some(exit_code),
                "{name} at -O {opt_level}"
            );
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                stdout,
                "{name} at -O {opt_level}"
            );
            if unit_return {
                let ir = fs::read_to_string(dir.join(format!("{name}.ll"))).unwrap();
                assert!(
                    ir.contains("define internal void @__rock_main()"),
                    "{name} must retain its unit ABI"
                );
                assert!(
                    ir.contains("ret i32 0"),
                    "{name} must return zero from the C entry wrapper"
                );
            }
        }
    }
}

#[test]
fn test_impl_methods() {
    let output = compile_example("impl_methods");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "7"); // sum: 3+4
    assert_eq!(lines[1], "3"); // get_x: 3
    assert_eq!(lines[2], "13"); // add 10: 3+10
}

#[test]
fn test_string_concat() {
    let output = compile_and_run(
        r#"
main = ->
    a = "Hello, "
    b = "World!"
    c = string_concat a, b
    c.println!
    0
"#,
    );
    assert_eq!(output.trim(), "Hello, World!");
}

#[test]
fn test_string_add_operator_concats_owned_string_and_borrowed_str() {
    let output = compile_and_run(
        r#"
main = ->
    (String::from "Hello, ") + "World!" .println!
    "Hello, " + "World!" .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["Hello, World!"; 2]);
}

#[test]
fn test_struct_field_assignment() {
    let output = compile_and_run(
        r#"
struct Point
    < x: I64
    < y: I64

main = ->
    p = Point
        x: 10
        y: 20
    p.x.println!
    p.x = 42
    p.x.println!
    p.y.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10"); // original x
    assert_eq!(lines[1], "42"); // modified x
    assert_eq!(lines[2], "20"); // y unchanged
}

#[test]
fn test_type_conversion() {
    let output = compile_and_run(
        r#"
main = ->
    x = 42
    f = to_float x
    f.println!
    y = 3.7
    i = to_int y
    i.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "42"); // int to float (printed with %g)
    assert_eq!(lines[1], "3"); // float to int (truncates)
}

#[test]
fn test_boolean_printing() {
    let output = compile_and_run(
        r#"
main = ->
    true.println!
    false.println!
    3 > 2 .println!
    3 < 2 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "true");
    assert_eq!(lines[1], "false");
    assert_eq!(lines[2], "true");
    assert_eq!(lines[3], "false");
}

#[test]
fn test_string_len_method() {
    let output = compile_and_run(
        r#"
main = ->
    s = String::from "hello"
    s.len! .println!
    0
"#,
    );

    assert_eq!(output.trim(), "5");
}

#[test]
fn test_application_binds_before_infix() {
    let output = compile_and_run(
        r#"
double = x -> x * 2

main = ->
    complete_argument = double 2 + 2
    complete_argument.println!
    application_first = (double 2) + 2
    application_first.println!
    double 2 + 2 .println!
    Option::Some 2 <&> (+ 2) .unwrap_or 0 .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["8", "6", "8", "4"]);
}

#[test]
fn test_range_for_loop() {
    let output = compile_and_run(
        r#"
main = ->
    for i in 0..5
        i.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines.len(), 5);
    assert_eq!(lines[0], "0");
    assert_eq!(lines[4], "4");
}

#[test]
fn test_range_sum() {
    let output = compile_and_run(
        r#"
main = ->
    sum = 0
    for i in 1..101
        sum = sum + i
    sum.println!
    0
"#,
    );
    assert_eq!(output.trim(), "5050");
}

#[test]
fn test_range_with_variable() {
    let output = compile_and_run(
        r#"
main = ->
    n = 5
    total = 0
    for i in 0..n
        total = total + i
    total.println!
    0
"#,
    );
    assert_eq!(output.trim(), "10");
}

#[test]
fn test_range_nested() {
    let output = compile_and_run(
        r#"
main = ->
    count = 0
    for i in 0..3
        for j in 0..4
            count = count + 1
    count.println!
    0
"#,
    );
    assert_eq!(output.trim(), "12");
}

#[test]
fn test_comments() {
    let output = compile_and_run(
        r#"
// top level comment
// another top level comment
main = ->
    // comment in body
    x = 42
    // comment between statements
    y = x + 8
    // comment before result
    y.println!
    // trailing comment
    0
"#,
    );
    assert_eq!(output.trim(), "50");
}

use crate::support::*;
use std::fs;
use std::process::Command;
use std::sync::atomic::Ordering;

#[test]
fn test_std_sizeof_borrows_values_and_queries_pointees() {
    let source = r#"
> std::mem::sizeof
> std::mem::sizeof_pointee
> std::mem::Sized
> std::mem::Box
> std::mem::Drop
> std::show::Show

inspect: &T -> I64 where T: Sized
inspect = value -> sizeof value

slice_view: &[I64] -> &[I64]
slice_view = values -> values

struct Empty

struct Pair
    < small: U8
    < large: I64

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = !-> self.value.println!

main = !->
    (sizeof &(0 as U8)).println!
    (sizeof &(0 as U16)).println!
    (sizeof &(0 as U32)).println!
    (sizeof &(0 as I64)).println!
    (sizeof &true).println!
    (sizeof &'🦀').println!
    (sizeof &Empty).println!
    values = [1, 2, 3]
    (inspect &values).println!
    view = slice_view &values
    (sizeof &view).println!
    pair = Pair
        small: 1 as U8
        large: 2
    (sizeof &pair).println!
    null_pair = 0 as *Pair
    (sizeof_pointee null_pair).println!
    null_empty = 0 as *Empty
    (sizeof_pointee null_empty).println!
    null_integer = 0 as *I64
    (sizeof &null_integer).println!
    (sizeof_pointee null_integer).println!
    value = Tracked
        value: 73
    (inspect &value).println!
    value.value.println!
    boxed = Box::new value
    (sizeof boxed.as_ref!).println!
    "end".println!
"#;
    for opt_level in [0, 3] {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = test_temp_dir(id);
        fs::create_dir_all(&dir).unwrap();
        let _cleanup = TestDirCleanup(dir.clone());
        let entry = dir.join("sizeof.rk");
        fs::write(&entry, source).unwrap();
        let mut config = test_config(entry, dir.clone());
        config.opt_level = opt_level;
        config.no_prelude = true;
        rock_lib::compile(&config).unwrap();
        let output = run_test_command(&mut Command::new(dir.join("sizeof")));
        assert_eq!(
            output.status.code(),
            Some(0),
            "optimization level {opt_level}"
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "1\n2\n4\n8\n1\n4\n0\n24\n16\n16\n16\n0\n8\n8\n8\n73\n8\nend\n73\n",
            "optimization level {opt_level}",
        );
    }
}

#[test]
fn test_std_sizeof_rejects_unsized_pointees() {
    for source in [
        r#"
main = !->
    values = [1, 2, 3]
    slice = &values[..]
    sizeof slice
"#,
        r#"
main = !->
    sizeof "hello"
"#,
        r#"
main = !->
    values = [1, 2, 3]
    slice = &values[..]
    sizeof_pointee (slice as *[I64])
"#,
    ] {
        compile_should_fail(source, "Sized");
    }
}

#[test]
fn test_std_domain_facades_without_prelude() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("facades.rk");
    std::fs::write(
        &entry,
        r#"
> std::collections::Vec
> std::control::Option
> std::control::Result
> std::control::ControlFlow
> std::mem::Box
> std::mem::sizeof
> std::string::String
> std::functional::Functor
> std::functional::fmap
> std::cmp::Eq
> std::num::Add
> std::ops::Index
> std::ops::Deref
> std::concurrency::Mutex
> std::concurrency::thread::spawn
> std::io::Write
> std::io::fs::File
> std::io::net::Ipv4Addr
> std::ffi::libc::sqrt

first: &Vec I64 -> I64
first = values -> values[0]

main = ->
    mut values: Vec I64 = Vec::new!
    values.push 42
    boxed: Box I64 = Box::new (first &values)
    selected: Option I64 = fmap (value -> value), (Option::Some *boxed)
    match selected
        Option::Some value => value
        Option::None => 0
"#,
    )
    .unwrap();
    let mut config = test_config(entry, dir.clone());
    config.no_prelude = true;
    rock_lib::compile(&config).expect("domain facades should work without prelude imports");
    let output = Command::new(dir.join("facades")).output().unwrap();
    assert_eq!(output.status.code(), Some(42));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_std_domain_facades_with_prelude() {
    let output = compile_and_run(
        r#"
> std::collections::Vec
> std::control::Option
> std::mem::Box
> std::string::String

main = !->
    mut values: Vec I64 = Vec::new!
    values.push 42
    first: Option I64 = Option::Some values[0]
    first.println!
    boxed = Box::new 42
    (*boxed).println!
    text = String::from "hello"
    text.println!
"#,
    );
    assert_eq!(output, "Some(42)\n42\nhello\n");
}

#[test]
fn test_std_old_flat_module_paths_are_not_aliases() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("old_paths.rk");
    for (path, value) in [
        ("std::vec::Vec", "values: Vec I64 = Vec::new!"),
        ("std::option::Option", "value: Option I64 = Option::Some 42"),
        ("std::sync::Mutex", "value: Mutex I64 = Mutex::new 42"),
    ] {
        std::fs::write(&entry, format!("> {path}\nmain = !->\n    {value}\n")).unwrap();
        let mut config = test_config(entry.clone(), dir.clone());
        config.no_prelude = true;
        let diagnostics =
            rock_lib::compile(&config).expect_err("old module paths must not resolve");
        assert!(format!("{diagnostics:?}").contains(path));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_modules() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    // Write the module file
    std::fs::write(
        dir.join("utils.rk"),
        "\
add = a, b -> a + b

double = x -> x * 2

square = x -> x * x

< add
< double
< square
",
    )
    .unwrap();

    // Write the main file that uses the module
    std::fs::write(
        dir.join("test.rk"),
        "\
mod utils
> utils::add
> utils::double
> utils::square

main = ->
    add 3, 4 .println!
    double 10 .println!
    square 5 .println!
    0
",
    )
    .unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());

    rock_lib::compile(&config).expect("Compilation failed");

    let binary = dir.join("test");
    let output = Command::new(&binary)
        .output()
        .expect("Failed to run compiled binary");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        output.status.success(),
        "binary failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        stdout,
        stderr
    );
    assert!(
        !stdout.trim().is_empty(),
        "binary produced empty stdout with status {:?}\nstderr:\n{}",
        output.status.code(),
        stderr
    );

    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "7"); // add 3 4
    assert_eq!(lines[1], "20"); // double 10
    assert_eq!(lines[2], "25"); // square 5
}

#[test]
fn test_module_type_alias_import_export_and_qualification() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("types.rk"), "type Number = I64\n\n< Number\n").unwrap();
    std::fs::write(
        dir.join("test.rk"),
        "mod types\n> types::Number\n\nidentity: Number -> Number\nidentity = value -> value\n\nmain = ->\n    identity 42 .println!\n    0\n",
    )
    .unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());
    rock_lib::compile(&config).expect("Compilation failed");
    let output = Command::new(dir.join("test"))
        .output()
        .expect("Failed to run compiled binary");
    let _ = std::fs::remove_dir_all(&dir);

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");
}

#[test]
fn test_module_directory_mod_file_resolution() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(dir.join("utils")).unwrap();

    std::fs::write(
        dir.join("utils").join("mod.rk"),
        "\
answer = -> 42

< answer
",
    )
    .unwrap();

    std::fs::write(
        dir.join("test.rk"),
        "\
mod utils
> utils::answer

main = ->
    answer!.println!
    0
",
    )
    .unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());
    rock_lib::compile(&config).expect("Compilation failed");

    let binary = dir.join("test");
    let output = Command::new(&binary)
        .output()
        .expect("Failed to run compiled binary");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(stdout.trim(), "42");
}

#[test]
fn test_missing_module_reports_all_searched_paths() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(dir.join("test.rk"), "mod missing\nmain = -> 0\n").unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());
    let result = rock_lib::compile(&config);
    let diagnostics = result.expect_err("missing module should fail");
    let text = diagnostics
        .0
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let _ = std::fs::remove_dir_all(&dir);

    assert!(text.contains("missing.rk"), "diagnostics were {text}");
    assert!(text.contains("missing/mod.rk"), "diagnostics were {text}");
}

#[test]
fn test_unresolved_import_reports_diagnostic() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(dir.join("util.rk"), "answer = -> 42\n\n< answer\n").unwrap();
    std::fs::write(
        dir.join("test.rk"),
        "mod util\n> util::answr\n\nmain = -> 0\n",
    )
    .unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());
    let diagnostics =
        rock_lib::compile(&config).expect_err("an import of a missing item should fail");
    let text = diagnostics
        .0
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        text.contains("unresolved import 'util::answr'"),
        "diagnostics were {text}"
    );
}

#[test]
fn test_unresolved_import_in_source_module_reports_one_diagnostic() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());

    std::fs::write(dir.join("test.rk"), "mod util\nmain = -> 0\n").unwrap();
    std::fs::write(dir.join("util.rk"), "> missing::answer\n").unwrap();

    let mut config = test_config(dir.join("test.rk"), dir.clone());
    config.no_std = true;
    config.no_prelude = true;
    let diagnostics =
        rock_lib::compile(&config).expect_err("an import of a missing item should fail");

    assert_eq!(diagnostics.0.len(), 1, "diagnostics were {diagnostics:?}");
    assert_eq!(
        diagnostics.0[0].message,
        "unresolved import 'missing::answer'"
    );
}

#[test]
fn test_circular_module_load_reports_diagnostic() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(dir.join("test.rk"), "mod a\nmain = -> 0\n").unwrap();
    std::fs::write(dir.join("a.rk"), "mod b\n").unwrap();
    std::fs::write(dir.join("b.rk"), "mod a\n").unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());
    let result = rock_lib::compile(&config);
    let diagnostics = result.expect_err("cycle should fail");
    let text = diagnostics
        .0
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        text.contains("Circular module load"),
        "diagnostics were {text}"
    );
    assert!(text.contains("a.rk"), "diagnostics were {text}");
    assert!(text.contains("b.rk"), "diagnostics were {text}");
}

#[test]
fn test_module_glob_import_and_export() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(
        dir.join("math_impl.rk"),
        "\
add = a, b -> a + b

double = x -> x * 2

< add
< double
",
    )
    .unwrap();

    std::fs::write(
        dir.join("api.rk"),
        "\
mod math_impl

< math_impl::*
",
    )
    .unwrap();

    std::fs::write(
        dir.join("test.rk"),
        "\
mod api
> api::*

main = ->
    add 2, 5 .println!
    double 9 .println!
    0
",
    )
    .unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());

    rock_lib::compile(&config).expect("Compilation failed");

    let binary = dir.join("test");
    let output = Command::new(&binary)
        .output()
        .expect("Failed to run compiled binary");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let _ = std::fs::remove_dir_all(&dir);

    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "7");
    assert_eq!(lines[1], "18");
}

#[test]
fn test_id_owned_hir_preserves_same_name_module_calls() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(
        dir.join("left.rk"),
        "\
value = -> 11

< value
",
    )
    .unwrap();
    std::fs::write(
        dir.join("right.rk"),
        "\
value = -> 31

< value
",
    )
    .unwrap();
    std::fs::write(
        dir.join("test.rk"),
        "\
mod left
mod right

main = ->
    left::value! + right::value! .println!
    0
",
    )
    .unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());
    rock_lib::compile(&config).expect("Compilation failed");

    let binary = dir.join("test");
    let output = Command::new(&binary)
        .output()
        .expect("Failed to run compiled binary");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(stdout.trim(), "42");
}

#[test]
fn test_module_with_structs() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    // Write the math module with a struct and functions
    std::fs::write(
        dir.join("geometry.rk"),
        "\
struct Point
    < x: I64
    < y: I64

make_point = a, b ->
    Point
        x: a
        y: b

distance_sq = p ->
    p.x * p.x + p.y * p.y

< Point
< make_point
< distance_sq
",
    )
    .unwrap();

    // Write the main file
    std::fs::write(
        dir.join("test.rk"),
        "\
mod geometry
> geometry::Point
> geometry::make_point
> geometry::distance_sq

main = ->
    p = make_point 3, 4
    p.x.println!
    p.y.println!
    distance_sq p .println!
    0
",
    )
    .unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());

    rock_lib::compile(&config).expect("Compilation failed");

    let binary = dir.join("test");
    let output = Command::new(&binary)
        .output()
        .expect("Failed to run compiled binary");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let _ = std::fs::remove_dir_all(&dir);

    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "3"); // p.x
    assert_eq!(lines[1], "4"); // p.y
    assert_eq!(lines[2], "25"); // 3^2 + 4^2 = 25
}

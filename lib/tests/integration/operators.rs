use crate::support::*;
use std::process::Command;
use std::sync::atomic::Ordering;

#[test]
fn test_operator_declared_twice_is_rejected() {
    compile_should_fail(
        r#"
infix 9 |+|
|+| = a, b -> a + b
infix 9 |+|

main = !->
    (1 |+| 2).println!
"#,
        "Operator '|+|' is declared more than once",
    );
}

/// Writes `source` as crate `name` and publishes its artifact.
fn publish_operator_crate(
    dir: &std::path::Path,
    name: &str,
    source: &str,
    deps: Vec<(String, std::path::PathBuf)>,
) -> std::path::PathBuf {
    let entry = dir.join(format!("{name}.rk"));
    std::fs::write(&entry, source).unwrap();
    let artifact = dir.join(format!("{name}.rkca"));
    rock_lib::compile_with_products(&rock_lib::Config {
        entry_file: entry,
        output_dir: dir.to_path_buf(),
        current_crate_name: Some(name.to_string()),
        extern_artifacts: deps,
        no_std: true,
        no_prelude: true,
        no_link: true,
        ..Default::default()
    })
    .unwrap()
    .products
    .unwrap()
    .write_artifact_to_path(&artifact)
    .unwrap();
    artifact
}

fn compile_operator_root(
    dir: &std::path::Path,
    deps: Vec<(String, std::path::PathBuf)>,
) -> Result<(), String> {
    let entry = dir.join("root.rk");
    std::fs::write(&entry, "main = -> 0\n").unwrap();
    rock_lib::compile(&rock_lib::Config {
        entry_file: entry,
        output_dir: dir.to_path_buf(),
        extern_artifacts: deps,
        no_std: true,
        no_prelude: true,
        no_link: true,
        ..Default::default()
    })
    .map(|_| ())
    .map_err(|diagnostics| diagnostics.to_string())
}

#[test]
fn test_operator_declared_by_two_dependencies_is_rejected() {
    let dir = test_temp_dir(TEST_COUNTER.fetch_add(1, Ordering::SeqCst));
    std::fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    let left = publish_operator_crate(&dir, "left", "infix 9 |+|\n", Vec::new());
    let right = publish_operator_crate(&dir, "right", "infix 9 |+|\n", Vec::new());

    let error = compile_operator_root(
        &dir,
        vec![("left".to_string(), left), ("right".to_string(), right)],
    )
    .expect_err("two declarations of one operator must be rejected");

    assert!(
        error.contains("Operator '|+|' is declared more than once"),
        "{error}"
    );
}

#[test]
fn test_operator_inherited_through_two_dependencies_is_one_declaration() {
    let dir = test_temp_dir(TEST_COUNTER.fetch_add(1, Ordering::SeqCst));
    std::fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    let base = publish_operator_crate(&dir, "base", "infix 9 |+|\n", Vec::new());
    let middle = publish_operator_crate(
        &dir,
        "middle",
        "value = -> 1\n",
        vec![("base".to_string(), base.clone())],
    );

    compile_operator_root(
        &dir,
        vec![("base".to_string(), base), ("middle".to_string(), middle)],
    )
    .expect("an operator inherited through two paths is declared once");
}

#[test]
fn test_operator_declared_by_dependency_is_rejected() {
    compile_should_fail(
        r#"
infix 9 >>

main = !->
    1.println!
"#,
        "Operator '>>' is already declared by crate 'std'",
    );
}

#[test]
fn test_operator_resolution_does_not_depend_on_std_trait_names() {
    let output = compile_and_run(
        r#"< struct Boxed
    < value: I64

< trait Combine Rhs
    @+: Rhs -> I64

impl Combine Boxed for Boxed
    @+ = other -> self.value + other.value

main = ->
    lhs = Boxed
        value: 40
    rhs = Boxed
        value: 2
    result = lhs + rhs
    result.println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_primitive_operator_semantics_come_from_user_defined_operator() {
    let output = compile_and_run(
        r#"+ = left, right -> ~I64Sub left, right

main = ->
    40 + 2 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "38");
}

#[test]
fn test_primitive_operator_accepts_inferred_rhs_type() {
    let output = compile_and_run(
        r#"main = ->
    values = [2]
    result = 40 + values[0]
    result.println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_logical_operators() {
    let output = compile_and_run(
        r#"
main = ->
    a = 5
    b = 10
    if a > 3 && b < 20
        1.println!
    else
        0.println!
    if a > 10 || b > 5
        1.println!
    else
        0.println!
    if a > 10 && b > 100
        1.println!
    else
        0.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1"); // 5>3 && 10<20 = true
    assert_eq!(lines[1], "1"); // 5>10 || 10>5 = true
    assert_eq!(lines[2], "0"); // 5>10 && 10>100 = false
}

#[test]
fn test_logical_operators_short_circuit_rhs_bounds_checks() {
    let (output, success) = compile_and_run_with_status(
        r#"
main = ->
    arr = [1]
    if false && arr[1] == 0
        99.println!
    else
        1.println!
    if true || arr[1] == 0
        2.println!
    else
        99.println!
    0
"#,
    );

    assert!(
        success,
        "short-circuited RHS should not evaluate bounds checks"
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["1", "2"]);
}

#[test]
fn test_std_pipe_operator_function() {
    let output = compile_and_run(
        r#"
inc = x -> x + 1

main = ->
    41 |> inc .println!
    (((Option::Some 41) <&> inc).unwrap_or 0) .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["42", "42"]);
}

#[test]
fn test_module_local_pipe_operator_shadows_prelude() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(
        dir.join("ops.rk"),
        "\
|> = x, f -> 43

< |>
",
    )
    .unwrap();

    std::fs::write(
        dir.join("test.rk"),
        "\
mod ops
> ops::|>

inc = x -> x + 1

main = ->
    41 |> inc .println!
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

    assert_eq!(stdout.trim(), "43");
}

#[test]
fn test_root_pipe_operator_shadows_prelude_with_same_param_names() {
    let output = compile_and_run(
        r#"
|> = x, f -> 43
inc = x -> x + 1

main = ->
    41 |> inc .println!
    0
"#,
    );

    assert_eq!(output.trim(), "43");
}

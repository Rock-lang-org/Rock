use crate::support::*;
use std::sync::atomic::Ordering;

#[test]
fn test_generic_function_argument_in_monomorphized_method_call() {
    let output = compile_and_run(
        r#"
identity = x -> x

struct Box T
    value: T

impl Box T
    new = value ->
        Box
            value: value

    @apply = f -> f self.value

main = ->
    b = Box::new 21
    b.apply identity .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "21");
}

#[test]
fn test_generic_instance_call_edges_for_function_and_method() {
    let output = compile_and_run(
        r#"
identity = x -> x

struct Holder T
    value: T

impl Holder T
    new = value ->
        Holder
            value: value

    @apply = f -> f self.value

main = ->
    value = identity 42
    h = Holder::new value
    h.apply identity .println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_generic_method_call_preserves_receiver_specialization() {
    let output = compile_and_run(
        r#"
struct Box T
    value: T

impl Box T
    new = value ->
        Box
            value: value

    @get = -> @value

main = ->
    ib = Box::new 41
    bb = Box::new true
    if bb.get!
        ib.get!.println!
    else
        0.println!
    0
"#,
    );

    assert_eq!(output.trim(), "41");
}

fn compile_to_llvm_ir(source: &str) -> String {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();
    let source_path = dir.join(format!("test_{}.rk", id));
    std::fs::write(&source_path, source).unwrap();
    let mut config = test_config(source_path.clone(), dir.clone());
    config.no_link = true;
    config.emit_llvm = true;
    config.emit_object = Some(dir.join(format!("test_{}.o", id)));

    rock_lib::compile(&config).expect("Compilation failed");

    let module_name = source_path.file_stem().unwrap().to_string_lossy();
    let llvm = std::fs::read_to_string(dir.join(format!("{}.ll", module_name)))
        .expect("Expected LLVM IR output");
    let _ = std::fs::remove_dir_all(&dir);
    llvm
}

fn symbol_fragment_bytes(fragment: &str) -> String {
    let mut mangled = String::new();
    for byte in fragment.as_bytes() {
        mangled.push_str(&format!("{byte:02x}"));
    }
    mangled
}

fn defined_symbol_mentions_fragment(line: &str, fragment: &str) -> bool {
    line.contains(fragment) || line.contains(&symbol_fragment_bytes(fragment))
}

#[test]
fn test_instance_dce_does_not_emit_unused_current_function() {
    let llvm = compile_to_llvm_ir(
        r#"
used = -> 1
unused = -> 2

main = ->
    used! .println!
    0
"#,
    );

    let defined_symbols: Vec<&str> = llvm
        .lines()
        .filter(|line| line.starts_with("define "))
        .collect();
    let used_symbol = "__rock_fn_d0_";
    let unused_symbol = "__rock_fn_d1_";
    assert!(
        defined_symbols
            .iter()
            .any(|line| defined_symbol_mentions_fragment(line, used_symbol)),
        "expected used function definition in LLVM IR: {defined_symbols:?}"
    );
    assert!(
        !defined_symbols
            .iter()
            .any(|line| defined_symbol_mentions_fragment(line, unused_symbol)),
        "unused function should not be defined in LLVM IR: {defined_symbols:?}"
    );
}

#[test]
fn test_instance_dce_does_not_emit_unused_generic_specialization() {
    let llvm = compile_to_llvm_ir(
        r#"
identity = x -> x

use_i64 = -> identity 7
use_i32 = -> identity (8 as I32)

main = ->
    use_i64! .println!
    0
"#,
    );

    let defined_symbols: Vec<&str> = llvm
        .lines()
        .filter(|line| line.starts_with("define "))
        .collect();
    let identity_specializations: Vec<&str> = defined_symbols
        .iter()
        .copied()
        .filter(|line| defined_symbol_mentions_fragment(line, "__rock_fn_d0_"))
        .collect();
    assert_eq!(
        identity_specializations.len(),
        1,
        "only the reachable identity specialization should be defined: {defined_symbols:?}"
    );
    let unused_caller_symbol = "__rock_fn_d2_";
    assert!(
        !defined_symbols
            .iter()
            .any(|line| defined_symbol_mentions_fragment(line, unused_caller_symbol)),
        "unused caller should not be defined in LLVM IR: {defined_symbols:?}"
    );
}

#[test]
fn test_generic_function_argument_app_ir_does_not_define_std_object_symbols() {
    let llvm = compile_to_llvm_ir(
        r#"
identity = x -> x

struct Box T
    value: T

impl Box T
    new = value ->
        Box
            value: value

    @apply = f -> f self.value

main = ->
    b = Box::new 21
    b.apply identity .println!
    0
"#,
    );
    let offending_defs: Vec<&str> = llvm
        .lines()
        .filter(|line| line.starts_with("define ") && line.contains("@std__String_none_new"))
        .collect();
    assert!(
        offending_defs.is_empty(),
        "app IR unexpectedly defines std object-backed symbols: {:?}",
        offending_defs
    );
}

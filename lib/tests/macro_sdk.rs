use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use rock_lib::{Config, SourceProvider};

fn run_client(path: &Path) -> Output {
    let mut child = Command::new(path).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!("SDK client exceeded 15s: {output:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn build_library(entry: PathBuf, name: &str, output: &Path, dependencies: Vec<(String, PathBuf)>) -> PathBuf {
    std::fs::create_dir_all(output).unwrap();
    let result = rock_lib::compile_with_products(&Config {
        entry_file: entry, output_dir: output.into(), current_crate_name: Some(name.into()),
        extern_artifacts: dependencies, no_std: true, no_prelude: name != "std", no_proc_execution: true,
        no_link: true, emit_object: Some(output.join(format!("{name}.o"))), ..Default::default()
    }).unwrap_or_else(|diagnostics| panic!("{name}: {diagnostics:?}"));
    let products = result.products.unwrap();
    assert!(products.proc_macros.is_empty(), "the SDK is an ordinary library");
    let artifact = output.join(format!("{name}.rkca"));
    products.write_artifact_to_path(&artifact).unwrap();
    artifact
}

#[test]
fn macro_b_06_plain_rock_sdk_data_and_operations_run_with_only_explicit_artifact_dependencies() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let root = std::env::temp_dir().join(format!("rock-macro-sdk-data-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let std = build_library(workspace.join("std/src/lib.rk"), "std", &root.join("std"), Vec::new());
    eprintln!("SDK acceptance: std artifact built");
    let sdk = build_library(workspace.join("rock_macro/src/lib.rk"), "rock_macro", &root.join("sdk"), vec![("std".into(), std.clone())]);
    eprintln!("SDK acceptance: SDK artifact built");
    let entry = root.join("main.rk");
    let config = Config {
        entry_file: entry.clone(), output_dir: root.join("app"), no_std: true, no_proc_execution: true,
        current_crate_name: Some("sdk_client".into()),
        extern_artifacts: vec![("std".into(), std.clone()), ("rock_macro".into(), sdk)],
        source_providers: vec![SourceProvider::Virtual { path: entry,
            text: include_str!("fixtures/macros/sdk-data.rk").into() }], ..Default::default()
    };
    rock_lib::compile(&config).unwrap_or_else(|diagnostics| panic!("SDK client: {diagnostics:?}"));
    let output = run_client(&root.join("app/main"));
    assert!(output.status.success(), "{:?}", output);
    assert_eq!(output.stdout, "0\n0\n42\n\n7\n1\n42\n雪{'é\nwarning alongside output\nstructured error\n3\n4\n5\n雪\n1\nhelper\n9\n".as_bytes());
    eprintln!("SDK acceptance: data client passed");
    let mut operations = config.clone();
    operations.source_providers = vec![SourceProvider::Virtual { path: operations.entry_file.clone(),
        text: include_str!("fixtures/macros/sdk-operations.rk").into() }];
    rock_lib::compile(&operations).unwrap_or_else(|diagnostics| panic!("SDK operations: {diagnostics:?}"));
    eprintln!("SDK acceptance: operations client compiled");
    let output = run_client(&root.join("app/main"));
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"copies/slices/joins\ncursor/opaque/DSL\nidentifiers/paths/groups\ndiagnostics\nSDK operations passed\n");
    let mut missing = config.clone();
    missing.extern_artifacts.retain(|(name, _)| name != "rock_macro");
    assert!(rock_lib::analyze(&missing).is_err(), "SDK is never injected or discovered by spelling");
    missing = config.clone();
    missing.extern_artifacts.retain(|(name, _)| name != "std");
    assert!(rock_lib::analyze(&missing).is_err(), "std is never discovered implicitly");
    std::fs::remove_dir_all(root).unwrap();
}

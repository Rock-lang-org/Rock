use std::path::{Path, PathBuf};

use rock_lib::{Config, SourceProvider};

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
fn macro_b_05_plain_rock_sdk_data_runs_with_only_explicit_artifact_dependencies() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let root = std::env::temp_dir().join(format!("rock-macro-sdk-data-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let std = build_library(workspace.join("std/src/lib.rk"), "std", &root.join("std"), Vec::new());
    let sdk = build_library(workspace.join("rock_macro/src/lib.rk"), "rock_macro", &root.join("sdk"), vec![("std".into(), std.clone())]);
    let entry = root.join("main.rk");
    let config = Config {
        entry_file: entry.clone(), output_dir: root.join("app"), no_std: true, no_proc_execution: true,
        current_crate_name: Some("sdk_client".into()),
        extern_artifacts: vec![("std".into(), std.clone()), ("rock_macro".into(), sdk)],
        source_providers: vec![SourceProvider::Virtual { path: entry,
            text: include_str!("fixtures/macros/sdk-data.rk").into() }], ..Default::default()
    };
    rock_lib::compile(&config).unwrap_or_else(|diagnostics| panic!("SDK client: {diagnostics:?}"));
    let output = std::process::Command::new(root.join("app/main")).output().unwrap();
    assert!(output.status.success(), "{:?}", output);
    assert_eq!(output.stdout, "0\n0\n42\n\n7\n1\n42\n雪{'é\nwarning alongside output\nstructured error\n3\n4\n5\n雪\n1\nhelper\n9\n".as_bytes());
    let mut missing = config.clone();
    missing.extern_artifacts.retain(|(name, _)| name != "rock_macro");
    assert!(rock_lib::analyze(&missing).is_err(), "SDK is never injected or discovered by spelling");
    missing = config.clone();
    missing.extern_artifacts.retain(|(name, _)| name != "std");
    assert!(rock_lib::analyze(&missing).is_err(), "std is never discovered implicitly");
    std::fs::remove_dir_all(root).unwrap();
}

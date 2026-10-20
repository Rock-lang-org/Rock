use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use rock_lib::{Config, SourceProvider};

#[path = "support/macro_requests.rs"]
mod macro_requests;
#[path = "support/macro_results.rs"]
mod macro_results;
#[path = "support/macro_workers.rs"]
mod macro_workers;
#[path = "support/macro_hosts.rs"]
mod macro_hosts;

fn run_client(path: &Path) -> Output {
    run_client_with_input(path, &[])
}

fn run_client_with_input(path: &Path, input: &[u8]) -> Output {
    assert!(input.len() <= 1048577, "test input is bounded independently of pipe capacity");
    let mut child = Command::new(path).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let stdout = child.stdout.take().unwrap();
    let stdout = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.take(1048577).read_to_end(&mut bytes).unwrap();
        bytes
    });
    let stderr = child.stderr.take().unwrap();
    let stderr = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.take(65536).read_to_end(&mut bytes).unwrap();
        bytes
    });
    // Writing runs concurrently with the deadline, including rejection before
    // EOF. The IO controls retain their two-byte fragments; request cases may
    // exceed pipe capacity. Output drains concurrently into capped buffers too.
    let mut stdin = child.stdin.take().unwrap();
    let input = input.to_vec();
    let writer = std::thread::spawn(move || {
        let chunk_size = if input.len() <= 33 { 2 } else { 257 };
        for chunk in input.chunks(chunk_size) {
            if let Err(error) = stdin.write_all(chunk) {
                assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe, "{error}");
                break;
            }
        }
    });
    loop {
        if child.try_wait().unwrap().is_some() {
            writer.join().unwrap();
            let mut output = child.wait_with_output().unwrap();
            output.stdout = stdout.join().unwrap();
            output.stderr = stderr.join().unwrap();
            return output;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let mut output = child.wait_with_output().unwrap();
            writer.join().unwrap();
            output.stdout = stdout.join().unwrap();
            output.stderr = stderr.join().unwrap();
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
fn macro_b_13_plain_rock_sdk_and_generated_host_use_only_explicit_artifact_dependencies() {
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
    let mut requests = config.clone();
    requests.source_providers = vec![SourceProvider::Virtual { path: requests.entry_file.clone(),
        text: include_str!("fixtures/macros/sdk-request.rk").into() }];
    rock_lib::compile(&requests).unwrap_or_else(|diagnostics| panic!("SDK request decoder: {diagnostics:?}"));
    let cases = macro_requests::cases();
    for (name, input, accepted) in &cases {
        let output = run_client_with_input(&root.join("app/main"), input);
        assert_eq!(output.status.code(), Some(if *accepted { 0 } else { 2 }), "{name}: {output:?}");
        assert_eq!(output.stdout, if *accepted { b"request decoded\n".as_slice() } else { b"" }, "{name}");
        assert!(output.stderr.is_empty(), "{name}: {output:?}");
    }
    eprintln!("SDK acceptance: {} Rust/Rock request controls passed", cases.len());
    let mut results = config.clone();
    results.source_providers = vec![SourceProvider::Virtual { path: results.entry_file.clone(),
        text: include_str!("fixtures/macros/sdk-result.rk").into() }];
    rock_lib::compile(&results).unwrap_or_else(|diagnostics| panic!("SDK result encoder: {diagnostics:?}"));
    let golden = rock_lib::macro_expansion::proc_macro::decode_request(&cases[0].1).unwrap();
    let rich = cases.iter().find(|(name, _, _)| name == "all provenance and integer widths").unwrap();
    let rich = rock_lib::macro_expansion::proc_macro::decode_request(&rich.1).unwrap();
    let results = macro_results::cases(&golden, &rich);
    for (request, accepted) in &results {
        let bytes = rock_lib::macro_expansion::proc_macro::encode_request(request).unwrap();
        let output = run_client_with_input(&root.join("app/main"), &bytes);
        assert_eq!(output.status.code(), Some(if *accepted { 0 } else { 2 }), "{}: {output:?}", request.macro_name);
        assert!(output.stderr.is_empty(), "{}: {output:?}", request.macro_name);
        if *accepted { macro_results::verify(request, &output.stdout); }
        else { assert!(output.stdout.is_empty(), "failed encoder emitted partial output"); }
    }
    eprintln!("SDK acceptance: {} Rock/Rust result controls passed", results.len());
    let mut worker = config.clone();
    worker.source_providers = vec![SourceProvider::Virtual { path: worker.entry_file.clone(),
        text: include_str!("fixtures/macros/sdk-worker.rk").into() }];
    rock_lib::compile(&worker).unwrap_or_else(|diagnostics| panic!("SDK worker: {diagnostics:?}"));
    macro_workers::verify(&root.join("app/main"), &golden, &rich);
    macro_hosts::verify(&config, &golden);
    macro_hosts::verify_literals(&config);
    let mut missing = config.clone();
    missing.extern_artifacts.retain(|(name, _)| name != "rock_macro");
    assert!(rock_lib::analyze(&missing).is_err(), "SDK is never injected or discovered by spelling");
    missing = config.clone();
    missing.extern_artifacts.retain(|(name, _)| name != "std");
    assert!(rock_lib::analyze(&missing).is_err(), "std is never discovered implicitly");
    let mut binary_io = config.clone();
    binary_io.extern_artifacts = vec![("std".into(), std)];
    binary_io.source_providers = vec![SourceProvider::Virtual { path: binary_io.entry_file.clone(),
        text: include_str!("fixtures/macros/sdk-io.rk").into() }];
    rock_lib::compile(&binary_io).unwrap_or_else(|diagnostics| panic!("binary IO: {diagnostics:?}"));
    let mut binary = "A\0雪é🦀\r\n".as_bytes().to_vec();
    binary.push(255); // Binary transport must not round-trip through String.
    for input in [Vec::new(), binary, (0..32).collect::<Vec<u8>>()] {
        let output = run_client_with_input(&root.join("app/main"), &input);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, input);
        assert!(output.stderr.is_empty(), "{output:?}");
    }
    let output = run_client_with_input(&root.join("app/main"), &[0; 33]);
    assert_eq!(output.status.code(), Some(2), "oversized input: {output:?}");
    assert!(output.stdout.is_empty(), "no partial output on a failed bounded read");
    assert!(output.stderr.is_empty(), "{output:?}");
    std::fs::remove_dir_all(root).unwrap();
}

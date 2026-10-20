//! Opt-in native acceptance groups. Ordinary macro regressions remain in the
//! compiler unit tests, integration.rs, macro_diagnostics and macro_process.

use rock_lib::SourceProvider;

#[path = "support/macro_requests.rs"]
mod macro_requests;
#[path = "support/macro_results.rs"]
mod macro_results;
#[path = "support/macro_workers.rs"]
mod macro_workers;
#[path = "support/macro_hosts.rs"]
mod macro_hosts;
#[path = "support/macro_blocks.rs"]
mod macro_blocks;
#[path = "support/macro_interpolation.rs"]
mod macro_interpolation;
#[path = "support/macro_fixture.rs"]
mod macro_fixture;
#[path = "support/macro_client.rs"]
mod macro_client;

use macro_client::{run_batch, run_client, run_client_with_input};
use macro_fixture::{build_library, SdkCase};

fn golden_requests() -> (rock_lib::macro_expansion::proc_macro::ProcMacroRequest, rock_lib::macro_expansion::proc_macro::ProcMacroRequest) {
    let cases = macro_requests::cases();
    let decode = rock_lib::macro_expansion::proc_macro::decode_request;
    (decode(&cases[0].1).unwrap(), decode(&cases.iter().find(|(name, _, _)| name == "all provenance and integer widths").unwrap().1).unwrap())
}

#[test]
fn macro_sdk_one_shot_smoke() {
    use rock_lib::macro_expansion::proc_macro::*;

    let case = SdkCase::new("smoke", include_str!("fixtures/macros/sdk-worker.rk"));
    rock_lib::compile(&case.config).unwrap_or_else(|errors| panic!("SDK worker smoke: {errors:?}"));
    let (request, _) = golden_requests();
    let artifact = ProcMacroArtifact::for_executable("smoke".into(), case.config.output_dir.join("main"),
        vec![ProcMacroExport { name: request.macro_name.clone(), identity: request.macro_identity.clone(),
            kind: request.kind.clone(), input_shape: ProcMacroInputShape::TokenStream }]).unwrap();
    for _ in 0..2 {
        let response = run_proc_macro_process(&artifact, &request, std::time::Duration::from_secs(15), false, None).unwrap();
        assert_eq!(response, ProcMacroResponse::expanded(request.input.clone()));
    }
}

#[test]
#[ignore = "native SDK acceptance; run cargo macro-acceptance"]
fn macro_acceptance_sdk_data_and_operations() {
    let case = SdkCase::new("data", include_str!("fixtures/macros/sdk-data.rk"));
    rock_lib::compile(&case.config).unwrap_or_else(|errors| panic!("SDK data: {errors:?}"));
    let output = run_client(&case.config.output_dir.join("main"));
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, "0\n0\n42\n\n7\n1\n42\n雪{'é\nwarning alongside output\nstructured error\n3\n4\n5\n雪\n1\nhelper\n9\n".as_bytes());
    let mut config = case.config.clone();
    config.source_providers = vec![SourceProvider::Virtual { path: config.entry_file.clone(), text: include_str!("fixtures/macros/sdk-operations.rk").into() }];
    rock_lib::compile(&config).unwrap_or_else(|errors| panic!("SDK operations: {errors:?}"));
    let output = run_client(&config.output_dir.join("main"));
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"copies/slices/joins\ncursor/opaque/DSL\nidentifiers/paths/groups\ndiagnostics\nSDK operations passed\n");
}

fn batch_case(group: &str, source: &str) -> SdkCase {
    let mut case = SdkCase::new(group, source);
    case.config.source_providers.push(SourceProvider::Virtual {
        path: case.config.entry_file.with_file_name("sdk_batch.rk"), text: include_str!("fixtures/macros/sdk_batch.rk").into(),
    });
    rock_lib::compile(&case.config).unwrap_or_else(|errors| panic!("SDK {group}: {errors:?}"));
    case
}

#[test]
#[ignore = "cross-language codec acceptance; run cargo macro-acceptance"]
fn macro_acceptance_sdk_request_codec() {
    let case = batch_case("requests", include_str!("fixtures/macros/sdk-request.rk"));
    let cases = macro_requests::cases();
    let input = cases.iter().map(|(_, input, _)| input.clone()).collect::<Vec<_>>();
    for ((name, _, accepted), (status, bytes)) in cases.iter().zip(run_batch(&case.config.output_dir.join("main"), &input)) {
        assert_eq!(status, if *accepted { 0 } else { 2 }, "{name}");
        assert_eq!(bytes, if *accepted { b"request decoded\n".as_slice() } else { b"" }, "{name}");
    }
    eprintln!("SDK acceptance: {} Rust/Rock request controls in one codec process", cases.len());
}

#[test]
#[ignore = "cross-language codec acceptance; run cargo macro-acceptance"]
fn macro_acceptance_sdk_result_codec() {
    let case = batch_case("results", include_str!("fixtures/macros/sdk-result.rk"));
    let (golden, rich) = golden_requests();
    let cases = macro_results::cases(&golden, &rich);
    let input = cases.iter().map(|(request, _)| rock_lib::macro_expansion::proc_macro::encode_request(request).unwrap()).collect::<Vec<_>>();
    for ((request, accepted), (status, bytes)) in cases.iter().zip(run_batch(&case.config.output_dir.join("main"), &input)) {
        assert_eq!(status, if *accepted { 0 } else { 2 }, "{}", request.macro_name);
        if *accepted { macro_results::verify(request, &bytes); }
        else { assert!(bytes.is_empty(), "failed encoder emitted partial output"); }
    }
    eprintln!("SDK acceptance: {} Rock/Rust result controls in one codec process", cases.len());
}

#[test]
#[ignore = "native one-shot lifecycle acceptance; run cargo macro-acceptance"]
fn macro_acceptance_sdk_worker_lifecycle() {
    let case = SdkCase::new("worker", include_str!("fixtures/macros/sdk-worker.rk"));
    rock_lib::compile(&case.config).unwrap_or_else(|errors| panic!("SDK worker: {errors:?}"));
    let (golden, rich) = golden_requests();
    macro_workers::verify(&case.config.output_dir.join("main"), &golden, &rich);
}

#[test]
#[ignore = "native host/publication acceptance; run cargo macro-acceptance"]
fn macro_acceptance_sdk_host_publication() {
    let case = SdkCase::new("host", include_str!("fixtures/macros/sdk-data.rk"));
    let (golden, _) = golden_requests();
    macro_hosts::verify(&case.config, &golden);
    for missing in ["std", "rock_macro"] {
        let mut config = case.config.clone();
        config.extern_artifacts.retain(|(name, _)| name != missing);
        assert!(rock_lib::analyze(&config).is_err(), "{missing} must never be injected implicitly");
    }
}

#[test]
#[ignore = "native literal/fragment acceptance; run cargo macro-acceptance"]
fn macro_acceptance_sdk_literals_and_fragments() {
    macro_hosts::verify_literals(&SdkCase::new("literals", "").config);
}

#[test]
#[ignore = "native DSL acceptance; run cargo macro-acceptance"]
fn macro_acceptance_sdk_block_dsl() {
    macro_blocks::verify(&SdkCase::new("blocks", "").config);
}

#[test]
#[ignore = "native interpolation/ownership acceptance; run cargo macro-acceptance"]
fn macro_acceptance_sdk_interpolation_ownership() {
    macro_interpolation::verify(&SdkCase::new("interpolation", "").config);
}

#[test]
#[ignore = "native binary IO acceptance; run cargo macro-acceptance"]
fn macro_acceptance_sdk_binary_io() {
    let case = SdkCase::new("io", include_str!("fixtures/macros/sdk-io.rk"));
    let mut config = case.config.clone(); config.extern_artifacts.retain(|(name, _)| name == "std");
    rock_lib::compile(&config).unwrap_or_else(|errors| panic!("binary IO: {errors:?}"));
    let mut binary = "A\0雪é🦀\r\n".as_bytes().to_vec(); binary.push(255);
    for input in [Vec::new(), binary, (0..32).collect::<Vec<u8>>()] {
        let output = run_client_with_input(&config.output_dir.join("main"), &input);
        assert!(output.status.success() && output.stderr.is_empty(), "{output:?}"); assert_eq!(output.stdout, input);
    }
    let output = run_client_with_input(&config.output_dir.join("main"), &[0; 33]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
}

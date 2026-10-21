use std::io::Write;
use std::process::{Command, Stdio};

use rock_lib::crate_system::TargetRole;
use rock_lib::macro_expansion::host::ProcMacroSdk;
use rock_lib::macro_expansion::proc_macro::*;
use rock_lib::{Config, SourceProvider};

pub fn verify(config: &Config) {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut host = config.clone();
    host.current_crate_name = Some("derive_dispatch".into());
    host.target_role = TargetRole::ProcMacro;
    host.output_dir = config.output_dir.parent().unwrap().join("host");
    host.source_providers = vec![SourceProvider::Virtual { path: host.entry_file.clone(),
        text: std::fs::read_to_string(workspace.join("lib/tests/fixtures/macros/sdk-derive-dispatch.rk")).unwrap() }];
    let built = rock_lib::compile_proc_macro_with_products(&host, &ProcMacroSdk { dependency: "rock_macro".into() })
        .unwrap_or_else(|errors| panic!("derive host: {:?}", errors.0.iter().map(|error| (&error.message, &error.location)).collect::<Vec<_>>()));
    let products = built.products.unwrap();
    let descriptor = products.proc_macros[0].clone();
    assert_eq!(descriptor.exports.iter().filter(|entry| entry.kind == ProcMacroKind::DeriveLike).count(), 2);
    let artifact = host.output_dir.join("derive_dispatch.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let descriptor = descriptor.resolve_for_host(&artifact).unwrap();
    let mut consumer = config.clone();
    consumer.entry_file = config.output_dir.parent().unwrap().join("consumer.rk");
    consumer.output_dir = config.output_dir.parent().unwrap().join("consumer");
    consumer.current_crate_name = Some("derive_consumer".into());
    consumer.no_proc_execution = false;
    consumer.extern_artifacts.retain(|(name, _)| name == "std");
    consumer.extern_artifacts.push(("derive_dispatch".into(), artifact));
    let source = "> derive_dispatch::Stamp\n> derive_dispatch::Empty\n> derive_dispatch::pick\n%derive Stamp, Empty\nstruct S\nmain = !->\n    answer!.println!\n    (%pick!).println!\n";
    consumer.source_providers = vec![SourceProvider::Virtual { path: consumer.entry_file.clone(), text: source.into() }];
    rock_lib::compile(&consumer).unwrap_or_else(|errors| panic!("derive consumer: {:?}", errors.0.iter().map(|error| (&error.message, &error.location)).collect::<Vec<_>>()));
    let output = Command::new(consumer.output_dir.join("consumer")).output().unwrap();
    assert!(output.status.success() && output.stderr.is_empty(), "{output:?}");
    assert_eq!(output.stdout, b"42\n7\n");

    // Bypass only the compiler's export-kind preflight in this test, so the
    // generated host's own identity/kind checks are independently exercised.
    for (name, kind) in [("Stamp", ProcMacroKind::FunctionLike), ("pick", ProcMacroKind::DeriveLike), ("Empty", ProcMacroKind::AttributeLike)] {
        let export = descriptor.exports.iter().find(|entry| entry.name == name).unwrap();
        let request = ProcMacroRequest {
            protocol_version: PROC_MACRO_PROTOCOL_VERSION, compiler_version: env!("CARGO_PKG_VERSION").into(),
            macro_name: name.into(), macro_identity: export.identity.clone(), source_module: None, expansion_id: None,
            invocation_id: rock_lib::ast::InvocationId::fresh().unwrap(), parent: None,
            output_category: rock_lib::ast::MacroOutputCategory::Items, kind, header_indent: 0, block_base: None,
            input: encode_stream(&rock_lib::macro_expansion::TokenStream::from_bytes("<derive-smoke>".into(), Vec::new(),
                rock_lib::macro_expansion::ScanLimits { max_source_bytes: 1048576, max_runs: 65536 }).unwrap()).unwrap(),
            definition_dependencies: descriptor.definition_dependencies.clone(), views: Default::default(),
        };
        let mut child = Command::new(&descriptor.executable).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(&encode_request(&request).unwrap()).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success() && output.stderr.is_empty(), "{output:?}");
        let response = decode_response(&output.stdout).unwrap();
        assert!(response.output.is_none());
        assert!(response.diagnostics.iter().any(|diagnostic| diagnostic.message.contains("identity or kind")));
    }
}

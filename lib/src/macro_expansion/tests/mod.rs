use super::*;
use crate::{language_items::LanguageItemRole, parser::parse_string, Config};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

mod diagnostics;
mod matching;
mod proc_macros;
mod repetition;

fn proc_macro_artifact_with_response(
    macro_name: &str,
    response: proc_macro::ProcMacroResponse,
) -> (proc_macro::ProcMacroArtifact, PathBuf) {
    let response = proc_macro::encode_response(&response).unwrap();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!(
        "rock-proc-macro-test-{}-{}",
        std::process::id(),
        nonce
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let host = temp_dir.join("proc_macro_host");
    fs::write(host.with_extension("response"), response).unwrap();
    fs::write(&host, "#!/bin/sh\ncat >/dev/null\ncat \"$0.response\"\n").unwrap();
    let mut permissions = fs::metadata(&host).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&host, permissions).unwrap();

    let artifact = proc_macro::ProcMacroArtifact {
        artifact_format_version: crate::products::PRODUCT_ARTIFACT_FORMAT_VERSION,
        crate_identity: "test_macros".to_string(),
        protocol_version: proc_macro::PROC_MACRO_PROTOCOL_VERSION,
        host_triple: "x86_64-unknown-linux-gnu".to_string(),
        executable: host,
        capabilities: vec![proc_macro::ProcMacroCapability::Stdio],
        exports: vec![proc_macro::ProcMacroExport {
            name: macro_name.to_string(),
            identity: format!("test_macros::{macro_name}"),
            kind: proc_macro::ProcMacroKind::FunctionLike,
            input_shape: proc_macro::ProcMacroInputShape::TokenStream,
        }],
    };

    (artifact, temp_dir)
}

fn test_config() -> Config {
    Config {
        entry_file: PathBuf::from("/test.rk"),
        output_dir: PathBuf::new(),
        debug_print: Vec::new(),
        meta_files: Vec::new(),
        extern_artifacts: Vec::new(),
        source_providers: Vec::new(),
        current_crate_name: None,
        opt_level: 0,
        overflow_checks: None,
        emit_llvm: false,
        no_link: false,
        emit_object: None,
        no_prelude: false,
        no_std: false,
        sysroot: None,
    }
}

fn assert_program_semantically_eq(actual: &Program, expected: &Program) {
    assert_eq!(
        crate::fmt::format(crate::fmt::FormatInput::program(actual)),
        crate::fmt::format(crate::fmt::FormatInput::program(expected)),
    );
}

fn assert_expands_to(input: &str, expected: &str) {
    let config = test_config();
    let context = MacroExpansionContext::new(&config);

    let expanded =
        expand_macros_with_context(parse_string(input, &config).unwrap(), &context).unwrap();
    let expected = parse_string(expected, &config).unwrap();

    assert_program_semantically_eq(&expanded, &expected);
}

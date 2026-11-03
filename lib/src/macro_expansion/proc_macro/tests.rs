use super::*;

const OVERSIZED_PAYLOAD_BYTES: usize = MAX_PROC_MACRO_MESSAGE_BYTES as usize + 1;

#[test]
fn proc_macro_protocol_version_tracks_enriched_request_shape() {
    assert_eq!(PROC_MACRO_PROTOCOL_VERSION, 2);
}

#[test]
fn proc_macro_protocol_roundtrips_request_and_response() {
    let request = ProcMacroRequest {
        protocol_version: PROC_MACRO_PROTOCOL_VERSION,
        compiler_version: env!("CARGO_PKG_VERSION").to_string(),
        macro_name: "make_main".to_string(),
        macro_identity: "macros::make_main".to_string(),
        source_module: None,
        expansion_id: None,
        input: Vec::new(),
    };
    let encoded = encode_request(&request).unwrap();
    let decoded = decode_request(&encoded).unwrap();

    assert_eq!(decoded.macro_name, "make_main");

    let response = ProcMacroResponse::Expand { output: Vec::new() };
    let encoded = encode_response(&response).unwrap();
    let decoded = decode_response(&encoded).unwrap();

    assert!(matches!(decoded, ProcMacroResponse::Expand { .. }));
}

#[test]
fn proc_macro_protocol_preserves_identity_context_and_capabilities() {
    let request = ProcMacroRequest {
        protocol_version: PROC_MACRO_PROTOCOL_VERSION,
        compiler_version: env!("CARGO_PKG_VERSION").to_string(),
        macro_name: "make_main".to_string(),
        macro_identity: "macros::make_main".to_string(),
        source_module: Some("main".to_string()),
        expansion_id: Some(7),
        input: vec![1, 2, 3],
    };
    let decoded = decode_request(&encode_request(&request).unwrap()).unwrap();
    assert_eq!(decoded.macro_identity, "macros::make_main");
    assert_eq!(decoded.source_module.as_deref(), Some("main"));
    assert_eq!(decoded.expansion_id, Some(7));

    let artifact = ProcMacroArtifact {
        artifact_format_version: crate::products::PRODUCT_ARTIFACT_FORMAT_VERSION,
        crate_identity: "macros".to_string(),
        protocol_version: PROC_MACRO_PROTOCOL_VERSION,
        host_triple: "x86_64-unknown-linux-gnu".to_string(),
        executable: "macro-host".into(),
        capabilities: vec![ProcMacroCapability::Stdio],
        exports: vec![ProcMacroExport {
            name: "make_main".to_string(),
            identity: "macros::make_main".to_string(),
            kind: ProcMacroKind::FunctionLike,
            input_shape: ProcMacroInputShape::TokenStream,
        }],
    };
    assert_eq!(
        artifact.exports[0].input_shape,
        ProcMacroInputShape::TokenStream
    );
    assert_eq!(artifact.capabilities, vec![ProcMacroCapability::Stdio]);
}

#[test]
fn proc_macro_protocol_roundtrips_token_payload() {
    let tokens = vec![crate::lexer::Token::from(crate::lexer::TokenType::Ident(
        "main".to_string(),
    ))];

    let encoded = encode_tokens(&tokens).unwrap();
    let decoded = decode_tokens(&encoded).unwrap();

    assert_eq!(decoded, tokens);
}

#[test]
fn proc_macro_artifact_records_host_executable_and_exports() {
    let artifact = ProcMacroArtifact {
        artifact_format_version: crate::products::PRODUCT_ARTIFACT_FORMAT_VERSION,
        crate_identity: "macros".to_string(),
        protocol_version: PROC_MACRO_PROTOCOL_VERSION,
        host_triple: "x86_64-unknown-linux-gnu".to_string(),
        executable: "target/proc-macros/make_main".into(),
        capabilities: vec![ProcMacroCapability::Stdio],
        exports: vec![ProcMacroExport {
            name: "make_main".to_string(),
            identity: "macros::make_main".to_string(),
            kind: ProcMacroKind::FunctionLike,
            input_shape: ProcMacroInputShape::TokenStream,
        }],
    };

    assert_eq!(artifact.exports[0].kind, ProcMacroKind::FunctionLike);
}

#[test]
fn decode_request_rejects_oversized_payload() {
    let request = ProcMacroRequest {
        protocol_version: PROC_MACRO_PROTOCOL_VERSION,
        compiler_version: env!("CARGO_PKG_VERSION").to_string(),
        macro_name: "make_main".to_string(),
        macro_identity: "macros::make_main".to_string(),
        source_module: None,
        expansion_id: None,
        input: vec![0; OVERSIZED_PAYLOAD_BYTES],
    };
    let encoded = bincode::serialize(&request).unwrap();

    assert!(decode_request(&encoded).is_err());
}

#[test]
fn decode_response_rejects_oversized_payload() {
    let response = ProcMacroResponse::Expand {
        output: vec![0; OVERSIZED_PAYLOAD_BYTES],
    };
    let encoded = bincode::serialize(&response).unwrap();

    assert!(decode_response(&encoded).is_err());
}

#[test]
fn proc_macro_runner_rejects_protocol_version_mismatch() {
    let artifact = ProcMacroArtifact {
        artifact_format_version: crate::products::PRODUCT_ARTIFACT_FORMAT_VERSION,
        crate_identity: "macros".to_string(),
        protocol_version: PROC_MACRO_PROTOCOL_VERSION,
        host_triple: "x86_64-unknown-linux-gnu".to_string(),
        executable: "/definitely/not/a/proc/macro".into(),
        capabilities: vec![ProcMacroCapability::Stdio],
        exports: Vec::new(),
    };
    let request = ProcMacroRequest {
        protocol_version: PROC_MACRO_PROTOCOL_VERSION + 1,
        compiler_version: env!("CARGO_PKG_VERSION").to_string(),
        macro_name: "make_main".to_string(),
        macro_identity: "macros::make_main".to_string(),
        source_module: None,
        expansion_id: None,
        input: Vec::new(),
    };

    let response = run_proc_macro_process(&artifact, &request, Duration::from_millis(10))
        .expect("protocol mismatch should not fail the runner");

    assert!(
        matches!(response, ProcMacroResponse::Diagnostics { messages } if messages[0].contains("protocol version mismatch"))
    );
}

#[test]
fn proc_macro_runner_encodes_request_before_spawning_child() {
    let artifact = ProcMacroArtifact {
        artifact_format_version: crate::products::PRODUCT_ARTIFACT_FORMAT_VERSION,
        crate_identity: "macros".to_string(),
        protocol_version: PROC_MACRO_PROTOCOL_VERSION,
        host_triple: "x86_64-unknown-linux-gnu".to_string(),
        executable: "/definitely/not/a/proc/macro".into(),
        capabilities: vec![ProcMacroCapability::Stdio],
        exports: Vec::new(),
    };
    let request = ProcMacroRequest {
        protocol_version: PROC_MACRO_PROTOCOL_VERSION,
        compiler_version: env!("CARGO_PKG_VERSION").to_string(),
        macro_name: "make_main".to_string(),
        macro_identity: "macros::make_main".to_string(),
        source_module: None,
        expansion_id: None,
        input: vec![0; OVERSIZED_PAYLOAD_BYTES],
    };

    let err = run_proc_macro_process(&artifact, &request, Duration::from_millis(10))
        .expect_err("oversized request should fail during encoding");

    assert!(err.contains("the size limit has been reached"), "{err}");
    assert!(!err.contains("failed to start proc macro"), "{err}");
}

#[test]
fn read_limited_to_end_rejects_oversized_pipe_output() {
    let output = vec![0; OVERSIZED_PAYLOAD_BYTES];

    let err = read_limited_to_end(&output[..], MAX_PROC_MACRO_MESSAGE_BYTES).unwrap_err();

    assert!(err.contains("exceeded maximum proc macro message size"));
}

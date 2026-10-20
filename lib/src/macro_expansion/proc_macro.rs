use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use bincode::Options;
use serde::{Deserialize, Serialize};

use crate::ast::{InvocationId, MacroOutputCategory};
use super::{ExpansionId, TokenStream};

mod schema;
mod literals;
mod fragments;
pub(super) use fragments::materialize;
#[cfg(test)]
mod fragment_tests;
mod transport;
mod artifact;
pub use artifact::{ProcMacroFileIdentity, ProcMacroHostFile};
#[cfg(test)]
mod artifact_tests;
pub use schema::*;

pub const PROC_MACRO_PROTOCOL_VERSION: u32 = 7;
pub const MAX_PROC_MACRO_MESSAGE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcMacroKind {
    FunctionLike,
    AttributeLike,
    DeriveLike,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcMacroInputShape {
    TokenStream,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcMacroCapability {
    Stdio,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcMacroExport {
    pub name: String,
    pub identity: String,
    pub kind: ProcMacroKind,
    pub input_shape: ProcMacroInputShape,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcMacroArtifact {
    pub artifact_format_version: u32,
    pub crate_identity: String,
    pub protocol_version: u32,
    pub compiler_version: String,
    pub host_triple: String,
    pub executable: PathBuf,
    pub executable_identity: ProcMacroFileIdentity,
    #[serde(deserialize_with = "crate::serialization::bounded_vec::<'de, _, _, 128>")]
    pub required_files: Vec<ProcMacroHostFile>,
    #[serde(deserialize_with = "crate::serialization::bounded_vec::<'de, _, _, 1>")]
    pub capabilities: Vec<ProcMacroCapability>,
    #[serde(deserialize_with = "crate::serialization::bounded_vec::<'de, _, _, MAX_METADATA_ENTRIES>")]
    pub definition_dependencies: Vec<crate::products::ProductCrateIdentity>,
    #[serde(deserialize_with = "crate::serialization::bounded_vec::<'de, _, _, MAX_METADATA_ENTRIES>")]
    pub exports: Vec<ProcMacroExport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcMacroRequest {
    pub protocol_version: u32,
    pub compiler_version: String,
    pub macro_name: String,
    pub macro_identity: String,
    pub source_module: Option<String>,
    pub expansion_id: Option<u32>,
    pub invocation_id: InvocationId,
    pub parent: Option<ExpansionId>,
    pub output_category: MacroOutputCategory,
    pub kind: ProcMacroKind,
    pub header_indent: usize,
    pub block_base: Option<usize>,
    #[serde(deserialize_with = "crate::serialization::bounded_vec::<'de, _, _, 1048576>")]
    pub input: Vec<u8>,
    #[serde(deserialize_with = "crate::serialization::bounded_vec::<'de, _, _, MAX_METADATA_ENTRIES>")]
    pub definition_dependencies: Vec<crate::products::ProductCrateIdentity>,
    pub views: ProcMacroViews,
}

impl ProcMacroRequest {
    pub fn construction_context(&self, kind: crate::lexer::lossless::ConstructionKind) -> crate::lexer::lossless::ConstructionContext {
        crate::lexer::lossless::ConstructionContext { invocation: self.invocation_id, kind }
    }

    pub fn source_fragment(&self, text: String, kind: crate::lexer::lossless::ConstructionKind) -> Result<TokenStream, crate::lexer::lossless::LosslessError> {
        TokenStream::source_fragment(text, self.construction_context(kind), super::token_tree::MACRO_LIMITS)
    }
}

pub fn encode_request(request: &ProcMacroRequest) -> Result<Vec<u8>, bincode::Error> {
    let bytes = bincode::DefaultOptions::new()
        .with_limit(MAX_PROC_MACRO_MESSAGE_BYTES)
        .serialize(request)?;
    request.validate().map_err(protocol_error)?;
    Ok(bytes)
}

pub fn decode_request(bytes: &[u8]) -> Result<ProcMacroRequest, bincode::Error> {
    check_size(bytes)?;
    check_version(bytes)?;
    let request: ProcMacroRequest = bincode::DefaultOptions::new()
        .with_limit(MAX_PROC_MACRO_MESSAGE_BYTES)
        .deserialize(bytes)?;
    request.validate().map_err(protocol_error)?;
    Ok(request)
}

pub fn encode_response(response: &ProcMacroResponse) -> Result<Vec<u8>, bincode::Error> {
    let bytes = bincode::DefaultOptions::new()
        .with_limit(MAX_PROC_MACRO_MESSAGE_BYTES)
        .serialize(response)?;
    response.validate().map_err(protocol_error)?;
    Ok(bytes)
}

pub fn decode_response(bytes: &[u8]) -> Result<ProcMacroResponse, bincode::Error> {
    check_size(bytes)?;
    check_version(bytes)?;
    let response: ProcMacroResponse = bincode::DefaultOptions::new()
        .with_limit(MAX_PROC_MACRO_MESSAGE_BYTES)
        .deserialize(bytes)?;
    response.validate().map_err(protocol_error)?;
    Ok(response)
}

pub fn encode_stream(stream: &TokenStream) -> Result<Vec<u8>, bincode::Error> {
    bincode::DefaultOptions::new()
        .with_limit(MAX_PROC_MACRO_MESSAGE_BYTES)
        .serialize(stream)
}

pub fn decode_stream(bytes: &[u8]) -> Result<TokenStream, bincode::Error> {
    check_size(bytes)?;
    bincode::DefaultOptions::new()
        .with_limit(MAX_PROC_MACRO_MESSAGE_BYTES)
        .deserialize(bytes)
}

fn protocol_error(message: String) -> bincode::Error { Box::new(bincode::ErrorKind::Custom(message)) }

fn check_size(bytes: &[u8]) -> Result<(), bincode::Error> {
    if bytes.len() as u64 > MAX_PROC_MACRO_MESSAGE_BYTES { Err(Box::new(bincode::ErrorKind::SizeLimit)) } else { Ok(()) }
}

fn check_version(bytes: &[u8]) -> Result<(), bincode::Error> {
    let version: u32 = bincode::DefaultOptions::new().with_limit(MAX_PROC_MACRO_MESSAGE_BYTES)
        .deserialize_from(std::io::Cursor::new(bytes))?;
    if version != PROC_MACRO_PROTOCOL_VERSION { return Err(protocol_error("unsupported proc macro protocol version".into())); }
    Ok(())
}

#[cfg(test)]
mod wire_tests;

pub fn run_proc_macro_process(
    artifact: &ProcMacroArtifact,
    request: &ProcMacroRequest,
    timeout: Duration,
    no_proc_execution: bool,
    cancellation: Option<&AtomicBool>,
) -> Result<ProcMacroResponse, String> {
    let started = Instant::now();
    let describe = |cause| format!("Proc macro '{}' ({}): {cause}", request.macro_name, request.macro_identity);
    if no_proc_execution { return Err(describe("procedural macro execution is disabled (--no-proc-execution)".into())); }
    if cancellation.is_some_and(|flag| flag.load(Ordering::Acquire)) { return Err(describe("proc macro cancelled before spawn".into())); }
    if artifact.protocol_version != PROC_MACRO_PROTOCOL_VERSION || request.protocol_version != PROC_MACRO_PROTOCOL_VERSION {
        return Err(describe(format!(
                "protocol version mismatch: artifact={}, request={}",
                artifact.protocol_version, request.protocol_version
            )));
    }

    let request_bytes = encode_request(request).map_err(|err| describe(format!("failed to encode request: {err}")))?;
    if started.elapsed() >= timeout { return Err(describe("proc macro timed out before spawn".into())); }
    artifact.validate_invocation(request).map_err(describe)?;
    transport::run(artifact, request, &request_bytes, started, timeout, no_proc_execution, cancellation).map_err(describe)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OVERSIZED_PAYLOAD_BYTES: usize = MAX_PROC_MACRO_MESSAGE_BYTES as usize + 1;

    #[test]
    fn proc_macro_protocol_version_tracks_enriched_request_shape() {
        assert_eq!(PROC_MACRO_PROTOCOL_VERSION, 7);
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
            invocation_id: InvocationId::fresh().unwrap(),
            parent: None,
            output_category: MacroOutputCategory::Items,
            kind: ProcMacroKind::FunctionLike,
            header_indent: 0, block_base: None, views: Default::default(),
            input: encode_stream(&super::super::token_tree::source("")).unwrap(),
            definition_dependencies: Vec::new(),
        };
        let encoded = encode_request(&request).unwrap();
        let decoded = decode_request(&encoded).unwrap();

        assert_eq!(decoded.macro_name, "make_main");

        let response = ProcMacroResponse::expanded(encode_stream(&super::super::token_tree::source("")).unwrap());
        let encoded = encode_response(&response).unwrap();
        let decoded = decode_response(&encoded).unwrap();

        assert!(decoded.output.is_some());
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
            invocation_id: InvocationId::fresh().unwrap(),
            parent: Some(ExpansionId(3)),
            output_category: MacroOutputCategory::Expression,
            kind: ProcMacroKind::FunctionLike,
            header_indent: 0, block_base: None, views: Default::default(),
            input: encode_stream(&super::super::token_tree::source("\"é\" /* trivia */")).unwrap(),
            definition_dependencies: Vec::new(),
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
            compiler_version: env!("CARGO_PKG_VERSION").into(), executable_identity: Default::default(), required_files: Vec::new(),
            capabilities: vec![ProcMacroCapability::Stdio],
            definition_dependencies: Vec::new(),
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
        let input = super::super::token_tree::source("main /* trivia */\r\n");

        let encoded = encode_stream(&input).unwrap();
        let decoded = decode_stream(&encoded).unwrap();

        assert_eq!(encode_stream(&decoded).unwrap(), encoded);
    }

    #[test]
    fn proc_macro_artifact_records_host_executable_and_exports() {
        let artifact = ProcMacroArtifact {
            artifact_format_version: crate::products::PRODUCT_ARTIFACT_FORMAT_VERSION,
            crate_identity: "macros".to_string(),
            protocol_version: PROC_MACRO_PROTOCOL_VERSION,
            host_triple: "x86_64-unknown-linux-gnu".to_string(),
            executable: "target/proc-macros/make_main".into(),
            compiler_version: env!("CARGO_PKG_VERSION").into(), executable_identity: Default::default(), required_files: Vec::new(),
            capabilities: vec![ProcMacroCapability::Stdio],
            definition_dependencies: Vec::new(),
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
            invocation_id: InvocationId::fresh().unwrap(),
            parent: None,
            output_category: MacroOutputCategory::Items,
            kind: ProcMacroKind::FunctionLike,
            header_indent: 0, block_base: None, views: Default::default(),
            input: vec![0; OVERSIZED_PAYLOAD_BYTES],
            definition_dependencies: Vec::new(),
        };
        let encoded = bincode::serialize(&request).unwrap();

        assert!(decode_request(&encoded).is_err());
    }

    #[test]
    fn decode_response_rejects_oversized_payload() {
        let response = ProcMacroResponse::expanded(vec![0; OVERSIZED_PAYLOAD_BYTES]);
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
            compiler_version: env!("CARGO_PKG_VERSION").into(), executable_identity: Default::default(), required_files: Vec::new(),
            capabilities: vec![ProcMacroCapability::Stdio],
            exports: Vec::new(),
            definition_dependencies: Vec::new(),
        };
        let request = ProcMacroRequest {
            protocol_version: PROC_MACRO_PROTOCOL_VERSION + 1,
            compiler_version: env!("CARGO_PKG_VERSION").to_string(),
            macro_name: "make_main".to_string(),
            macro_identity: "macros::make_main".to_string(),
            source_module: None,
            expansion_id: None,
            invocation_id: InvocationId::fresh().unwrap(),
            parent: None,
            output_category: MacroOutputCategory::Items,
            kind: ProcMacroKind::FunctionLike,
            header_indent: 0, block_base: None, views: Default::default(),
            input: Vec::new(),
            definition_dependencies: Vec::new(),
        };

        let error = run_proc_macro_process(&artifact, &request, Duration::from_millis(10), false, None)
            .expect_err("protocol mismatch must reject before spawn");

        assert!(
            error.contains("protocol version mismatch")
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
            compiler_version: env!("CARGO_PKG_VERSION").into(), executable_identity: Default::default(), required_files: Vec::new(),
            capabilities: vec![ProcMacroCapability::Stdio],
            exports: Vec::new(),
            definition_dependencies: Vec::new(),
        };
        let request = ProcMacroRequest {
            protocol_version: PROC_MACRO_PROTOCOL_VERSION,
            compiler_version: env!("CARGO_PKG_VERSION").to_string(),
            macro_name: "make_main".to_string(),
            macro_identity: "macros::make_main".to_string(),
            source_module: None,
            expansion_id: None,
            invocation_id: InvocationId::fresh().unwrap(),
            parent: None,
            output_category: MacroOutputCategory::Items,
            kind: ProcMacroKind::FunctionLike,
            header_indent: 0, block_base: None, views: Default::default(),
            input: vec![0; OVERSIZED_PAYLOAD_BYTES],
            definition_dependencies: Vec::new(),
        };

        let err = run_proc_macro_process(&artifact, &request, Duration::from_millis(10), false, None)
            .expect_err("oversized request should fail during encoding");

        assert!(err.contains("the size limit has been reached"), "{err}");
        assert!(!err.contains("failed to start proc macro"), "{err}");
    }

}

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bincode::Options;
use serde::{Deserialize, Serialize};

use crate::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics};
use crate::lexer::{Span, Token};

pub const PROC_MACRO_PROTOCOL_VERSION: u32 = 2;
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
    pub host_triple: String,
    pub executable: PathBuf,
    pub capabilities: Vec<ProcMacroCapability>,
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
    pub input: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcMacroResponse {
    Expand { output: Vec<u8> },
    Diagnostics { messages: Vec<String> },
}

impl ProcMacroResponse {
    pub fn into_diagnostics_at(self, macro_name: &str, span: Span) -> Diagnostics {
        let mut diagnostics = Diagnostics::default();
        match self {
            ProcMacroResponse::Expand { .. } => {}
            ProcMacroResponse::Diagnostics { messages } => {
                for message in messages {
                    diagnostics.push(
                        Diagnostic::new(
                            format!("Proc macro '{}' failed: {}", macro_name, message),
                            span.clone(),
                        )
                        .with_code(DiagnosticCode::Macro),
                    );
                }
            }
        }
        diagnostics
    }
}

pub fn encode_request(request: &ProcMacroRequest) -> Result<Vec<u8>, bincode::Error> {
    bincode::DefaultOptions::new()
        .with_limit(MAX_PROC_MACRO_MESSAGE_BYTES)
        .serialize(request)
}

pub fn decode_request(bytes: &[u8]) -> Result<ProcMacroRequest, bincode::Error> {
    bincode::DefaultOptions::new()
        .with_limit(MAX_PROC_MACRO_MESSAGE_BYTES)
        .deserialize(bytes)
}

pub fn encode_response(response: &ProcMacroResponse) -> Result<Vec<u8>, bincode::Error> {
    bincode::DefaultOptions::new()
        .with_limit(MAX_PROC_MACRO_MESSAGE_BYTES)
        .serialize(response)
}

pub fn decode_response(bytes: &[u8]) -> Result<ProcMacroResponse, bincode::Error> {
    bincode::DefaultOptions::new()
        .with_limit(MAX_PROC_MACRO_MESSAGE_BYTES)
        .deserialize(bytes)
}

pub fn encode_tokens(tokens: &[Token]) -> Result<Vec<u8>, bincode::Error> {
    bincode::DefaultOptions::new()
        .with_limit(MAX_PROC_MACRO_MESSAGE_BYTES)
        .serialize(tokens)
}

pub fn decode_tokens(bytes: &[u8]) -> Result<Vec<Token>, bincode::Error> {
    bincode::DefaultOptions::new()
        .with_limit(MAX_PROC_MACRO_MESSAGE_BYTES)
        .deserialize(bytes)
}

fn read_limited_to_end(mut reader: impl Read, limit: u64) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut buffer = [0; 8192];

    loop {
        let bytes_read = reader.read(&mut buffer).map_err(|err| err.to_string())?;
        if bytes_read == 0 {
            return Ok(output);
        }

        if output.len() as u64 + bytes_read as u64 > limit {
            return Err(format!(
                "exceeded maximum proc macro message size of {} bytes",
                limit
            ));
        }

        output.extend_from_slice(&buffer[..bytes_read]);
    }
}

fn join_io_thread<T>(
    handle: JoinHandle<Result<T, String>>,
    stream_name: &str,
) -> Result<T, String> {
    handle
        .join()
        .map_err(|_| format!("proc macro {} thread panicked", stream_name))?
}

fn kill_and_wait(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

pub fn run_proc_macro_process(
    artifact: &ProcMacroArtifact,
    request: &ProcMacroRequest,
    timeout: Duration,
) -> Result<ProcMacroResponse, String> {
    if artifact.protocol_version != request.protocol_version {
        return Ok(ProcMacroResponse::Diagnostics {
            messages: vec![format!(
                "protocol version mismatch: artifact={}, request={}",
                artifact.protocol_version, request.protocol_version
            )],
        });
    }

    let request_bytes = encode_request(request).map_err(|err| err.to_string())?;

    let mut child = Command::new(&artifact.executable)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("failed to start proc macro: {}", err))?;

    let mut child_stdin = match child.stdin.take() {
        Some(stdin) => stdin,
        None => {
            kill_and_wait(&mut child);
            return Err("proc macro stdin unavailable".to_string());
        }
    };
    let child_stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            kill_and_wait(&mut child);
            return Err("proc macro stdout unavailable".to_string());
        }
    };
    let child_stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            kill_and_wait(&mut child);
            return Err("proc macro stderr unavailable".to_string());
        }
    };

    let stdin_handle = std::thread::spawn(move || {
        child_stdin
            .write_all(&request_bytes)
            .map_err(|err| err.to_string())
    });

    let stdout_handle =
        std::thread::spawn(move || read_limited_to_end(child_stdout, MAX_PROC_MACRO_MESSAGE_BYTES));

    let stderr_handle =
        std::thread::spawn(move || read_limited_to_end(child_stderr, MAX_PROC_MACRO_MESSAGE_BYTES));

    let started = Instant::now();
    loop {
        if started.elapsed() > timeout {
            kill_and_wait(&mut child);
            let _ = join_io_thread(stdin_handle, "stdin");
            let _ = join_io_thread(stdout_handle, "stdout");
            let _ = join_io_thread(stderr_handle, "stderr");
            return Ok(ProcMacroResponse::Diagnostics {
                messages: vec!["proc macro timed out".to_string()],
            });
        }

        if let Some(status) = child.try_wait().map_err(|err| err.to_string())? {
            let stdin_result = join_io_thread(stdin_handle, "stdin");
            let stdout = join_io_thread(stdout_handle, "stdout")?;
            let _stderr = join_io_thread(stderr_handle, "stderr")?;

            if !status.success() {
                return Ok(ProcMacroResponse::Diagnostics {
                    messages: vec![format!("proc macro exited with {}", status)],
                });
            }

            stdin_result?;

            return decode_response(&stdout).map_err(|err| err.to_string());
        }

        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests;

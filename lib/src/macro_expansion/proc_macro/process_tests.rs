#![cfg(unix)]

use std::path::PathBuf;
use std::sync::{atomic::{AtomicU64, Ordering}, mpsc};

use super::*;

fn fixture(mode: &str, large_input: bool) -> (ProcMacroArtifact, ProcMacroRequest, PathBuf) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let directory = std::env::temp_dir().join(format!("rock-proc-io-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&directory).unwrap();
    let executable = directory.join(mode);
    let status = std::process::Command::new("cc")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/macros/io-worker.c"))
        .arg("-o").arg(&executable).status().unwrap();
    assert!(status.success());
    let input = super::super::token_tree::source(&if large_input { "x".repeat(128 * 1024) } else { String::new() });
    let request = ProcMacroRequest { protocol_version: PROC_MACRO_PROTOCOL_VERSION, compiler_version: env!("CARGO_PKG_VERSION").into(),
        macro_name: "run".into(), macro_identity: "fixture::run".into(), source_module: None, expansion_id: Some(1),
        invocation_id: InvocationId::fresh().unwrap(), parent: None, output_category: MacroOutputCategory::Items,
        kind: ProcMacroKind::FunctionLike, header_indent: 0, block_base: None, input: encode_stream(&input).unwrap(),
        definition_dependencies: Vec::new(), views: Default::default(),
    };
    let artifact = ProcMacroArtifact { artifact_format_version: crate::products::PRODUCT_ARTIFACT_FORMAT_VERSION,
        crate_identity: "fixture".into(), protocol_version: PROC_MACRO_PROTOCOL_VERSION, host_triple: "test-host".into(),
        executable, capabilities: vec![ProcMacroCapability::Stdio], definition_dependencies: Vec::new(), exports: Vec::new(),
    };
    let response = ProcMacroResponse { protocol_version: PROC_MACRO_PROTOCOL_VERSION,
        output: Some(ProcMacroOutput { parts: Vec::new() }), diagnostics: Vec::new() };
    std::fs::write(artifact.executable.with_extension("response"), encode_response(&response).unwrap()).unwrap();
    (artifact, request, directory)
}

fn bounded_run(artifact: ProcMacroArtifact, request: ProcMacroRequest, timeout: Duration) -> Result<ProcMacroResponse, String> {
    let pid_path = artifact.executable.with_extension("pid");
    let (sender, receiver) = mpsc::channel();
    let thread = std::thread::spawn(move || { let _ = sender.send(run_proc_macro_process(&artifact, &request, timeout)); });
    let result = match receiver.recv_timeout(Duration::from_secs(3)) {
        Ok(result) => result,
        Err(error) => {
            if let Ok(pid) = std::fs::read_to_string(&pid_path).unwrap_or_default().parse::<i32>() {
                unsafe { libc::kill(-pid, libc::SIGKILL); libc::kill(pid, libc::SIGKILL); }
            }
            panic!("transport exceeded independent watchdog: {error}");
        }
    };
    thread.join().unwrap();
    if let Ok(pid) = std::fs::read_to_string(&pid_path).unwrap_or_default().parse::<i32>() {
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "worker {pid} survived transport return");
        assert_eq!(unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) }, -1, "worker was not reaped");
    }
    result
}

#[test]
fn macro_b_02_partial_io_success_requires_a_valid_response_and_clean_exit() {
    let (artifact, request, directory) = fixture("partial", true);
    let response = bounded_run(artifact, request, Duration::from_secs(2)).unwrap();
    assert!(response.output.is_some());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn macro_b_02_io_and_shutdown_stalls_share_the_invocation_deadline() {
    for mode in ["blocked-stdin", "blocked-stdout", "blocked-shutdown", "broken-pipe"] {
        let (artifact, request, directory) = fixture(mode, true);
        let pid_path = artifact.executable.with_extension("pid");
        let started = Instant::now();
        let error = bounded_run(artifact, request, Duration::from_millis(500)).unwrap_err();
        assert!(pid_path.exists(), "{mode} never reached its blocking fixture: {error}");
        assert!(started.elapsed() < Duration::from_secs(2), "{mode}: {error}");
        assert!(error.contains("timed out") || error.contains("stdin"), "{mode}: {error}");
        assert!(error.contains("fixture::run"), "{error}");
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn macro_b_02_oversized_stdout_and_stderr_are_bounded_and_reaped() {
    for mode in ["oversized-stdout", "oversized-stderr"] {
        let (artifact, request, directory) = fixture(mode, false);
        let error = bounded_run(artifact, request, Duration::from_secs(2)).unwrap_err();
        assert!(error.contains("exceeded maximum"), "{mode}: {error}");
        assert!(error.len() < 5000, "unbounded diagnostic stderr");
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn macro_b_02_malformed_nonzero_and_crashed_workers_preserve_failure_stderr() {
    for (mode, cause, marker) in [("malformed", "malformed", "bad wire"), ("nonzero", "exited", "nonzero marker"), ("crash", "exited", "crash marker")] {
        let (artifact, request, directory) = fixture(mode, false);
        let error = bounded_run(artifact, request, Duration::from_secs(2)).unwrap_err();
        assert!(error.contains(cause) && error.contains(marker), "{mode}: {error}");
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn macro_b_02_missing_executable_and_expired_deadline_do_not_spawn() {
    let (mut artifact, request, directory) = fixture("partial", false);
    let error = bounded_run(artifact.clone(), request.clone(), Duration::ZERO).unwrap_err();
    assert!(error.contains("before spawn"), "{error}");
    assert!(!artifact.executable.with_extension("pid").exists());
    artifact.executable = directory.join("missing");
    let error = bounded_run(artifact, request, Duration::from_secs(1)).unwrap_err();
    assert!(error.contains("failed to start") && error.contains("os error"), "{error}");
    std::fs::remove_dir_all(directory).unwrap();
}

#![cfg(unix)]

use std::path::PathBuf;
use std::sync::{atomic::{AtomicBool, AtomicU64, Ordering}, mpsc, Arc};
use std::time::{Duration, Instant};

use rock_lib as compiler;
use compiler::ast::{InvocationId, MacroOutputCategory};
use compiler::macro_expansion::proc_macro::*;
use compiler::macro_expansion::TokenStream;

fn fixture(mode: &str, large_input: bool) -> (ProcMacroArtifact, ProcMacroRequest, PathBuf) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let directory = std::env::temp_dir().join(format!("rock-proc-io-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&directory).unwrap();
    let executable = directory.join(mode);
    let status = std::process::Command::new("cc")
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/macros/io-worker.c"))
        .arg("-o").arg(&executable).status().unwrap();
    assert!(status.success());
    let input = TokenStream::from_bytes("/virtual/worker-input.rk".into(),
        if large_input { vec![b'x'; 128 * 1024] } else { Vec::new() }, compiler::macro_expansion::token_tree::MACRO_LIMITS).unwrap();
    let request = ProcMacroRequest { protocol_version: PROC_MACRO_PROTOCOL_VERSION, compiler_version: env!("CARGO_PKG_VERSION").into(),
        macro_name: "run".into(), macro_identity: "fixture::run".into(), source_module: None, expansion_id: Some(1),
        invocation_id: InvocationId::fresh().unwrap(), parent: None, output_category: MacroOutputCategory::Items,
        kind: ProcMacroKind::FunctionLike, header_indent: 0, block_base: None, input: encode_stream(&input).unwrap(),
        definition_dependencies: Vec::new(), views: Default::default(),
    };
    let artifact = ProcMacroArtifact { artifact_format_version: compiler::products::PRODUCT_ARTIFACT_FORMAT_VERSION,
        crate_identity: "fixture".into(), protocol_version: PROC_MACRO_PROTOCOL_VERSION, host_triple: "test-host".into(),
        executable, capabilities: vec![ProcMacroCapability::Stdio], definition_dependencies: Vec::new(), exports: Vec::new(),
    };
    let response = ProcMacroResponse { protocol_version: PROC_MACRO_PROTOCOL_VERSION,
        output: Some(ProcMacroOutput { parts: Vec::new() }), diagnostics: Vec::new() };
    std::fs::write(artifact.executable.with_extension("response"), encode_response(&response).unwrap()).unwrap();
    (artifact, request, directory)
}

fn bounded_run(artifact: ProcMacroArtifact, request: ProcMacroRequest, timeout: Duration) -> Result<ProcMacroResponse, String> {
    bounded_run_with_cancellation(artifact, request, timeout, None)
}

fn bounded_run_with_cancellation(artifact: ProcMacroArtifact, request: ProcMacroRequest, timeout: Duration, cancellation: Option<Arc<AtomicBool>>) -> Result<ProcMacroResponse, String> {
    let pid_path = artifact.executable.with_extension("pid");
    let descendant_path = artifact.executable.with_extension("desc.pid");
    let (sender, receiver) = mpsc::channel();
    let thread = std::thread::spawn(move || { let _ = sender.send(run_proc_macro_process(&artifact, &request, timeout, cancellation.as_deref())); });
    let result = match receiver.recv_timeout(Duration::from_secs(3)) {
        Ok(result) => result,
        Err(error) => {
            if let Ok(pid) = std::fs::read_to_string(&pid_path).unwrap_or_default().parse::<i32>() {
                unsafe { libc::kill(-pid, libc::SIGKILL); libc::kill(pid, libc::SIGKILL); }
            }
            if let Ok(pid) = std::fs::read_to_string(&descendant_path).unwrap_or_default().parse::<i32>() {
                unsafe { libc::kill(pid, libc::SIGKILL); }
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

fn assert_descendant_gone(path: &std::path::Path) {
    let pid = std::fs::read_to_string(path).unwrap().parse::<i32>().unwrap();
    let started = Instant::now();
    while unsafe { libc::kill(pid, 0) } == 0 && started.elapsed() < Duration::from_millis(500) {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "descendant {pid} survived worker cleanup");
}

#[test]
fn macro_b_03_descendants_retaining_pipes_close_on_success_and_decode_failure() {
    for mode in ["descendant-success", "descendant-malformed"] {
        let (artifact, request, directory) = fixture(mode, false);
        let descendant = artifact.executable.with_extension("desc.pid");
        let result = bounded_run(artifact, request, Duration::from_secs(2));
        if mode == "descendant-success" { assert!(result.is_ok(), "{result:?}"); }
        else { assert!(result.unwrap_err().contains("malformed")); }
        assert_descendant_gone(&descendant);
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn macro_b_03_descendants_close_on_timeout_and_broken_request() {
    for mode in ["descendant-timeout", "descendant-broken"] {
        let (artifact, request, directory) = fixture(mode, true);
        let descendant = artifact.executable.with_extension("desc.pid");
        let error = bounded_run(artifact, request, Duration::from_millis(500)).unwrap_err();
        assert!(error.contains("timed out") || error.contains("stdin"), "{error}");
        assert_descendant_gone(&descendant);
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn macro_b_03_cancellation_reaps_worker_and_descendants_during_io() {
    let (artifact, request, directory) = fixture("descendant-cancel", true);
    let descendant = artifact.executable.with_extension("desc.pid");
    let marker = descendant.clone();
    let cancellation = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancellation);
    let canceller = std::thread::spawn(move || {
        let started = Instant::now();
        while std::fs::read_to_string(&marker).unwrap_or_default().parse::<i32>().is_err()
            && started.elapsed() < Duration::from_secs(1) { std::thread::sleep(Duration::from_millis(1)); }
        flag.store(true, Ordering::Release);
    });
    let error = bounded_run_with_cancellation(artifact, request, Duration::from_secs(2), Some(cancellation)).unwrap_err();
    canceller.join().unwrap();
    assert!(error.contains("cancelled"), "{error}");
    assert_descendant_gone(&descendant);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn macro_b_03_pre_cancelled_expansion_never_spawns_a_worker() {
    let (mut artifact, _, directory) = fixture("partial", false);
    let config = compiler::Config::default();
    let program = compiler::parser::parse_string("%provider::make!", &config).unwrap();
    let cancellation = Arc::new(AtomicBool::new(true));
    artifact.exports.push(ProcMacroExport { name: "make".into(), identity: "provider::make".into(),
        kind: ProcMacroKind::FunctionLike, input_shape: ProcMacroInputShape::TokenStream });
    let marker = artifact.executable.with_extension("pid");
    let context = compiler::macro_expansion::MacroExpansionContext::new(&config)
        .with_named_proc_macro_artifact("provider".into(), artifact).with_cancellation(cancellation);
    let error = compiler::macro_expansion::expand_macros_with_context(program, &context).unwrap_err();
    assert!(format!("{error:?}").contains("cancelled"), "{error:?}");
    assert!(!marker.exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn macro_b_03_cancellation_interrupts_each_io_phase_and_shutdown() {
    for (mode, extension) in [("blocked-stdin", "pid"), ("blocked-stdout", "read"), ("blocked-shutdown", "written")] {
        let (artifact, request, directory) = fixture(mode, true);
        let marker = artifact.executable.with_extension(extension);
        let observed = marker.clone();
        let cancellation = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancellation);
        let canceller = std::thread::spawn(move || {
            let started = Instant::now();
            while !marker.exists() && started.elapsed() < Duration::from_secs(1) { std::thread::sleep(Duration::from_millis(1)); }
            flag.store(true, Ordering::Release);
        });
        let error = bounded_run_with_cancellation(artifact, request, Duration::from_secs(2), Some(cancellation)).unwrap_err();
        canceller.join().unwrap();
        assert!(observed.exists(), "{mode} never reached {extension}: {error}");
        assert!(error.contains("cancelled"), "{mode}: {error}");
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[cfg(target_os = "linux")]
#[test]
fn macro_b_03_repeated_cleanup_does_not_leak_descriptors_or_threads() {
    const ISOLATED: &str = "ROCK_MACRO_RESOURCE_CHILD";
    if std::env::var_os(ISOLATED).is_some() {
        let count = |path| std::fs::read_dir(path).unwrap().count();
        let descriptors = count("/proc/self/fd");
        let threads = count("/proc/self/task");
        for _ in 0..2 {
            macro_b_03_descendants_retaining_pipes_close_on_success_and_decode_failure();
            macro_b_03_descendants_close_on_timeout_and_broken_request();
            macro_b_03_cancellation_reaps_worker_and_descendants_during_io();
            macro_b_03_cancellation_interrupts_each_io_phase_and_shutdown();
            assert_eq!(count("/proc/self/fd"), descriptors, "worker pipe descriptors leaked");
            assert_eq!(count("/proc/self/task"), threads, "worker IO threads leaked");
        }
        return;
    }
    // Resource counts must not race unrelated tests opening files or threads.
    use std::os::unix::process::CommandExt;
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "macro_b_03_repeated_cleanup_does_not_leak_descriptors_or_threads", "--nocapture"])
        .env(ISOLATED, "1").process_group(0).spawn().unwrap();
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() { assert!(status.success(), "resource acceptance failed: {status}"); break; }
        if started.elapsed() > Duration::from_secs(20) {
            unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL); }
            let _ = child.kill();
            let _ = child.wait();
            panic!("isolated resource test exceeded its watchdog");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

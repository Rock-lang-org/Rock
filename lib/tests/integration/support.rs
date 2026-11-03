use crate::std_cache_key::{std_cache_compiler_stamp, std_cache_key};
use std::fs;
use std::io::Read;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::OnceLock;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

pub(crate) static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) const STD_CACHE_LOCK_STALE_AFTER: Duration = Duration::from_secs(10 * 60);
pub(crate) const TEST_PROCESS_TIMEOUT: Duration = Duration::from_secs(15);

pub(crate) fn test_temp_dir(id: u64) -> PathBuf {
    let tid = std::thread::current().id();
    std::env::temp_dir().join(format!("rock_test_{}_{:?}_{}", std::process::id(), tid, id))
}

pub(crate) fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

pub(crate) fn std_path() -> PathBuf {
    workspace_root().join("std")
}

pub(crate) struct CacheLock {
    path: PathBuf,
}

impl Drop for CacheLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub(crate) fn acquire_cache_lock(path: &Path) -> Option<CacheLock> {
    loop {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(mut file) => {
                if let Err(err) = file.write_all(cache_lock_metadata().as_bytes()) {
                    let _ = std::fs::remove_file(path);
                    panic!("failed to write std cache lock {}: {}", path.display(), err);
                }
                return Some(CacheLock {
                    path: path.to_path_buf(),
                });
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                if std_cache_entry_is_ready_for_lock(path) {
                    return None;
                }
                if recover_stale_cache_lock(path, STD_CACHE_LOCK_STALE_AFTER) {
                    continue;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err(err) => panic!(
                "failed to acquire std cache lock {}: {}",
                path.display(),
                err
            ),
        }
    }
}

pub(crate) fn cache_lock_metadata() -> String {
    format!(
        "pid={}\ncreated_unix_nanos={}\n",
        std::process::id(),
        current_unix_nanos()
    )
}

pub(crate) fn current_unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

pub(crate) fn std_cache_entry_is_ready_for_lock(lock_path: &Path) -> bool {
    let artifact_dir = lock_path.with_extension("");
    std_cache_entry_is_ready(
        &artifact_dir.join("std.rkca"),
        &artifact_dir.join("std.o"),
        &artifact_dir.join("ready"),
    )
}

pub(crate) fn std_cache_lock_is_stale(path: &Path, stale_after: Duration) -> bool {
    let age = std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| lock_age_from_metadata(&contents))
        .or_else(|| lock_age_from_file_metadata(path));

    age.is_some_and(|age| age >= stale_after)
}

pub(crate) fn recover_stale_cache_lock(path: &Path, stale_after: Duration) -> bool {
    if std_cache_entry_is_ready_for_lock(path) || !std_cache_lock_is_stale(path, stale_after) {
        return false;
    }

    match std::fs::remove_file(path) {
        Ok(()) => true,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => true,
        Err(err) => panic!(
            "failed to remove stale std cache lock {}: {}",
            path.display(),
            err
        ),
    }
}

pub(crate) fn lock_age_from_metadata(contents: &str) -> Option<Duration> {
    let created = contents
        .lines()
        .find_map(|line| line.strip_prefix("created_unix_nanos="))?
        .parse::<u128>()
        .ok()?;
    let now = current_unix_nanos();
    if created > now {
        return None;
    }

    Some(Duration::from_nanos(
        (now - created).min(u128::from(u64::MAX)) as u64,
    ))
}

pub(crate) fn lock_age_from_file_metadata(path: &Path) -> Option<Duration> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .elapsed()
        .ok()
}

pub(crate) fn std_cache_entry_is_ready(
    artifact_path: &Path,
    object_path: &Path,
    ready_path: &Path,
) -> bool {
    ready_path.is_file()
        && object_path.is_file()
        && artifact_path.is_file()
        && rock_lib::products::CompilerProducts::read_artifact_from_path(artifact_path).is_ok()
}

pub(crate) fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &dst_path)?;
        } else {
            std::fs::copy(entry.path(), dst_path)?;
        }
    }

    Ok(())
}

pub(crate) fn publish_std_cache_dir(
    build_dir: &Path,
    artifact_dir: &Path,
    ready_path: &Path,
) -> Result<(), String> {
    remove_file_if_exists(ready_path).map_err(|err| {
        format!(
            "failed to remove stale std cache ready marker {}: {}",
            ready_path.display(),
            err
        )
    })?;
    remove_dir_if_exists(artifact_dir).map_err(|err| {
        format!(
            "failed to remove stale std cache directory {}: {}",
            artifact_dir.display(),
            err
        )
    })?;
    if artifact_dir.exists() {
        return Err(format!(
            "std cache destination still exists before publish: {}",
            artifact_dir.display()
        ));
    }

    match std::fs::rename(build_dir, artifact_dir) {
        Ok(()) => {}
        Err(rename_err) => {
            let publish_dir = artifact_dir.with_extension(format!(
                "publishing-{}-{}",
                std::process::id(),
                current_unix_nanos()
            ));
            remove_dir_if_exists(&publish_dir).map_err(|cleanup_err| {
                format!(
                    "failed to clean std cache publish directory {} after rename failed: {}; cleanup failed: {}",
                    publish_dir.display(),
                    rename_err,
                    cleanup_err
                )
            })?;
            copy_dir_all(build_dir, &publish_dir).map_err(|copy_err| {
                let cleanup = cleanup_error_suffix(remove_dir_if_exists(&publish_dir));
                format!(
                    "failed to copy std cache from {} to {} after rename failed: {}; copy failed: {}{}",
                    build_dir.display(),
                    publish_dir.display(),
                    rename_err,
                    copy_err,
                    cleanup
                )
            })?;
            validate_std_cache_payload(&publish_dir).map_err(|validate_err| {
                let cleanup = cleanup_error_suffix(remove_dir_if_exists(&publish_dir));
                format!(
                    "failed to validate std cache publish directory {}: {}{}",
                    publish_dir.display(),
                    validate_err,
                    cleanup
                )
            })?;
            std::fs::rename(&publish_dir, artifact_dir).map_err(|publish_err| {
                let cleanup = cleanup_error_suffix(remove_dir_if_exists(&publish_dir));
                format!(
                    "failed to publish copied std cache from {} to {} after rename failed: {}; publish failed: {}{}",
                    publish_dir.display(),
                    artifact_dir.display(),
                    rename_err,
                    publish_err,
                    cleanup
                )
            })?;
            remove_dir_if_exists(build_dir).map_err(|cleanup_err| {
                format!(
                    "failed to remove copied std cache build directory {}: {}",
                    build_dir.display(),
                    cleanup_err
                )
            })?;
        }
    }

    validate_std_cache_payload(artifact_dir)?;
    std::fs::write(ready_path, b"ready").map_err(|err| {
        format!(
            "failed to mark std cache directory ready {}: {}",
            ready_path.display(),
            err
        )
    })
}

pub(crate) fn remove_file_if_exists(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

pub(crate) fn remove_dir_if_exists(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

pub(crate) fn cleanup_error_suffix(result: std::io::Result<()>) -> String {
    match result {
        Ok(()) => String::new(),
        Err(err) => format!("; cleanup failed: {err}"),
    }
}

pub(crate) fn validate_std_cache_payload(dir: &Path) -> Result<(), String> {
    let artifact_path = dir.join("std.rkca");
    let object_path = dir.join("std.o");
    if !object_path.is_file() {
        return Err(format!(
            "std cache object file is missing: {}",
            object_path.display()
        ));
    }
    rock_lib::products::CompilerProducts::read_artifact_from_path(&artifact_path).map(|_| ())
}

pub(crate) fn std_artifact_path() -> PathBuf {
    static STD_ARTIFACT: OnceLock<PathBuf> = OnceLock::new();

    STD_ARTIFACT
        .get_or_init(|| {
            let std_dir = std_path();
            let cache_key = std_cache_key(&std_dir, &std_cache_compiler_stamp());
            let artifact_dir = std::env::temp_dir().join(format!(
                "rock_integration_std_product_artifact_{}",
                cache_key
            ));
            let artifact_path = artifact_dir.join("std.rkca");
            let object_path = artifact_dir.join("std.o");
            let ready_path = artifact_dir.join("ready");
            let lock_path = artifact_dir.with_extension("lock");
            if std_cache_entry_is_ready(&artifact_path, &object_path, &ready_path) {
                return artifact_path;
            }

            let Some(_cache_lock) = acquire_cache_lock(&lock_path) else {
                return artifact_path;
            };
            if std_cache_entry_is_ready(&artifact_path, &object_path, &ready_path) {
                return artifact_path;
            }

            let build_dir = artifact_dir.with_extension(format!("building-{}", std::process::id()));
            remove_dir_if_exists(&build_dir)
                .expect("Failed to clean std product artifact build directory");
            std::fs::create_dir_all(&build_dir).unwrap();
            let build_artifact_path = build_dir.join("std.rkca");
            let build_object_path = build_dir.join("std.o");

            let output = rock_lib::compile_with_products(&rock_lib::Config {
                entry_file: std_dir.join("src/lib.rk"),
                output_dir: build_dir.clone(),
                debug_print: vec![],
                meta_files: vec![],
                extern_artifacts: vec![],
                source_providers: Vec::new(),
                current_crate_name: Some("std".to_string()),
                opt_level: 0,
                overflow_checks: None,
                emit_llvm: false,
                no_link: true,
                emit_object: Some(build_object_path),
                no_prelude: false,
                no_std: true,
                sysroot: None,
            })
            .unwrap_or_else(|diagnostics| {
                diagnostics.report();
                panic!("Failed to compile std product artifact for integration tests");
            });
            let mut products = output
                .products
                .expect("std compile should produce product data");
            products.link.object_path = Some(PathBuf::from("std.o"));
            products
                .write_artifact_to_path(&build_artifact_path)
                .expect("Failed to write std product artifact for integration tests");

            rock_lib::products::CompilerProducts::read_artifact_from_path(&build_artifact_path)
                .expect("Failed to validate std product artifact for integration tests");

            publish_std_cache_dir(&build_dir, &artifact_dir, &ready_path)
                .expect("Failed to publish std product artifact cache");
            artifact_path
        })
        .clone()
}

pub(crate) fn test_config(entry_file: PathBuf, output_dir: PathBuf) -> rock_lib::Config {
    rock_lib::Config {
        entry_file,
        output_dir,
        debug_print: vec![],
        meta_files: vec![],
        extern_artifacts: vec![("std".to_string(), std_artifact_path())],
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

pub(crate) struct TestDirCleanup(pub(crate) PathBuf);

impl Drop for TestDirCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn child_exited_without_reaping(child: &Child) -> Result<bool, std::io::Error> {
    let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            child.id() as libc::id_t,
            info.as_mut_ptr(),
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { info.assume_init().si_pid() } != 0)
}

pub(crate) fn terminate_test_command_group(child: &Child) -> Result<(), std::io::Error> {
    // A negative PID targets the child's process group; ESRCH only means it already exited.
    let result = unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
    if result != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error);
        }
    }
    Ok(())
}

pub(crate) fn terminate_and_collect_test_command(
    child: &mut Child,
    stdout_reader: std::thread::JoinHandle<Vec<u8>>,
    stderr_reader: std::thread::JoinHandle<Vec<u8>>,
) -> Result<(std::process::ExitStatus, Vec<u8>, Vec<u8>), (std::io::Error, std::process::ExitStatus)>
{
    if let Err(error) = terminate_test_command_group(child) {
        let _ = child.kill();
        let status = child.wait().expect("failed child must be reaped");
        drop(stdout_reader);
        drop(stderr_reader);
        return Err((error, status));
    }
    let status = child.wait().expect("failed child must be reaped");
    let stdout = stdout_reader.join().expect("stdout reader must finish");
    let stderr = stderr_reader.join().expect("stderr reader must finish");
    Ok((status, stdout, stderr))
}

pub(crate) fn run_test_command(command: &mut Command) -> Output {
    let executable = command.get_program().to_string_lossy().into_owned();
    command.process_group(0);
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("failed to spawn {executable}: {error}"));
    let stdout = child.stdout.take().expect("child stdout must be piped");
    let stderr = child.stderr.take().expect("child stderr must be piped");
    let stdout_reader = std::thread::spawn(move || {
        let mut stdout = stdout;
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut stderr = stderr;
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let started = Instant::now();
    loop {
        match child_exited_without_reaping(&child) {
            Ok(true) => break,
            Ok(false) if started.elapsed() < TEST_PROCESS_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(false) => {
                let (status, stdout, stderr) = terminate_and_collect_test_command(
                    &mut child,
                    stdout_reader,
                    stderr_reader,
                )
                .unwrap_or_else(|(error, status)| {
                    panic!(
                        "failed to terminate process group {} for {executable}: {error} (direct child status {status})",
                        child.id()
                    )
                });
                panic!(
                    "{executable} exceeded {:?} (status {status}); stdout:\n{}\nstderr:\n{}",
                    TEST_PROCESS_TIMEOUT,
                    String::from_utf8_lossy(&stdout),
                    String::from_utf8_lossy(&stderr),
                );
            }
            Err(error) => {
                let (status, stdout, stderr) = terminate_and_collect_test_command(
                    &mut child,
                    stdout_reader,
                    stderr_reader,
                )
                .unwrap_or_else(|(cleanup_error, status)| {
                    panic!(
                        "failed while waiting for {executable}: {error}; failed to terminate process group {}: {cleanup_error} (direct child status {status})",
                        child.id()
                    )
                });
                panic!(
                    "failed while waiting for {executable}: {error} (status {status}); stdout:\n{}\nstderr:\n{}",
                    String::from_utf8_lossy(&stdout),
                    String::from_utf8_lossy(&stderr),
                );
            }
        }
    }
    if let Err(error) = terminate_test_command_group(&child) {
        let status = child.wait().expect("exited child must be reaped");
        drop(stdout_reader);
        drop(stderr_reader);
        panic!(
            "failed to terminate process group {} for {executable}: {error} (direct child status {status})",
            child.id(),
        );
    }
    let status = child.wait().expect("exited child must be reaped");
    let stdout = stdout_reader.join().expect("stdout reader must finish");
    let stderr = stderr_reader.join().expect("stderr reader must finish");

    Output {
        status,
        stdout,
        stderr,
    }
}

pub(crate) fn compile_and_run_without_std(source: &str) -> i32 {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    let source_path = dir.join(format!("no_std_{id}.rk"));
    fs::write(&source_path, source).unwrap();

    let mut config = test_config(source_path, dir.clone());
    config.extern_artifacts.clear();
    config.no_std = true;
    config.no_prelude = true;
    rock_lib::compile(&config).expect("no-std compilation failed");

    let mut command = Command::new(dir.join(format!("no_std_{id}")));
    run_test_command(&mut command).status.code().unwrap_or(1)
}

pub(crate) fn compile_and_run(source: &str) -> String {
    compile_and_run_at_opt(source, 0)
}

pub(crate) fn compile_and_run_at_opt(source: &str, opt_level: u8) -> String {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    let source_path = dir.join(format!("test_{}.rk", id));
    std::fs::write(&source_path, source).unwrap();

    let mut config = test_config(source_path.clone(), dir.clone());
    config.opt_level = opt_level;

    rock_lib::compile(&config).expect("Compilation failed");

    let binary = dir.join(format!("test_{}", id));
    let output = Command::new(&binary)
        .output()
        .expect("Failed to run compiled binary");

    assert!(
        output.status.success(),
        "compiled program failed with {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();

    // Cleanup
    let _ = std::fs::remove_dir_all(&dir);

    stdout
}

pub(crate) fn compile_and_run_with_args(source: &str, args: &[&str]) -> String {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    let source_path = dir.join(format!("test_{}.rk", id));
    std::fs::write(&source_path, source).unwrap();

    let config = test_config(source_path, dir.clone());
    rock_lib::compile(&config).expect("Compilation failed");

    let binary = dir.join(format!("test_{}", id));
    let output = Command::new(&binary)
        .args(args)
        .output()
        .expect("Failed to run compiled binary");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();

    let _ = std::fs::remove_dir_all(&dir);
    stdout
}

pub(crate) fn compile_and_run_with_status(source: &str) -> (String, bool) {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    let source_path = dir.join(format!("test_{}.rk", id));
    std::fs::write(&source_path, source).unwrap();

    let config = test_config(source_path.clone(), dir.clone());
    rock_lib::compile(&config).expect("Compilation failed");

    let binary = dir.join(format!("test_{}", id));
    let output = Command::new(&binary)
        .output()
        .expect("Failed to run compiled binary");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let success = output.status.success();

    let _ = std::fs::remove_dir_all(&dir);

    (stdout, success)
}

pub(crate) fn compile_and_run_in_working_dir(source: &str, dir: &Path) -> Output {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    fs::create_dir_all(dir).unwrap();

    let source_path = dir.join(format!("test_{id}.rk"));
    fs::write(&source_path, source).unwrap();

    let config = test_config(source_path, dir.to_path_buf());
    rock_lib::compile(&config).expect("Compilation failed");

    let mut command = Command::new(dir.join(format!("test_{id}")));
    command.current_dir(dir);
    run_test_command(&mut command)
}

pub(crate) fn compile_example(name: &str) -> String {
    let examples_dir = workspace_root().join("examples");
    let source_path = examples_dir.join(format!("{}.rk", name));
    let source = std::fs::read_to_string(&source_path)
        .unwrap_or_else(|_| panic!("Failed to read example: {}", name));
    compile_and_run(&source)
}

// Helper for testing that compilation fails with expected error message
pub(crate) fn compile_should_fail(source: &str, expected_error: &str) {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    let source_path = dir.join("test.rk");
    std::fs::write(&source_path, source).unwrap();

    let config = test_config(source_path.clone(), dir.clone());

    let result = rock_lib::compile(&config);
    assert!(
        result.is_err(),
        "Expected compilation to fail but it succeeded"
    );

    // Check that the diagnostics contain the expected error
    if let Err(diagnostics) = result {
        let messages: Vec<String> = diagnostics.0.iter().map(|d| d.message.clone()).collect();
        let has_expected = messages.iter().any(|m| m.contains(expected_error));
        assert!(
            has_expected,
            "Expected error containing '{}' but got: {:?}",
            expected_error, messages
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

pub(crate) fn compile_should_fail_with_exact_diagnostic(source: &str, expected_error: &str) {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let source_path = dir.join("test.rk");
    fs::write(&source_path, source).unwrap();

    let mut config = test_config(source_path, dir.clone());
    config.extern_artifacts.clear();
    config.no_std = true;
    config.no_prelude = true;
    config.no_link = true;
    let diagnostics = rock_lib::compile(&config).expect_err("expected compilation to fail");
    let messages: Vec<_> = diagnostics
        .0
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();

    assert!(
        messages.iter().any(|message| *message == expected_error),
        "expected exact diagnostic {expected_error:?}, got {messages:?}"
    );
    let _ = fs::remove_dir_all(dir);
}

pub(crate) fn compile_should_pass(source: &str) {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    let source_path = dir.join("test.rk");
    std::fs::write(&source_path, source).unwrap();

    let mut config = test_config(source_path, dir.clone());
    config.no_link = true;
    rock_lib::compile(&config).expect("Compilation failed");

    let _ = std::fs::remove_dir_all(&dir);
}

pub(crate) fn compile_should_pass_without_std(source: &str) {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let source_path = dir.join("test.rk");
    fs::write(&source_path, source).unwrap();

    let mut config = test_config(source_path, dir.clone());
    config.extern_artifacts.clear();
    config.no_std = true;
    config.no_prelude = true;
    config.no_link = true;
    rock_lib::compile(&config).expect("expected compilation to pass");
    let _ = fs::remove_dir_all(dir);
}

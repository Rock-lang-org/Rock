//! Integration tests for the Rock compiler
//!
//! These tests compile example .rk files and verify their output.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bincode::Options;
use serde::Serialize;

#[path = "support/stdlib_cache_key.rs"]
mod stdlib_cache_key;
use stdlib_cache_key::{stdlib_cache_compiler_stamp, stdlib_cache_key};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);
const STDLIB_CACHE_LOCK_STALE_AFTER: Duration = Duration::from_secs(10 * 60);
const TEST_PROCESS_TIMEOUT: Duration = Duration::from_secs(15);

fn test_temp_dir(id: u64) -> PathBuf {
    let tid = std::thread::current().id();
    std::env::temp_dir().join(format!("rock_test_{}_{:?}_{}", std::process::id(), tid, id))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn stdlib_path() -> PathBuf {
    workspace_root().join("stdlib")
}

struct CacheLock {
    path: PathBuf,
}

impl Drop for CacheLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn acquire_cache_lock(path: &Path) -> Option<CacheLock> {
    loop {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(mut file) => {
                if let Err(err) = file.write_all(cache_lock_metadata().as_bytes()) {
                    let _ = std::fs::remove_file(path);
                    panic!(
                        "failed to write stdlib cache lock {}: {}",
                        path.display(),
                        err
                    );
                }
                return Some(CacheLock {
                    path: path.to_path_buf(),
                });
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                if stdlib_cache_entry_is_ready_for_lock(path) {
                    return None;
                }
                if recover_stale_cache_lock(path, STDLIB_CACHE_LOCK_STALE_AFTER) {
                    continue;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err(err) => panic!(
                "failed to acquire stdlib cache lock {}: {}",
                path.display(),
                err
            ),
        }
    }
}

fn cache_lock_metadata() -> String {
    format!(
        "pid={}\ncreated_unix_nanos={}\n",
        std::process::id(),
        current_unix_nanos()
    )
}

fn current_unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

fn stdlib_cache_entry_is_ready_for_lock(lock_path: &Path) -> bool {
    let artifact_dir = lock_path.with_extension("");
    stdlib_cache_entry_is_ready(
        &artifact_dir.join("stdlib.rkca"),
        &artifact_dir.join("stdlib.o"),
        &artifact_dir.join("ready"),
    )
}

fn stdlib_cache_lock_is_stale(path: &Path, stale_after: Duration) -> bool {
    let age = std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| lock_age_from_metadata(&contents))
        .or_else(|| lock_age_from_file_metadata(path));

    age.is_some_and(|age| age >= stale_after)
}

fn recover_stale_cache_lock(path: &Path, stale_after: Duration) -> bool {
    if stdlib_cache_entry_is_ready_for_lock(path) || !stdlib_cache_lock_is_stale(path, stale_after)
    {
        return false;
    }

    match std::fs::remove_file(path) {
        Ok(()) => true,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => true,
        Err(err) => panic!(
            "failed to remove stale stdlib cache lock {}: {}",
            path.display(),
            err
        ),
    }
}

fn lock_age_from_metadata(contents: &str) -> Option<Duration> {
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

fn lock_age_from_file_metadata(path: &Path) -> Option<Duration> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .elapsed()
        .ok()
}

fn stdlib_cache_entry_is_ready(
    artifact_path: &Path,
    object_path: &Path,
    ready_path: &Path,
) -> bool {
    ready_path.is_file()
        && object_path.is_file()
        && artifact_path.is_file()
        && rock_lib::products::CompilerProducts::read_artifact_from_path(artifact_path).is_ok()
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
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

fn publish_stdlib_cache_dir(
    build_dir: &Path,
    artifact_dir: &Path,
    ready_path: &Path,
) -> Result<(), String> {
    remove_file_if_exists(ready_path).map_err(|err| {
        format!(
            "failed to remove stale stdlib cache ready marker {}: {}",
            ready_path.display(),
            err
        )
    })?;
    remove_dir_if_exists(artifact_dir).map_err(|err| {
        format!(
            "failed to remove stale stdlib cache directory {}: {}",
            artifact_dir.display(),
            err
        )
    })?;
    if artifact_dir.exists() {
        return Err(format!(
            "stdlib cache destination still exists before publish: {}",
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
                    "failed to clean stdlib cache publish directory {} after rename failed: {}; cleanup failed: {}",
                    publish_dir.display(),
                    rename_err,
                    cleanup_err
                )
            })?;
            copy_dir_all(build_dir, &publish_dir).map_err(|copy_err| {
                let cleanup = cleanup_error_suffix(remove_dir_if_exists(&publish_dir));
                format!(
                    "failed to copy stdlib cache from {} to {} after rename failed: {}; copy failed: {}{}",
                    build_dir.display(),
                    publish_dir.display(),
                    rename_err,
                    copy_err,
                    cleanup
                )
            })?;
            validate_stdlib_cache_payload(&publish_dir).map_err(|validate_err| {
                let cleanup = cleanup_error_suffix(remove_dir_if_exists(&publish_dir));
                format!(
                    "failed to validate stdlib cache publish directory {}: {}{}",
                    publish_dir.display(),
                    validate_err,
                    cleanup
                )
            })?;
            std::fs::rename(&publish_dir, artifact_dir).map_err(|publish_err| {
                let cleanup = cleanup_error_suffix(remove_dir_if_exists(&publish_dir));
                format!(
                    "failed to publish copied stdlib cache from {} to {} after rename failed: {}; publish failed: {}{}",
                    publish_dir.display(),
                    artifact_dir.display(),
                    rename_err,
                    publish_err,
                    cleanup
                )
            })?;
            remove_dir_if_exists(build_dir).map_err(|cleanup_err| {
                format!(
                    "failed to remove copied stdlib cache build directory {}: {}",
                    build_dir.display(),
                    cleanup_err
                )
            })?;
        }
    }

    validate_stdlib_cache_payload(artifact_dir)?;
    std::fs::write(ready_path, b"ready").map_err(|err| {
        format!(
            "failed to mark stdlib cache directory ready {}: {}",
            ready_path.display(),
            err
        )
    })
}

fn remove_file_if_exists(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

fn remove_dir_if_exists(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

fn cleanup_error_suffix(result: std::io::Result<()>) -> String {
    match result {
        Ok(()) => String::new(),
        Err(err) => format!("; cleanup failed: {err}"),
    }
}

fn validate_stdlib_cache_payload(dir: &Path) -> Result<(), String> {
    let artifact_path = dir.join("stdlib.rkca");
    let object_path = dir.join("stdlib.o");
    if !object_path.is_file() {
        return Err(format!(
            "stdlib cache object file is missing: {}",
            object_path.display()
        ));
    }
    rock_lib::products::CompilerProducts::read_artifact_from_path(&artifact_path).map(|_| ())
}

fn write_minimal_stdlib_artifact_for_cache_test(path: &Path) {
    rock_lib::products::CompilerProducts {
        crate_identity: rock_lib::products::ProductCrateIdentity::local("stdlib".to_string()),
        identity_table: rock_lib::products::ProductIdentityTable::default(),
        interface: rock_lib::products::ProductInterface::default(),
        bodies: rock_lib::products::ProductBodies::default(),
        link: rock_lib::products::ProductLinkData {
            object_path: Some(PathBuf::from("stdlib.o")),
            ..rock_lib::products::ProductLinkData::default()
        },
        dependencies: Vec::new(),
        source_fingerprint: rock_lib::products::ProductSourceFingerprint::default(),
        infix_precedence: std::collections::BTreeMap::new(),
        proc_macros: Vec::new(),
    }
    .write_artifact_to_path(path)
    .unwrap();
}

#[test]
fn stdlib_cache_key_changes_when_stdlib_source_changes() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    let stdlib_dir = dir.join("stdlib");
    std::fs::create_dir_all(&stdlib_dir).unwrap();
    std::fs::write(stdlib_dir.join("lib.rk"), "main = -> 0\n").unwrap();

    let first = stdlib_cache_key(&stdlib_dir, "compiler-stamp");
    let second = stdlib_cache_key(&stdlib_dir, "compiler-stamp");
    assert_eq!(first, second);

    std::fs::write(stdlib_dir.join("lib.rk"), "main = -> 1\n").unwrap();
    let changed = stdlib_cache_key(&stdlib_dir, "compiler-stamp");

    assert_ne!(first, changed);
    assert_ne!(changed, stdlib_cache_key(&stdlib_dir, "rebuilt-compiler"));

    std::fs::write(stdlib_dir.join("rock.toml"), "[crate]\nname = \"stdlib\"\n").unwrap();
    let with_manifest = stdlib_cache_key(&stdlib_dir, "compiler-stamp");
    assert_ne!(changed, with_manifest);
    std::fs::write(stdlib_dir.join("rock.toml"), "[crate]\nname = \"renamed\"\n").unwrap();
    assert_ne!(with_manifest, stdlib_cache_key(&stdlib_dir, "compiler-stamp"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn stdlib_cache_rejects_missing_ready_marker() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();
    write_minimal_stdlib_artifact_for_cache_test(&dir.join("stdlib.rkca"));
    std::fs::write(dir.join("stdlib.o"), b"object").unwrap();

    assert!(!stdlib_cache_entry_is_ready(
        &dir.join("stdlib.rkca"),
        &dir.join("stdlib.o"),
        &dir.join("ready")
    ));
}

#[test]
fn stdlib_cache_stale_lock_recovery_does_not_spin_forever() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();
    let artifact_dir = dir.join("cache");
    let lock_path = artifact_dir.with_extension("lock");
    std::fs::write(&lock_path, "pid=0\ncreated_unix_nanos=0\n").unwrap();

    assert!(recover_stale_cache_lock(&lock_path, Duration::from_secs(1)));
    assert!(!lock_path.exists());
}

#[test]
fn stdlib_cache_publish_replaces_stale_ready_entry_atomically() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    let artifact_dir = dir.join("cache");
    let build_dir = dir.join("build");
    let ready_path = artifact_dir.join("ready");
    std::fs::create_dir_all(&artifact_dir).unwrap();
    std::fs::create_dir_all(&build_dir).unwrap();
    std::fs::write(artifact_dir.join("ready"), b"ready").unwrap();
    std::fs::write(artifact_dir.join("stdlib.rkca"), b"invalid stale artifact").unwrap();
    std::fs::write(artifact_dir.join("stdlib.o"), b"stale object").unwrap();
    write_minimal_stdlib_artifact_for_cache_test(&build_dir.join("stdlib.rkca"));
    std::fs::write(build_dir.join("stdlib.o"), b"fresh object").unwrap();

    publish_stdlib_cache_dir(&build_dir, &artifact_dir, &ready_path)
        .expect("cache publish should replace stale ready entry");

    assert!(stdlib_cache_entry_is_ready(
        &artifact_dir.join("stdlib.rkca"),
        &artifact_dir.join("stdlib.o"),
        &ready_path,
    ));
    assert_eq!(
        std::fs::read(artifact_dir.join("stdlib.o")).unwrap(),
        b"fresh object"
    );
}

fn stdlib_artifact_path() -> PathBuf {
    static STDLIB_ARTIFACT: OnceLock<PathBuf> = OnceLock::new();

    STDLIB_ARTIFACT
        .get_or_init(|| {
            let stdlib_dir = stdlib_path();
            let cache_key = stdlib_cache_key(&stdlib_dir, &stdlib_cache_compiler_stamp());
            let artifact_dir = std::env::temp_dir().join(format!(
                "rock_integration_stdlib_product_artifact_{}",
                cache_key
            ));
            let artifact_path = artifact_dir.join("stdlib.rkca");
            let object_path = artifact_dir.join("stdlib.o");
            let ready_path = artifact_dir.join("ready");
            let lock_path = artifact_dir.with_extension("lock");
            if stdlib_cache_entry_is_ready(&artifact_path, &object_path, &ready_path) {
                return artifact_path;
            }

            let Some(_cache_lock) = acquire_cache_lock(&lock_path) else {
                return artifact_path;
            };
            if stdlib_cache_entry_is_ready(&artifact_path, &object_path, &ready_path) {
                return artifact_path;
            }

            let build_dir = artifact_dir.with_extension(format!("building-{}", std::process::id()));
            remove_dir_if_exists(&build_dir)
                .expect("Failed to clean stdlib product artifact build directory");
            std::fs::create_dir_all(&build_dir).unwrap();
            let build_artifact_path = build_dir.join("stdlib.rkca");
            let build_object_path = build_dir.join("stdlib.o");

            let output = rock_lib::compile_with_products(&rock_lib::Config {
                entry_file: stdlib_dir.join("lib.rk"),
                output_dir: build_dir.clone(),
                debug_print: vec![],
                meta_files: vec![],
                extern_artifacts: vec![],
                source_providers: Vec::new(),
                current_crate_name: Some("stdlib".to_string()),
                opt_level: 0,
                emit_llvm: false,
                no_link: true,
                emit_object: Some(build_object_path),
                no_prelude: false,
                no_std: true,
                sysroot: None,
            })
            .expect("Failed to compile stdlib product artifact for integration tests");
            let mut products = output
                .products
                .expect("stdlib compile should produce product data");
            products.link.object_path = Some(PathBuf::from("stdlib.o"));
            products
                .write_artifact_to_path(&build_artifact_path)
                .expect("Failed to write stdlib product artifact for integration tests");

            rock_lib::products::CompilerProducts::read_artifact_from_path(&build_artifact_path)
                .expect("Failed to validate stdlib product artifact for integration tests");

            publish_stdlib_cache_dir(&build_dir, &artifact_dir, &ready_path)
                .expect("Failed to publish stdlib product artifact cache");
            artifact_path
        })
        .clone()
}

fn test_config(entry_file: PathBuf, output_dir: PathBuf) -> rock_lib::Config {
    rock_lib::Config {
        entry_file,
        output_dir,
        debug_print: vec![],
        meta_files: vec![],
        extern_artifacts: vec![("stdlib".to_string(), stdlib_artifact_path())],
        source_providers: Vec::new(),
        current_crate_name: None,
        opt_level: 0,
        emit_llvm: false,
        no_link: false,
        emit_object: None,
        no_prelude: false,
        no_std: false,
        sysroot: None,
    }
}

struct TestDirCleanup(PathBuf);

impl Drop for TestDirCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn child_exited_without_reaping(child: &Child) -> Result<bool, std::io::Error> {
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

fn terminate_test_command_group(child: &Child) -> Result<(), std::io::Error> {
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

fn terminate_and_collect_test_command(
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

fn run_test_command(command: &mut Command) -> Output {
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

fn compile_and_run_without_stdlib(source: &str) -> i32 {
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
    rock_lib::compile(&config).expect("no-stdlib compilation failed");

    let mut command = Command::new(dir.join(format!("no_std_{id}")));
    run_test_command(&mut command).status.code().unwrap_or(1)
}

fn compile_and_run(source: &str) -> String {
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

fn compile_and_run_with_args(source: &str, args: &[&str]) -> String {
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

fn compile_and_run_with_status(source: &str) -> (String, bool) {
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

fn compile_and_run_in_working_dir(source: &str, dir: &Path) -> Output {
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

fn compile_example(name: &str) -> String {
    let examples_dir = workspace_root().join("examples");
    let source_path = examples_dir.join(format!("{}.rk", name));
    let source = std::fs::read_to_string(&source_path)
        .unwrap_or_else(|_| panic!("Failed to read example: {}", name));
    compile_and_run(&source)
}

#[test]
fn test_hello_world() {
    let output = compile_example("hello");
    assert_eq!(output.trim(), "Hello, World!");
}

#[test]
fn test_stdlib_program_args_are_owned_and_available_outside_main() {
    let output = compile_and_run_with_args(
        r#"
> stdlib::env::args

read_args = -> args!

main = ->
    first = read_args!
    second = read_args!
    first.len!.println!
    first[1].println!
    second[2].println!
    0
"#,
        &["alpha", "two words"],
    );

    assert_eq!(output, "3\nalpha\ntwo words\n");
}

#[test]
fn test_declarative_macro_expands_user_program() {
    let (_stdout, success) = compile_and_run_with_status(
        r#"macro make_main
    =>
        main = -> 0
%make_main"#,
    );

    assert!(success);
}

#[test]
fn test_declarative_macro_can_produce_call_argument_holes() {
    let output = compile_and_run(
        r#"macro make_main
    =>
        combine = a, b, c -> a * 100 + b * 10 + c
        main = ->
            section = combine 1, _, 3
            section 2 .println!
            0
%make_main"#,
    );

    assert_eq!(output.trim(), "123");
}

#[test]
fn test_arithmetic() {
    let output = compile_example("arithmetic");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "30");
    assert_eq!(lines[1], "205");
}

#[test]
fn test_operator_resolution_does_not_depend_on_stdlib_trait_names() {
    let output = compile_and_run(
        r#"< struct Boxed
    < value: I64

< trait Combine Rhs
    @+: Rhs -> I64

impl Combine Boxed for Boxed
    @+ = other -> self.value + other.value

main = ->
    lhs = Boxed
        value: 40
    rhs = Boxed
        value: 2
    result = lhs + rhs
    result.println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_primitive_operator_semantics_come_from_user_defined_operator() {
    let output = compile_and_run(
        r#"+ = left, right -> ~I64Sub left, right

main = ->
    40 + 2 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "38");
}

#[test]
fn test_unsafe_operator_function_requires_unsafe() {
    compile_should_fail(
        r#"unsafe + = left, right -> ~I64Add left, right

main = ->
    value = 40 + 2
    value.println!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_ampersand_function_reference_call_requires_unsafe() {
    compile_should_fail(
        r#"unsafe read_ref: &I64 -> I64
read_ref = x -> *x

main = ->
    value = 7
    result = read_ref & value
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_operator_method_requires_unsafe() {
    compile_should_fail(
        r#"< struct UnsafeAdder
    < value: I64

< trait Combine Rhs
    type Output
    @+: Rhs -> Self::Output

impl Combine I64 for UnsafeAdder
    type Output = I64

    unsafe @+ = other -> self.value + other

main = ->
    lhs = UnsafeAdder
        value: 40
    value = lhs + 2
    value.println!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_method_value_requires_unsafe() {
    compile_should_fail(
        r#"struct Secret
    < value: I64

impl Secret
    unsafe @read = -> @value

main = ->
    secret = Secret
        value: 7
    read = secret.read
    value = read!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_method_value_created_in_unsafe_still_requires_unsafe_call() {
    compile_should_fail(
        r#"struct Secret
    < value: I64

impl Secret
    unsafe @read = -> @value

main = ->
    secret = Secret
        value: 7
    read = unsafe secret.read
    value = read!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_function_alias_requires_unsafe() {
    compile_should_fail(
        r#"unsafe read_secret: I64 -> I64
read_secret = value -> value

main = ->
    alias = read_secret
    value = alias 7
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_function_alias_ampersand_call_requires_unsafe() {
    compile_should_fail(
        r#"unsafe read_ref: &I64 -> I64
read_ref = value -> *value

main = ->
    value = 7
    alias = read_ref
    result = alias & value
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_callable_cannot_flow_to_safe_callable_slot() {
    compile_should_fail(
        r#"unsafe read_secret: I64
read_secret = -> 7

apply: (() -> I64) -> I64
apply = reader -> reader!

main = ->
    value = apply read_secret
    0
"#,
        "unsafe",
    );
}

#[test]
fn test_unsafe_function_returned_callable_requires_unsafe() {
    compile_should_fail(
        r#"unsafe read_secret: I64 -> I64
read_secret = value -> value

make_reader = -> unsafe read_secret

main = ->
    reader = make_reader!
    value = reader 7
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_method_returned_callable_requires_unsafe() {
    compile_should_fail(
        r#"unsafe read_secret: I64
read_secret = -> 7

struct Factory

impl Factory
    @make_reader = -> unsafe read_secret

main = ->
    factory = Factory
    reader = factory.make_reader!
    value = reader!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_if_returned_callable_requires_unsafe() {
    compile_should_fail(
        r#"unsafe read_secret: I64
read_secret = -> 7

safe_reader: I64
safe_reader = -> 0

make_reader = flag ->
    if flag
        unsafe read_secret
    else
        safe_reader

main = ->
    reader = make_reader true
    value = reader!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_if_returned_callable_safe_then_unsafe_requires_unsafe() {
    compile_should_fail(
        r#"unsafe read_secret: I64
read_secret = -> 7

safe_reader: I64
safe_reader = -> 0

make_reader = flag ->
    if flag
        safe_reader
    else
        unsafe read_secret

main = ->
    reader = make_reader true
    value = reader!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_match_returned_callable_safe_then_unsafe_requires_unsafe() {
    compile_should_fail(
        r#"unsafe read_secret: I64
read_secret = -> 7

safe_reader: I64
safe_reader = -> 0

make_reader = flag ->
    match flag
        true => safe_reader
        false => unsafe read_secret

main = ->
    reader = make_reader true
    value = reader!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_if_tuple_callable_safe_then_unsafe_requires_unsafe() {
    compile_should_fail(
        r#"unsafe read_secret: I64
read_secret = -> 7

safe_reader: I64
safe_reader = -> 0

make_pair = flag ->
    if flag
        (safe_reader, 0)
    else
        (unsafe read_secret, 1)

main = ->
    pair = make_pair true
    reader = pair.0
    value = reader!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_match_tuple_callable_safe_then_unsafe_requires_unsafe() {
    compile_should_fail(
        r#"unsafe read_secret: I64
read_secret = -> 7

safe_reader: I64
safe_reader = -> 0

make_pair = flag ->
    match flag
        true => (safe_reader, 0)
        false => (unsafe read_secret, 1)

main = ->
    pair = make_pair true
    reader = pair.0
    value = reader!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_callable_tuple_projection_requires_unsafe() {
    compile_should_fail(
        r#"unsafe read_secret: I64
read_secret = -> 7

safe_reader: I64
safe_reader = -> 0

make_reader = -> unsafe read_secret

main = ->
    pair = (unsafe make_reader!, safe_reader)
    reader = pair.0
    value = reader!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_callable_struct_field_requires_unsafe() {
    compile_should_fail(
        r#"unsafe read_secret: I64
read_secret = -> 7

struct Holder
    < reader: () -> I64

make_reader = -> unsafe read_secret

main = ->
    holder = Holder
        reader: unsafe make_reader!
    value = holder.reader!
    0
"#,
        "unsafe",
    );
}

#[test]
fn test_unsafe_callable_reassignment_requires_unsafe() {
    compile_should_fail(
        r#"unsafe read_secret: I64
read_secret = -> 7

safe_reader: I64
safe_reader = -> 0

make_reader = -> unsafe read_secret

main = ->
    mut reader = safe_reader
    reader = unsafe make_reader!
    value = reader!
    0
"#,
        "unsafe",
    );
}

#[test]
fn test_unsafe_method_to_method_returned_callable_requires_unsafe() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();
    let source_path = dir.join("test.rk");
    std::fs::write(
        &source_path,
        r#"unsafe read_secret: I64
read_secret = -> 7

struct Factory

impl Factory
    @make_reader = -> unsafe read_secret
    @forward_reader = -> self.make_reader!

main = ->
    factory = Factory
    reader = factory.forward_reader!
    value = reader!
    0
"#,
    )
    .unwrap();

    let mut config = test_config(source_path, dir.clone());
    config.no_std = true;
    config.no_prelude = true;
    config.extern_artifacts.clear();
    let result = rock_lib::compile(&config);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        result.is_err(),
        "expected method-to-method unsafe callable laundering to be rejected"
    );
}

#[test]
fn test_unsafe_nested_method_laundering_requires_unsafe() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();
    let source_path = dir.join("test.rk");
    std::fs::write(
        &source_path,
        r#"unsafe read_secret: I64
read_secret = -> 7

safe_reader: I64
safe_reader = -> 0

struct Factory

impl Factory
    @use_reader = ->
        reader = if true
            self.forward_reader!
        else
            safe_reader
        if true
            reader!
        else
            0

    @forward_reader = -> self.make_reader!
    @make_reader = -> unsafe read_secret

main = ->
    factory = Factory
    value = factory.use_reader!
    0
"#,
    )
    .unwrap();

    let mut config = test_config(source_path, dir.clone());
    config.no_std = true;
    config.no_prelude = true;
    config.extern_artifacts.clear();
    let result = rock_lib::compile(&config);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        result.is_err(),
        "expected nested unsafe callable laundering to be rejected"
    );
}

#[test]
fn test_generic_function_alias_preserves_trait_bounds() {
    compile_should_fail(
        r#"trait Marker

requires_marker: T -> () where T: Marker
requires_marker = _ -> return

main = ->
    alias = requires_marker
    alias 1
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_generic_operator_function_preserves_trait_bounds() {
    compile_should_fail(
        r#"infix 9 %%

trait Marker

%%: T -> T -> T where T: Marker
%% = left, _ -> left

main = ->
    value = 1 %% 2
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_qualified_generic_function_value_preserves_trait_bounds() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(
        dir.join("utils.rk"),
        r#"trait Marker

requires_marker: T -> () where T: Marker
requires_marker = _ -> return

< Marker
< requires_marker
"#,
    )
    .unwrap();

    std::fs::write(
        dir.join("test.rk"),
        r#"mod utils

main = ->
    alias = utils::requires_marker
    alias 1
    0
"#,
    )
    .unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());
    let result = rock_lib::compile(&config);
    let _ = std::fs::remove_dir_all(&dir);
    let diagnostics = result.expect_err("expected compilation to fail");
    let messages: Vec<String> = diagnostics.0.iter().map(|d| d.message.clone()).collect();
    assert!(
        messages
            .iter()
            .any(|message| message.contains("does not implement trait")),
        "expected trait-bound failure, got: {messages:?}"
    );
}

#[test]
fn test_generic_method_call_preserves_method_bounds() {
    compile_should_fail(
        r#"trait Marker

struct Box

impl Box
    @requires_marker: T -> () where T: Marker
    @requires_marker = _ -> return

main = ->
    box = Box
    box.requires_marker 1
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_generic_method_value_preserves_method_bounds() {
    compile_should_fail(
        r#"trait Marker

struct Box

impl Box
    @requires_marker: T -> () where T: Marker
    @requires_marker = _ -> return

main = ->
    box = Box
    requires_marker = box.requires_marker
    requires_marker 1
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_trait_bound_method_signature_preserves_method_bounds() {
    compile_should_fail(
        r#"trait Marker

trait Provider
    @requires_marker: T -> () where T: Marker

struct Service

impl Provider for Service
    @requires_marker = _ -> return

call_requires: P -> () where P: Provider
call_requires = provider -> provider.requires_marker 1

main = ->
    service = Service
    call_requires service
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_generic_trait_bound_solver_checks_impl_bounds() {
    compile_should_fail(
        r#"trait Marker

trait Provider

struct Box T
    < value: T

impl Provider for Box T where T: Marker

requires_provider: T -> () where T: Provider
requires_provider = _ -> return

main = ->
    boxed = Box
        value: 7
    requires_provider boxed
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_primitive_operator_accepts_inferred_rhs_type() {
    let output = compile_and_run(
        r#"main = ->
    values = [2]
    result = 40 + values[0]
    result.println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_functions() {
    let output = compile_example("functions");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "7");
    assert_eq!(lines[1], "25");
    assert_eq!(lines[2], "25");
}

#[test]
fn test_if_else() {
    let output = compile_example("if_else");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "5");
    assert_eq!(lines[1], "10");
    assert_eq!(lines[2], "42");
}

#[test]
fn test_structs() {
    let output = compile_example("structs");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "20");
    assert_eq!(lines[2], "30");
}

#[test]
fn test_while_loop() {
    let output = compile_example("while_loop");
    let lines: Vec<&str> = output.trim().lines().collect();
    for i in 0..10 {
        assert_eq!(lines[i], i.to_string());
    }
    assert_eq!(lines[10], "done");
}

#[test]
fn test_for_loop() {
    let output = compile_example("for_loop");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "20");
    assert_eq!(lines[2], "30");
    assert_eq!(lines[3], "40");
    assert_eq!(lines[4], "50");
    assert_eq!(lines[5], "done");
}

#[test]
fn test_recursion() {
    let output = compile_example("recursion");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "3628800"); // 10!
    assert_eq!(lines[1], "55"); // fib(10)
}

#[test]
fn test_match_expr() {
    let output = compile_example("match_expr");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "0");
    assert_eq!(lines[1], "10");
    assert_eq!(lines[2], "20");
    assert_eq!(lines[3], "99");
}

#[test]
fn test_enums() {
    let output = compile_example("enums");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "enum test");
    assert_eq!(lines[1], "42");
}

#[test]
fn test_enum_match() {
    let output = compile_example("enum_match");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "2");
    assert_eq!(lines[2], "3");
}

#[test]
fn test_enum_string_payload_pattern_reports_diagnostic() {
    compile_should_fail(
        r#"
enum Token
    Text &Str
    Other

main = ->
    token = Token::Text "x"
    match token
        Token::Text "x" => 1
        _ => 0
"#,
        "unsupported enum payload literal pattern",
    );
}

#[test]
fn test_impl_method_enum_payload_pattern_reports_diagnostic() {
    compile_should_fail(
        r#"
enum Token
    Text &Str
    Other

struct Wrapper
    < token: Token

impl Wrapper
    @check = ->
        match @token
            Token::Text "x" => 1
            _ => 0

main = ->
    wrapper = Wrapper
        token: Token::Text "x"
    wrapper.check!
"#,
        "unsupported enum payload literal pattern",
    );
}

#[test]
fn test_trait_default_enum_payload_pattern_reports_diagnostic() {
    compile_should_fail(
        r#"
enum Token
    Text &Str
    Other

struct Wrapper

trait ChecksToken
    @check: Token -> I64
    @check = token ->
        match token
            Token::Text "x" => 1
            _ => 0

impl ChecksToken for Wrapper

main = ->
    wrapper = Wrapper
    wrapper.check (Token::Text "x")
"#,
        "unsupported enum payload literal pattern",
    );
}

#[test]
fn test_guarded_noncopy_enum_payload_binding_reports_diagnostic() {
    compile_should_fail(
        r#"
struct Boxed
    < value: I64

enum MaybeBoxed
    Some Boxed
    None

main = ->
    item = MaybeBoxed::Some (Boxed
        value: 1)
    match item
        MaybeBoxed::Some value if 1 == 1 => value.value
        _ => 0
"#,
        "unsupported guarded non-copy enum payload binding",
    );
}

#[test]
fn test_nested_control_flow() {
    let output = compile_example("nested");
    let lines: Vec<&str> = output.trim().lines().collect();
    // fizzbuzz for 1-20
    let expected = [
        "3", "3", "1", "3", "2", "1", "3", "3", "1", "2", "3", "1", "3", "3", "0", "3", "3", "1",
        "3", "2",
    ];
    for (i, exp) in expected.iter().enumerate() {
        assert_eq!(lines[i], *exp, "Mismatch at position {}", i);
    }
}

#[test]
fn test_string_literal_indexing_is_rejected() {
    compile_should_fail(
        r#"
main = ->
    s = "abc"
    s[1] .println!
    0
"#,
        "cannot index Str by integer",
    );
}

#[test]
fn test_duplicate_generic_parameter_names_are_rejected() {
    compile_should_fail(
        r#"
struct Box T, T
    value: T

main = ->
    0
"#,
        "duplicate generic parameter 'T'",
    );
}

#[test]
fn test_borrowed_u8_slice_struct_field_indexes_as_byte() {
    let output = compile_and_run(
        r#"
struct Holder
    < data: &[U8]

second: &[U8] -> U8
second = s -> s[1]

main = ->
    bytes = [97 as U8, 98 as U8, 99 as U8]
    holder = Holder
        data: &bytes
    slice = holder.data
    (second slice) as I64 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "98");
}

#[test]
fn test_bare_slice_struct_field_is_rejected() {
    compile_should_fail(
        r#"
struct Holder
    < data: [U8]

main = -> 0
"#,
        "bare slice type [T] must be written behind a reference",
    );
}

#[test]
fn test_array_literal_keeps_fixed_array_type_until_coercion() {
    let output = compile_and_run(
        r#"
show_slice: &[I64] -> String
show_slice = s -> s.show!

main = ->
    arr = [1, 2, 3]
    show_slice &arr .println!
    0
"#,
    );

    assert_eq!(output.trim(), "[1, 2, 3]");
}

#[test]
fn test_show_impl_for_slice_resolves_through_builtin_receiver_type() {
    let output = compile_and_run(
        r#"
main = ->
    s = "abc"
    s.show!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "abc");
}

#[test]
fn test_array_is_available_as_user_defined_type_name() {
    let output = compile_and_run(
        r#"
struct Array
    < value: I64

main = ->
    a = Array
        value: 41
    a.value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "41");
}

#[test]
fn test_bare_slice_impl_target_is_rejected() {
    compile_should_fail(
        r#"
trait SliceLen
    @slice_len = -> 0

impl SliceLen for [T]
    @slice_len = -> ~ArrayLen self

main = -> 0
"#,
        "bare slice type [T] must be written behind a reference",
    );
}

#[test]
fn test_bare_str_parameter_is_rejected() {
    compile_should_fail(
        r#"
len: Str -> I64
len = s -> ~ArrayLen s

main = -> 0
"#,
        "bare string slice type Str must be written behind a reference",
    );
}

#[test]
fn test_borrowed_slice_impl_target_dispatches() {
    let output = compile_and_run(
        r#"
trait SliceLen
    @slice_len = -> 0

impl SliceLen for &[T]
    @slice_len = -> ~ArrayLen (*self)

main = ->
    arr = [1, 2, 3]
    (&arr).slice_len!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "3");
}

#[test]
fn test_str_trait_impl_uses_borrowed_str_not_u8_slice() {
    let output = compile_and_run(
        r#"
trait Kind
    @kind = -> 0

impl Kind for &Str
    @kind = -> 7

impl Kind for &[U8]
    @kind = -> 3

main = ->
    s = "abc"
    s.kind!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_u8_slice_trait_impl_does_not_apply_to_string_literal() {
    compile_should_fail(
        r#"
trait BytesOnly
    @bytes_only = -> 0

impl BytesOnly for &[U8]
    @bytes_only = -> 1

main = ->
    s = "abc"
    s.bytes_only!.println!
    0
"#,
        "bytes_only",
    );
}

#[test]
fn test_user_defined_array_does_not_receive_slice_methods() {
    compile_should_fail(
        r#"
struct Array
    < value: I64

main = ->
    a = Array
        value: 1
    a.show!.println!
    0
"#,
        "Unknown field 'show' on struct 'Array'",
    );
}

#[test]
fn test_fixed_array_value_does_not_coerce_to_slice_parameter() {
    compile_should_fail(
        r#"
sum: &[I64] -> I64
sum = s ->
    (s[0] + s[1]) + s[2]

main = ->
    arr = [1, 2, 3]
    sum arr .println!
    0
"#,
        "Type mismatch",
    );
}

#[test]
fn test_fixed_array_value_does_not_coerce_to_slice_return() {
    compile_should_fail(
        r#"
make: () -> &[I64]
make = -> [1, 2, 3]

main = ->
    0
"#,
        "Type mismatch",
    );
}

#[test]
fn test_overapplied_non_unary_call_is_rejected() {
    compile_should_fail(
        r#"
add = x, y -> x + y

main = ->
    add 1, 2, 3 .println!
    0
"#,
        "Type mismatch",
    );
}

#[test]
fn test_fixed_array_borrow_coerces_to_slice_parameter() {
    let output = compile_and_run(
        r#"
sum: &[I64] -> I64
sum = s ->
    (s[0] + s[1]) + s[2]

main = ->
    arr = [1, 2, 3]
    sum &arr .println!
    0
"#,
    );

    assert_eq!(output.trim(), "6");
}

#[test]
fn test_array_literal_cast_seed_infers_remaining_integer_elements() {
    let output = compile_and_run(
        r#"
first_byte: &[U8] -> I64
first_byte = s -> s[0] as I64

main = ->
    buf = [7 as U8, 8, 9]
    first_byte &buf .println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_array_literal_binding_annotation_infers_integer_elements() {
    let output = compile_and_run(
        r#"
main = ->
    buf: [U8; 5] = [1, 2, 3, 4, 5]
    buf[4] as I64 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "5");
}

#[test]
fn test_repeat_array_initializer_is_evaluated_once() {
    let output = compile_and_run(
        r#"
next_byte: &mut I64 -> U8
next_byte = calls ->
    *calls = *calls + 1
    7 as U8

main = ->
    mut calls = 0
    buf: [U8; 256] = [next_byte (&mut calls); 256]
    buf[255] as I64 .println!
    calls.println!
    0
"#,
    );

    assert_eq!(output.trim().lines().collect::<Vec<_>>(), vec!["7", "1"]);
}

#[test]
fn test_repeat_array_binding_annotation_infers_integer_element() {
    let output = compile_and_run(
        r#"
main = ->
    buf: [U8; 256] = [0; 256]
    buf[255] as I64 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "0");
}

#[test]
fn test_repeat_array_rejects_non_copy_initializer() {
    compile_should_fail(
        r#"
main = ->
    values = [String::from_str "owned"; 2]
    values
    0
"#,
        "array repeat initializer must be Copy",
    );
}

#[test]
fn test_array_literal_mut_slice_use_infers_integer_elements() {
    let output = compile_and_run(
        r#"
write_first: &mut [U8] -> I64
write_first = s ->
    ptr = (~ArrPtr (*s)) as *U8
    unsafe *ptr = 42 as U8
    unsafe *ptr as I64

main = ->
    mut buf = [0, 0, 0, 0, 0]
    slice = &mut buf
    write_first slice .println!
    buf[0] as I64 .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["42", "42"]);
}

#[test]
fn test_array_literal_mut_slice_use_infers_integer_expressions() {
    let output = compile_and_run(
        r#"
first_byte: &mut [U8] -> I64
first_byte = s ->
    ptr = (~ArrPtr (*s)) as *U8
    unsafe *ptr as I64

main = ->
    value = 40
    mut buf = [value + 2, 0, 0]
    slice = &mut buf
    first_byte slice .println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_array_integer_literal_does_not_infer_as_bool() {
    compile_should_fail(
        r#"
main = ->
    buf: [Bool; 1] = [1]
    0
"#,
        "integer literal resolved to non-integer type `Bool`",
    );
}

#[test]
fn test_fixed_array_borrow_coercion_preserves_original_storage_alias() {
    let output = compile_and_run(
        r#"
slice_addr: &[I64] -> I64
slice_addr = s -> (~ArrPtr (*s)) as I64

first_elem_addr: &[I64] -> I64
first_elem_addr = s -> (&((*s)[0]) as *I64) as I64

main = ->
    arr = [1, 2, 3]
    base = slice_addr &arr
    first = first_elem_addr &arr
    if base == first
        1.println!
    else
        0.println!
    0
"#,
    );

    assert_eq!(output.trim(), "1");
}

#[test]
fn test_arr_ptr_rejects_fixed_array_values() {
    compile_should_fail(
        r#"
main = ->
    arr = [1, 2, 3]
    ptr = ~ArrPtr arr
    unsafe *ptr .println!
    0
"#,
        "ArrPtr expected slice",
    );
}

#[test]
fn test_fixed_array_show_uses_slice_impl_via_coercion() {
    let output = compile_and_run(
        r#"
main = ->
    arr = [1, 2, 3]
    arr.show!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "[1, 2, 3]");
}

#[test]
fn test_u8_buffer_show_prints_byte_values() {
    let output = compile_and_run(
        r#"
main = ->
    buf = [104 as U8, 101 as U8, 108 as U8]
    buf.println!
    0
"#,
    );

    assert_eq!(output.trim(), "[104, 101, 108]");
}

#[test]
fn test_explicit_fixed_array_trait_impl_dispatches_at_call_site() {
    let output = compile_and_run(
        r#"
trait FixedArrayCode
    @code = -> 0

impl FixedArrayCode for [I64; 3]
    @code = -> 33

main = ->
    arr: [I64; 3] = [1, 2, 3]
    arr.code!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "33");
}

#[test]
fn test_fixed_array_impl_body_can_use_self_as_array() {
    let output = compile_and_run(
        r#"
trait FixedArraySecond
    @second = -> 0

impl FixedArraySecond for [I64; 3]
    @second = -> self[1]

main = ->
    arr: [I64; 3] = [7, 8, 9]
    arr.second!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "8");
}

#[test]
fn test_generic_fixed_array_impl_method_uses_receiver_type_substitution() {
    let output = compile_and_run(
        r#"
trait EchoArray
    @echo = x -> x

impl EchoArray for [T; 3]
    @echo = x -> x

main = ->
    arr: [I64; 3] = [7, 8, 9]
    arr.echo 7 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_fixed_array_trait_bound_solver_finds_concrete_impl() {
    let output = compile_and_run(
        r#"
trait FixedArrayCode
    @code = -> 0

impl FixedArrayCode for [I64; 3]
    @code = -> 33

use_code: T -> I64 where T: FixedArrayCode
use_code = value -> value.code!

main = ->
    arr: [I64; 3] = [1, 2, 3]
    use_code arr .println!
    0
"#,
    );

    assert_eq!(output.trim(), "33");
}

#[test]
fn test_explicit_fixed_array_index_assignment_updates_storage() {
    let output = compile_and_run(
        r#"
main = ->
    mut arr: [I64; 3] = [1, 2, 3]
    arr[1] = 9
    arr[1].println!
    0
"#,
    );

    assert_eq!(output.trim(), "9");
}

#[test]
fn nested_index_mut_updates_matrix_element() {
    let output = compile_and_run(
        r#"
> stdlib::vec::Vec

main = ->
    mut first = Vec::new!
    first.push 1
    first.push 2
    mut second = Vec::new!
    second.push 3
    second.push 4
    mut matrix = Vec::new!
    matrix.push first
    matrix.push second
    matrix[1][0] = 9
    matrix[1][0].println!
    0
"#,
    );

    assert_eq!(output.trim(), "9");
}

#[test]
fn index_mut_then_field_updates_record() {
    let output = compile_and_run(
        r#"
struct Record
    < field: I64

main = ->
    mut records: [Record; 2] = [Record field: 1, Record field: 2]
    records[1].field = 9
    records[1].field.println!
    0
"#,
    );

    assert_eq!(output.trim(), "9");
}

#[test]
fn indexed_assignment_rhs_uses_index() {
    let output = compile_and_run(
        r#"
struct Cell
    < read_value: I64
    < write_value: I64

impl Index I64 for Cell
    type Output = I64
    @index = _ -> &@read_value

impl IndexMut I64 for Cell
    type Output = I64
    ^@index_mut = _ -> &mut @write_value

main = ->
    mut target = Cell
        read_value: 1
        write_value: 0
    source = Cell
        read_value: 7
        write_value: 2
    target[0] = source[0]
    target.write_value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn mutable_index_reference_uses_index_mut_authority() {
    let output = compile_and_run(
        r#"
struct Cell
    < read_value: I64
    < write_value: I64

impl Index I64 for Cell
    type Output = I64
    @index = _ -> &@read_value

impl IndexMut I64 for Cell
    type Output = I64
    ^@index_mut = _ -> &mut @write_value

main = ->
    mut target = Cell
        read_value: 1
        write_value: 2
    mutable = &mut target[0]
    *mutable = 9
    target.read_value.println!
    target.write_value.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["1", "9"]);
}

#[test]
fn shared_index_reference_uses_index_authority() {
    let output = compile_and_run(
        r#"
main = ->
    mut arr: [I64; 2] = [1, 2]
    shared = &arr[0]
    *shared .println!
    0
"#,
    );

    assert_eq!(output.trim(), "1");
}

#[test]
fn mutable_index_reference_preserves_distinct_index_aliases() {
    let output = compile_and_run(
        r#"
struct Cell
    < read_value: I64
    < write_value: I64

impl Index I64 for Cell
    type Output = I64
    @index = _ -> &@read_value

impl IndexMut I64 for Cell
    type Output = I64
    ^@index_mut = _ -> &mut @write_value

main = ->
    mut target = Cell
        read_value: 1
        write_value: 2
    mutable = &mut target[0]
    *mutable = 9
    shared = &target[0]
    *shared .println!
    target.write_value.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["1", "9"]);
}

#[test]
fn shared_index_borrow_blocks_index_mut_assignment() {
    compile_should_fail(
        r#"
main = ->
    mut arr: [I64; 2] = [1, 2]
    shared: &I64 = &arr[0]
    arr[0] = 9
    value = *shared
    value.println!
    0
"#,
        "borrow conflict on",
    );
}

#[test]
fn shared_range_borrow_blocks_overlapping_mutable_range() {
    compile_should_fail(
        r#"
main = ->
    mut values = [10, 20, 30, 40]
    shared: &[I64] = &values[..2]
    mut overlapping: &mut [I64] = &mut values[1..]
    overlapping[0] = 99
    shared[0].println!
    0
"#,
        "borrow conflict on",
    );
}

#[test]
fn mutable_index_reference_preserves_alias_rules() {
    compile_should_fail(
        r#"
main = ->
    mut arr: [I64; 2] = [1, 2]
    mutable = &mut arr[0]
    shared = &arr[0]
    *mutable .println!
    *shared .println!
    0
"#,
        "borrow conflict on",
    );
}

#[test]
fn aggregate_mut_reference_blocks_shared_access() {
    compile_should_fail(
        r#"
struct Holder
    < reference: &mut I64

main = ->
    mut x = 1
    holder = Holder
        reference: &mut x
    shared = &x
    *holder.reference .println!
    *shared .println!
    0
"#,
        "borrow conflict on",
    );
}

#[test]
fn aggregate_mut_reference_blocks_mutable_access() {
    compile_should_fail(
        r#"
struct Holder
    < reference: &mut I64

main = ->
    mut x = 1
    holder = Holder
        reference: &mut x
    other = &mut x
    *holder.reference .println!
    *other .println!
    0
"#,
        "borrow conflict on",
    );
}

#[test]
fn value_index_reads_release_before_later_mutation() {
    let output = compile_and_run(
        r#"
main = ->
    mut arr: [I64; 2] = [2, 1]
    if arr[0] > arr[1] then
        arr[0] = arr[1]
    arr[0] .println!
    0
"#,
    );

    assert_eq!(output.trim(), "1");
}

#[test]
fn test_array_read_without_index_provider_fails_before_mono() {
    compile_should_fail_with_exact_diagnostic(
        r#"
main = ->
    arr: [I64; 3] = [1, 2, 3]
    arr[0]
    0
"#,
        "Cannot use indexing because the Index language-item protocol is unavailable",
    );
}

#[test]
fn test_array_write_without_index_mut_provider_fails_before_mono() {
    compile_should_fail_with_exact_diagnostic(
        r#"
main = ->
    arr: [I64; 3] = [1, 2, 3]
    arr[0] = 4
    0
"#,
        "Cannot use mutable indexing because the IndexMut language-item protocol is unavailable",
    );
}

#[test]
fn test_slice_read_without_index_provider_fails_before_mono() {
    compile_should_fail_with_exact_diagnostic(
        r#"
read: &[I64] -> I64
read = slice -> slice[0]

main = -> 0
"#,
        "Cannot use indexing because the Index language-item protocol is unavailable",
    );
}

#[test]
fn test_slice_write_without_index_mut_provider_fails_before_mono() {
    compile_should_fail_with_exact_diagnostic(
        r#"
write: &mut [I64] -> ()
write = slice -> slice[0] = 4

main = -> 0
"#,
        "Cannot use mutable indexing because the IndexMut language-item protocol is unavailable",
    );
}

#[test]
fn test_pointer_read_without_index_provider_fails_before_mono() {
    compile_should_fail_with_exact_diagnostic(
        r#"
read: *I64 -> I64
read = ptr -> unsafe ptr[0]

main = -> 0
"#,
        "Cannot use indexing because the Index language-item protocol is unavailable",
    );
}

#[test]
fn test_pointer_write_without_index_mut_provider_fails_before_mono() {
    compile_should_fail_with_exact_diagnostic(
        r#"
write: *I64 -> ()
write = ptr -> unsafe ptr[0] = 4

main = -> 0
"#,
        "Cannot use mutable indexing because the IndexMut language-item protocol is unavailable",
    );
}

#[test]
fn test_foreign_index_impl_for_structural_array_is_rejected() {
    compile_should_fail(
        r#"
impl Index I64 for [Bool; 3]
    type Output = Bool
    @index = i ->
        ptr: *Bool = self as *Bool
        unsafe &(ptr[2 - i])

main = ->
    arr: [Bool; 3] = [false, false, true]
    arr[0].show!.println!
    p = &arr[0]
    *p .println!
    0
"#,
        "orphan impl",
    );
}

#[test]
fn test_generic_use_of_foreign_index_impl_for_structural_array_is_rejected() {
    compile_should_fail(
        r#"
impl Index I64 for [Bool; 3]
    type Output = Bool
    @index = i ->
        ptr: *Bool = self as *Bool
        unsafe &(ptr[2 - i])

is_selected_at_two = value ->
    if value[2]
        1
    else
        0

main = ->
    arr: [Bool; 3] = [true, false, false]
    is_selected_at_two arr .println!
    0
"#,
        "orphan impl",
    );
}

#[test]
fn test_repeated_receiver_generic_impl_does_not_match_incompatible_receiver_args() {
    compile_should_fail(
        r#"
trait SameCode
    @code = -> 0

struct Pair A, B
    < left: A
    < right: B

impl SameCode for Pair T, T
    @code = -> 99

main = ->
    pair = Pair I64, Bool
        left: 1
        right: true
    pair.code!.println!
    0
"#,
        "Unknown field 'code' on struct 'Pair'",
    );
}

#[test]
fn test_explicit_generic_slice_impl_uses_self_body_and_dispatches() {
    let output = compile_and_run(
        r#"
trait SliceLen
    @slice_len = -> 0

impl SliceLen for &[T]
    @slice_len = ->
        ~ArrayLen (*self)

main = ->
    arr = [97 as U8, 98 as U8, 99 as U8]
    (&arr).slice_len!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "3");
}

#[test]
fn test_explicit_generic_slice_impl_method_as_value_falls_back_from_u8_slice() {
    let output = compile_and_run(
        r#"
trait SliceLen
    @slice_len = -> 0

impl SliceLen for &[T]
    @slice_len = ->
        ~ArrayLen (*self)

main = ->
    arr = [97 as U8, 98 as U8, 99 as U8]
    f = (&arr).slice_len
    f!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "3");
}

#[test]
fn test_mut_ref_assign() {
    let output = compile_example("mir_tests/mut_ref_assign");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "2");
}

#[test]
fn test_stdlib_math() {
    let output = compile_and_run(
        r#"
main = ->
    stdlib::libc::sqrt 144.0 .println!
    stdlib::libc::sin 0.0 .println!
    stdlib::libc::cos 0.0 .println!
    stdlib::libc::pow 2.0, 10.0 .println!
    stdlib::libc::ceil 3.2 .println!
    stdlib::libc::floor 3.8 .println!
    string_len "hello" .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "12"); // sqrt(144)
    assert_eq!(lines[1], "0"); // sin(0)
    assert_eq!(lines[2], "1"); // cos(0)
    assert_eq!(lines[3], "1024"); // pow(2,10)
    assert_eq!(lines[4], "4"); // ceil(3.2)
    assert_eq!(lines[5], "3"); // floor(3.8)
    assert_eq!(lines[6], "5"); // strlen("hello")
}

#[test]
fn test_stdlib_prelude_extern_is_available_without_explicit_import() {
    let output = compile_and_run(
        r#"
main = ->
    sqrt 144.0 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "12");
}

#[test]
fn test_socket_posix_ffi_surface_compiles_and_returns_errno() {
    let output = compile_and_run(
        r#"
> stdlib::libc::socket
> stdlib::libc::close
> stdlib::libc::__errno_location

last_errno = ->
    ptr = __errno_location!
    unsafe *ptr
main = ->
    fd = socket (-1), 1, 0
    if fd < (0 as I32)
        last_errno! .println!
    else
        close fd .println!
    0
"#,
    );

    let errno = output
        .trim()
        .parse::<i64>()
        .expect("errno should print as integer");
    assert!(
        errno > 0,
        "invalid socket domain should set errno, got {errno}"
    );
}

#[test]
fn test_socket_addr_v4_constructors() {
    let output = compile_and_run(
        r#"
> stdlib::net::Ipv4Addr
> stdlib::net::SocketAddrV4

main = ->
    local = Ipv4Addr::localhost!
    any = Ipv4Addr::any!
    custom = Ipv4Addr::new 1, 2, 3, 4
    addr = SocketAddrV4::localhost 8080
    any_addr = SocketAddrV4::any 7000
    local.a as I64 .println!
    local.d as I64 .println!
    any.a as I64 .println!
    custom.c as I64 .println!
    new_addr = SocketAddrV4::new custom, 9000
    addr.port.println!
    new_addr.ip.a as I64 .println!
    new_addr.port.println!
    any_addr.ip.a as I64 .println!
    any_addr.port.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(
        lines,
        vec!["127", "1", "0", "3", "8080", "1", "9000", "0", "7000"]
    );
}

#[test]
fn test_tcp_listener_bind_port_zero_reports_local_addr() {
    let output = compile_and_run(
        r#"
> stdlib::net::TcpListener
> stdlib::net::SocketAddrV4
> stdlib::result::Result

main = ->
    match (TcpListener::bind (SocketAddrV4::localhost 0))
        Result::Ok listener =>
            match listener.local_addr!
                Result::Ok addr =>
                    addr.port > 0 .println!
                    addr.ip.a as I64 .println!
                    addr.ip.b as I64 .println!
                    addr.ip.c as I64 .println!
                    addr.ip.d as I64 .println!
                    0
                Result::Err _ => 2
        Result::Err _ => 1
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["true", "127", "0", "0", "1"]);
}

#[test]
fn test_tcp_listener_fd_field_is_private() {
    compile_should_fail(
        r#"
> stdlib::result::Result
> stdlib::net::TcpListener
> stdlib::net::SocketAddrV4

main = ->
    match (TcpListener::bind (SocketAddrV4::localhost 0))
        Result::Ok listener =>
            fd = listener.fd
            fd as I64
        Result::Err _ => 1
"#,
        "Field 'fd' of struct 'TcpListener' is private",
    );
}

#[test]
fn test_tcp_stream_fd_field_is_private() {
    compile_should_fail(
        r#"
> stdlib::result::Result
> stdlib::net::TcpStream
> stdlib::net::SocketAddrV4

main = ->
    match (TcpStream::connect (SocketAddrV4::localhost 1))
        Result::Ok stream =>
            fd = stream.fd
            fd as I64
        Result::Err _ => 1
"#,
        "Field 'fd' of struct 'TcpStream' is private",
    );
}

#[test]
fn test_tcp_stream_connect_failure_returns_error() {
    let output = compile_and_run(
        r#"
> stdlib::result::Result
> stdlib::net::TcpStream
> stdlib::net::SocketAddrV4
> stdlib::io::IoError

main = ->
    match (TcpStream::connect (SocketAddrV4::localhost 70000))
        Result::Ok _ =>
            0.println!
            0
        Result::Err IoError::InvalidAddress =>
            2.println!
            0
        Result::Err _ =>
            1.println!
            0
"#,
    );

    assert_eq!(output.trim(), "2");
}

#[test]
fn test_tcp_stream_shared_safe_io_and_write_shutdown() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    let _cleanup = TestDirCleanup(dir.clone());
    let output = compile_and_run_in_working_dir(
        r#"
> stdlib::arc::Arc
> stdlib::io::IoError
> stdlib::io::Write
> stdlib::result::Result
> stdlib::net::TcpListener
> stdlib::net::TcpStream
> stdlib::net::SocketAddrV4

roundtrip: () -> Result I64, IoError
roundtrip = ->
    listener = TcpListener::bind (SocketAddrV4::localhost 0)?
    addr = listener.local_addr!?
    client = Arc::new (TcpStream::connect addr?)
    server = Arc::new (listener.accept!?)
    bytes = [112 as U8, 105, 110, 103, 120]
    written = client.write_all (&bytes[..4])?
    mut received = [0 as U8, 0, 0, 0]
    read = server.recv (&mut received)?
    stopped = client.shutdown_write!
    written.println!
    read.println!
    received[0] as I64 .println!
    received[3] as I64 .println!
    Result::Ok 0

main = ->
    match roundtrip!
        Result::Ok code => code
        Result::Err _ => 1
"#,
        &dir,
    );

    assert!(
        output.status.success(),
        "binary failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .lines()
            .collect::<Vec<_>>(),
        ["4", "4", "112", "103"],
    );
}

#[test]
fn test_stdout_write_all_uses_native_slice() {
    let output = compile_and_run(
        r#"
> stdlib::io::stdout

main = ->
    mut output = stdout!
    bytes = [104 as U8, 101, 108, 108, 111]
    written = output.write_all (&bytes[..3])
    0
"#,
    );

    assert_eq!(output, "hel");
}

#[test]
fn test_stdlib_arc_forwards_shared_write_bound() {
    let output = compile_and_run(
        r#"
> stdlib::arc::Arc
> stdlib::io::IoError
> stdlib::io::Write
> stdlib::result::Result

struct Sink

impl Write for Sink
    @write = _ -> Result::Ok 1
    @write_str = value -> stdlib::io::write_str self, value
    @write_all = bytes -> stdlib::io::write_all self, bytes

write_one: &W -> Result I64, IoError where W: Write
write_one = target ->
    bytes: [U8; 1] = [0; 1]
    target.write_all (&bytes)

main = ->
    sink = Arc::new Sink
    match write_one (&sink)
        Result::Ok count => count.println!
        Result::Err _ => -1 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "1");
}

#[test]
fn test_write_helpers_handle_partial_string_and_zero_writes() {
    let output = compile_and_run(
        r#"
> stdlib::io::IoError
> stdlib::io::Write
> stdlib::result::Result

struct PartialSink
struct ZeroSink

impl Write for PartialSink
    @write = bytes ->
        len = ~ArrayLen bytes
        Result::Ok (if len < 2
            len
        else
            2)
    @write_str = value -> stdlib::io::write_str self, value
    @write_all = bytes -> stdlib::io::write_all self, bytes

impl Write for ZeroSink
    @write = _ -> Result::Ok 0
    @write_str = value -> stdlib::io::write_str self, value
    @write_all = bytes -> stdlib::io::write_all self, bytes

main = ->
    sink = PartialSink
    bytes = [1 as U8, 2, 3, 4, 5]
    match sink.write_all (&bytes)
        Result::Ok count => count.println!
        Result::Err _ => -1 .println!
    match sink.write_str "hello"
        Result::Ok count => count.println!
        Result::Err _ => -1 .println!
    zero = ZeroSink
    match zero.write_all (&bytes)
        Result::Err IoError::WriteZero => 1 .println!
        _ => 0 .println!
    0
"#,
    );

    assert_eq!(output.lines().collect::<Vec<_>>(), vec!["5", "5", "1"]);
}

#[test]
fn test_tcp_listener_stream_localhost_roundtrip() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    let _cleanup = TestDirCleanup(dir.clone());
    let output = compile_and_run_in_working_dir(
        r#"
> stdlib::io::IoError
> stdlib::io::Read
> stdlib::io::Write
> stdlib::result::Result
> stdlib::net::TcpListener
> stdlib::net::TcpStream
> stdlib::net::SocketAddrV4

roundtrip: () -> Result I64, IoError
roundtrip = ->
    listener = TcpListener::bind (SocketAddrV4::localhost 0)?
    addr = listener.local_addr!?
    mut client = TcpStream::connect addr?
    mut server = listener.accept!?
    written = client.write_str "ping"?
    mut buf = [0 as U8, 0, 0, 0]
    slice = &mut buf
    read = server.read slice?
    written.println!
    read.println!
    buf[0] as I64 .println!
    buf[3] as I64 .println!
    Result::Ok 0

main = ->
    match roundtrip!
        Result::Ok code => code
        Result::Err _ => 1
"#,
        &dir,
    );

    assert!(
        output.status.success(),
        "binary failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines, vec!["4", "4", "112", "103"]);
}

#[test]
fn test_tcp_listener_accept_method_section_in_result_chain() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    let _cleanup = TestDirCleanup(dir.clone());
    let output = compile_and_run_in_working_dir(
        r#"
> stdlib::io::IoError
> stdlib::io::Read
> stdlib::io::Write
> stdlib::result::Result
> stdlib::net::TcpListener
> stdlib::net::TcpStream
> stdlib::net::SocketAddrV4

accept_with_method_section: TcpListener -> Result TcpStream, IoError
accept_with_method_section = listener ->
    (listener.local_addr! >>= (ignored -> Result::Ok listener)) >>= (.accept!)

roundtrip: () -> Result I64, IoError
roundtrip = ->
    listener = TcpListener::bind (SocketAddrV4::localhost 0)?
    addr = listener.local_addr!?
    mut client = TcpStream::connect addr?
    mut server = accept_with_method_section listener?
    written = server.write_str "hello"?
    mut buf = [0, 0, 0, 0, 0]
    slice = &mut buf
    read = client.read slice?
    written.println!
    read.println!
    buf[0] as I64 .println!
    buf[4] as I64 .println!
    Result::Ok 0

main = ->
    match roundtrip!
        Result::Ok code => code
        Result::Err _ => 1
"#,
        &dir,
    );

    assert!(
        output.status.success(),
        "binary failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines, vec!["5", "5", "104", "111"]);
}

#[test]
fn test_file_create_write_str_uses_generic_write_trait() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    let _cleanup = TestDirCleanup(dir.clone());

    let output = compile_and_run_in_working_dir(
        r#"
> stdlib::fs::File
> stdlib::io::IoError
> stdlib::io::Write
> stdlib::result::Result

write_text: &mut W -> &Str -> Result I64, IoError where W: Write
write_text = writer, text ->
    written = writer.write_str text?
    Result::Ok written

write_file: &Str -> Result I64, IoError
write_file = path ->
    mut file = File::create path?
    written = write_text (&mut file), "hello"?
    Result::Ok written

main = ->
    match write_file "out.txt"
        Result::Ok written =>
            written.println!
            0
        Result::Err _ => 1
"#,
        &dir,
    );

    assert!(
        output.status.success(),
        "binary failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "5");
    assert_eq!(fs::read_to_string(dir.join("out.txt")).unwrap(), "hello");
}

#[test]
fn test_file_open_read_uses_generic_read_trait() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    fs::write(dir.join("input.txt"), b"world").unwrap();

    let output = compile_and_run_in_working_dir(
        r#"
> stdlib::fs::File
> stdlib::io::IoError
> stdlib::io::Read
> stdlib::result::Result

read_into: &mut R -> &mut [U8] -> Result I64, IoError where R: Read
read_into = reader, buf ->
    read = reader.read buf?
    Result::Ok read

read_file: &Str -> Result I64, IoError
read_file = path ->
    mut file = File::open path?
    mut buf = [0 as U8, 0, 0, 0, 0]
    slice = &mut buf
    read = read_into (&mut file), slice?
    read.println!
    buf[0] as I64 .println!
    buf[4] as I64 .println!
    Result::Ok 0

main = ->
    match read_file "input.txt"
        Result::Ok code => code
        Result::Err _ => 1
"#,
        &dir,
    );

    assert!(
        output.status.success(),
        "binary failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim().lines().collect::<Vec<_>>(),
        vec!["5", "119", "100"]
    );
}

#[test]
fn test_file_append_writes_at_end() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    fs::write(dir.join("log.txt"), b"first").unwrap();

    let output = compile_and_run_in_working_dir(
        r#"
> stdlib::fs::File
> stdlib::io::IoError
> stdlib::io::Write
> stdlib::result::Result

append_text: &Str -> Result I64, IoError
append_text = path ->
    mut file = File::append path?
    written = file.write_str "-second"?
    Result::Ok written

main = ->
    match append_text "log.txt"
        Result::Ok written =>
            written.println!
            0
        Result::Err _ => 1
"#,
        &dir,
    );

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "7");
    assert_eq!(
        fs::read_to_string(dir.join("log.txt")).unwrap(),
        "first-second"
    );
}

#[test]
fn test_io_pipe_operator_consumes_reader_and_copies_until_eof() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    let contents = "functional byte pipe".repeat(1024);
    fs::write(dir.join("input.txt"), contents.as_bytes()).unwrap();

    let output = compile_and_run_in_working_dir(
        r#"
> stdlib::fs::File
> stdlib::io::IoError
> stdlib::result::Result

copy_file: &Str -> &Str -> Result I64, IoError
copy_file = source, target ->
    output = File::create target?
    (File::open source?) |>> &output

main = ->
    match copy_file "input.txt", "output.txt"
        Result::Ok copied =>
            copied.println!
            0
        Result::Err _ => 1
"#,
        &dir,
    );

    assert!(
        output.status.success(),
        "pipe binary failed with {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        contents.len().to_string()
    );
    assert_eq!(
        fs::read_to_string(dir.join("output.txt")).unwrap(),
        contents
    );
}

#[test]
fn test_file_create_rejects_embedded_nul_path() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    fs::write(dir.join("nul_path.txt"), b"safe").unwrap();

    let output = compile_and_run_in_working_dir(
        r#"
> stdlib::fs::File
> stdlib::io::IoError
> stdlib::io::Write
> stdlib::result::Result

overwrite: &Str -> Result I64, IoError
overwrite = path ->
    mut file = File::create path?
    written = file.write_str "mutated"?
    Result::Ok written

main = ->
    match overwrite "nul_path.txt\0suffix"
        Result::Err IoError::InvalidPath => 0
        Result::Ok _ => 1
        Result::Err _ => 2
"#,
        &dir,
    );

    assert!(output.status.success());
    assert_eq!(
        fs::read_to_string(dir.join("nul_path.txt")).unwrap(),
        "safe"
    );
}

#[test]
fn test_file_open_missing_path_returns_os_error() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    let _cleanup = TestDirCleanup(dir.clone());

    let output = compile_and_run_in_working_dir(
        r#"
> stdlib::fs::File
> stdlib::io::IoError
> stdlib::result::Result

main = ->
    match File::open "missing.txt"
        Result::Err IoError::Os _ => 0
        Result::Ok _ => 1
        Result::Err _ => 2
"#,
        &dir,
    );

    assert!(output.status.success());
}

#[test]
fn test_file_fd_field_is_private() {
    compile_should_fail(
        r#"
> stdlib::fs::File
> stdlib::io::IoError
> stdlib::result::Result

read_fd: &Str -> Result I64, IoError
read_fd = path ->
    file = File::open path?
    Result::Ok (file.fd as I64)

main = -> 0
"#,
        "Field 'fd' of struct 'File' is private",
    );
}

#[test]
fn test_file_raw_fd_constructor_not_public() {
    compile_should_fail(
        r#"
> stdlib::fs::File

main = ->
    file = File
        fd: 0
    0
"#,
        "Cannot construct struct 'stdlib::fs::File' because it has private fields",
    );
}

#[test]
fn test_function_name_can_match_c_extern_symbol() {
    let output = compile_and_run(
        r#"
> stdlib::net::TcpListener

main = ->
    listen! .println!
    0

listen = -> 42
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_tcp_listener_raw_fd_constructor_not_public() {
    compile_should_fail(
        r#"
> stdlib::net::TcpListener

main = ->
    listener = TcpListener
        fd: 0
    0
"#,
        "Cannot construct struct 'stdlib::net::TcpListener' because it has private fields",
    );
}

#[test]
fn test_hash_trait_scalar_and_str() {
    let output = compile_and_run(
        r#"
main = ->
    42.hash! .println!
    (-42).hash! .println!
    true.hash! .println!
    false.hash! .println!
    'A'.hash! .println!
    "abc".hash! .println!
    "abc".hash! .println!
    "abd".hash! .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "42");
    assert_eq!(lines[1], "-42");
    assert_eq!(lines[2], "1");
    assert_eq!(lines[3], "0");
    assert_eq!(lines[4], "65");
    assert_eq!(lines[5], lines[6]);
    assert_ne!(lines[5], lines[7]);
}

#[test]
fn test_method_call_mut_receiver_requires_mutable_binding() {
    compile_should_fail(
        r#"
main = ->
    k = Lol::new!
    k.mdr!
    0

struct Lol
    mdr: I64

impl Lol
    new = ->
        Lol
            mdr: 8
    ^@mdr = -> 2
"#,
        "mutable receiver",
    );
}

#[test]
fn immutable_binding_cannot_call_mut_receiver_method() {
    compile_should_fail(
        r#"
struct Counter
    < value: I64

impl Counter
    ^@inc: ()
    ^@inc = ->
        self.value = self.value + 1
        return

main = ->
    c = Counter
        value: 0
    c.inc!
    0
"#,
        "mutable receiver",
    );
}

#[test]
fn shared_reference_cannot_call_mut_receiver_method() {
    compile_should_fail(
        r#"
struct Counter
    < value: I64

impl Counter
    ^@inc: ()
    ^@inc = ->
        self.value = self.value + 1
        return

use_shared: &Counter -> ()
use_shared = c ->
    c.inc!
    return

main = -> 0
"#,
        "mutable receiver",
    );
}

#[test]
fn immutable_field_cannot_call_mut_receiver_method() {
    compile_should_fail(
        r#"
struct Counter
    < value: I64

impl Counter
    ^@inc: ()
    ^@inc = ->
        self.value = self.value + 1
        return

struct Wrapper
    < counter: Counter

main = ->
    w = Wrapper
        counter: Counter
            value: 0
    w.counter.inc!
    0
"#,
        "mutable receiver",
    );
}

#[test]
fn deferred_method_section_reports_mutable_receiver_requirement() {
    compile_should_fail(
        r#"
struct Counter
    < value: I64

impl Counter
    ^@advance: Result I64, I64
    ^@advance = ->
        self.value = self.value + 1
        Result::Ok self.value

make: () -> Result Counter, I64
make = ->
    Result::Ok (Counter
        value: 0)

main = ->
    result = make! >>= (.advance!)
    0
"#,
        "Cannot call mutable receiver method 'advance' without a mutable receiver",
    );
}

#[test]
fn deferred_method_section_autorefs_shared_receiver() {
    let output = compile_and_run(
        r#"
> stdlib::result::Result

struct Counter
    < value: I64

impl Counter
    @read: Result I64, I64
    @read = -> Result::Ok self.value

make: () -> Result Counter, I64
make = -> Result::Ok (Counter
    value: 41)

main = ->
    result: Result I64, I64 = make! >>= (.read!)
    result.println!
    0
"#,
    );
    assert_eq!(output.trim(), "Ok(41)");
}

#[test]
fn deferred_method_section_uses_autoderef_receiver() {
    let output = compile_and_run(
        r#"
struct Inner
    < value: I64

impl Inner
    @read = -> self.value

struct Box T
    < value: T

impl Deref for Box T
    type Target = T
    @* = -> &@value

make: () -> Box Inner
make = -> Box
    value: Inner
        value: 42

main = ->
    result = make! |> (.read!)
    result.println!
    0
"#,
    );
    assert_eq!(output.trim(), "42");
}

#[test]
fn deferred_method_value_execution_preserves_selected_method() {
    let output = compile_and_run(
        r#"
struct Counter
    < value: I64

impl Counter
    @read = -> self.value

identity = value -> value

main = ->
    counter = identity (Counter
        value: 43)
    reader = counter.read
    value: I64 = reader!
    value.println!
    0
"#,
    );
    assert_eq!(output.trim(), "43");
}

#[test]
fn deferred_bounded_generic_method_rejects_invalid_argument() {
    compile_should_fail(
        r#"
trait Marker

struct Box

impl Box
    @requires_marker: T -> () where T: Marker
    @requires_marker = _ -> return

make: () -> Result Box, I64
make = -> Result::Ok Box

main = ->
    result = make! >>= (.requires_marker 1)
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn deferred_unknown_method_section_remains_unknown_field() {
    compile_should_fail(
        r#"
struct Counter
    < value: I64

make: () -> Result Counter, I64
make = ->
    Result::Ok (Counter
        value: 0)

main = ->
    result = make! >>= (.missing!)
    0
"#,
        "Unknown field 'missing' on struct 'Counter'",
    );
}

#[test]
fn mutable_binding_can_call_mut_receiver_method() {
    let output = compile_and_run(
        r#"
struct Counter
    < value: I64

impl Counter
    ^@inc: ()
    ^@inc = ->
        self.value = self.value + 1
        return

    @get: I64
    @get = -> self.value

main = ->
    mut c = Counter
        value: 1
    c.inc!
    c.get! .println!
    0
"#,
    );
    assert_eq!(output.trim(), "2");
}

#[test]
fn trait_bound_mut_receiver_call_autorefs_mut_reference() {
    let output = compile_and_run(
        r#"
struct Counter
    < value: I64

trait Touch
    ^@touch: I64

impl Touch for Counter
    ^@touch = ->
        self.value = self.value + 1
        self.value

poke: &mut T -> I64 where T: Touch
poke = value -> value.touch!

main = ->
    mut counter = Counter
        value: 41
    poke (&mut counter) .println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn trait_bound_shared_receiver_on_ref_does_not_double_borrow() {
    let output = compile_and_run(
        r#"
trait Read
    @read: I64

impl Read for I64
    @read = -> *self

read_ref: &T -> I64 where T: Read
read_ref = value -> value.read!

main = ->
    value = 42
    read_ref &value .println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_invalid_receiver_sequence_is_rejected() {
    compile_should_fail(
        r#"
main = ->
    k = Lol::new!
    k.mdr!.println!
    k.ta!.println!
    k.mdr!.println!
    0

struct Lol
    mdr: I64

impl Lol
    new = ->
        Lol
            mdr: 8
    ^@mdr = -> 2
    ~@ta = -> 1
"#,
        "mutable receiver",
    );
}

#[test]
fn test_inline_program() {
    let output = compile_and_run(
        r#"
max = a, b ->
    if a > b then a else b

min = a, b ->
    if a < b then a else b

main = ->
    max 10, 20 .println!
    min 10, 20 .println!
    max 5, 3 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "20");
    assert_eq!(lines[1], "10");
    assert_eq!(lines[2], "5");
}

#[test]
fn test_inline_nested_functions() {
    let output = compile_and_run(
        r#"
double = x -> x * 2
triple = x -> x * 3

main = ->
    double 5 .println!
    triple 5 .println!
    double (triple 3) .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "15");
    assert_eq!(lines[2], "18");
}

#[test]
fn unit_arrow_evaluates_body_and_discards_named_function_and_lambda_results() {
    let output = compile_and_run(
        r#"
discard = value !->
    7.println!
    value

emit_value = ->
    9.println!
    42

main = !->
    discard 99
    f = value !->
        8.println!
        value
    f 9
    run_effect = !-> emit_value!
    run_effect!
    emit_value!
"#,
    );

    assert_eq!(
        output.trim().lines().collect::<Vec<_>>(),
        ["7", "8", "9", "9"]
    );
}

#[test]
fn unit_arrow_main_exits_zero_without_stdlib() {
    for source in [
        "main = !-> 42\n",
        "main = !-> (42, 7)\n",
        "main = !-> return\n",
    ] {
        assert_eq!(compile_and_run_without_stdlib(source), 0);
    }
    assert_eq!(compile_and_run_without_stdlib("main = -> 7\n"), 7);
}

#[test]
fn test_impl_methods() {
    let output = compile_example("impl_methods");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "7"); // sum: 3+4
    assert_eq!(lines[1], "3"); // get_x: 3
    assert_eq!(lines[2], "13"); // add 10: 3+10
}

#[test]
fn test_lambdas() {
    let output = compile_example("lambdas");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10"); // double 5
    assert_eq!(lines[1], "14"); // apply double 7
    assert_eq!(lines[2], "105"); // apply (x -> x+100) 5
}

#[test]
fn test_call_argument_holes_desugar_to_lambdas() {
    let output = compile_and_run(
        r#"
struct Boxed
    < value: I64

combine = a, b, c -> a * 100 + b * 10 + c
apply = f, value -> f value
box = value -> Boxed value: value

main = ->
    fixed = 3
    middle = combine 1, _, fixed
    outer = combine _, 5, _
    nested = apply (combine 7, _, 9), 8
    projected = box _.value
    middle 2 .println!
    outer 4, 6 .println!
    nested.println!
    projected 42 .println!
    0
"#,
    );

    assert_eq!(
        output.trim().lines().collect::<Vec<_>>(),
        ["123", "456", "789", "42"]
    );
}

#[test]
fn test_modulo_operator() {
    let output = compile_and_run(
        r#"
main = ->
    10 % 3 .println!
    15 % 5 .println!
    7 % 2 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "0");
    assert_eq!(lines[2], "1");
}

#[test]
fn test_string_operations() {
    let output = compile_and_run(
        r#"
main = ->
    string_len "hello" .println!
    string_len "" .println!
    string_len "world!" .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "5");
    assert_eq!(lines[1], "0");
    assert_eq!(lines[2], "6");
}

#[test]
fn test_owned_string_borrowed_view_matches_string_literals() {
    let output = compile_and_run(
        r#"
classify: String -> I64
classify = value ->
    match value.as_str!
        "listen" => 1
        "connect" => 2
        _ => 3

main = ->
    classify (String::from_str "listen") .println!
    classify (String::from_str "connect") .println!
    classify (String::from_str "other") .println!
    0
"#,
    );

    assert_eq!(output, "1\n2\n3\n");
}

#[test]
fn test_owned_string_literal_match_requests_borrowed_str_view() {
    compile_should_fail(
        r#"
main = ->
    value = String::from_str "listen"
    match value
        "listen" => 1
        _ => 0
"#,
        "call .as_str! before matching an owned String",
    );
}

#[test]
fn test_complex_program() {
    let output = compile_example("complex");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "5"); // sqrt(3^2 + 4^2) = 5
    assert_eq!(lines[1], "55"); // sum 1..10
    assert_eq!(lines[2], "true"); // is_even(4)
    assert_eq!(lines[3], "false"); // is_even(7)
}

#[test]
fn test_extern_functions() {
    let output = compile_example("extern_test");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "12"); // sqrt(144)
    assert_eq!(lines[1], "1.41421"); // sqrt(2)
}

#[test]
fn test_modules() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    // Write the module file
    std::fs::write(
        dir.join("utils.rk"),
        "\
add = a, b -> a + b

double = x -> x * 2

square = x -> x * x

< add
< double
< square
",
    )
    .unwrap();

    // Write the main file that uses the module
    std::fs::write(
        dir.join("test.rk"),
        "\
mod utils
> utils::add
> utils::double
> utils::square

main = ->
    add 3, 4 .println!
    double 10 .println!
    square 5 .println!
    0
",
    )
    .unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());

    rock_lib::compile(&config).expect("Compilation failed");

    let binary = dir.join("test");
    let output = Command::new(&binary)
        .output()
        .expect("Failed to run compiled binary");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        output.status.success(),
        "binary failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        stdout,
        stderr
    );
    assert!(
        !stdout.trim().is_empty(),
        "binary produced empty stdout with status {:?}\nstderr:\n{}",
        output.status.code(),
        stderr
    );

    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "7"); // add 3 4
    assert_eq!(lines[1], "20"); // double 10
    assert_eq!(lines[2], "25"); // square 5
}

#[test]
fn test_module_type_alias_import_export_and_qualification() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("types.rk"), "type Number = I64\n\n< Number\n").unwrap();
    std::fs::write(
        dir.join("test.rk"),
        "mod types\n> types::Number\n\nidentity: Number -> Number\nidentity = value -> value\n\nmain = ->\n    identity 42 .println!\n    0\n",
    )
    .unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());
    rock_lib::compile(&config).expect("Compilation failed");
    let output = Command::new(dir.join("test"))
        .output()
        .expect("Failed to run compiled binary");
    let _ = std::fs::remove_dir_all(&dir);

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");
}

#[test]
fn test_module_directory_mod_file_resolution() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(dir.join("utils")).unwrap();

    std::fs::write(
        dir.join("utils").join("mod.rk"),
        "\
answer = -> 42

< answer
",
    )
    .unwrap();

    std::fs::write(
        dir.join("test.rk"),
        "\
mod utils
> utils::answer

main = ->
    answer!.println!
    0
",
    )
    .unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());
    rock_lib::compile(&config).expect("Compilation failed");

    let binary = dir.join("test");
    let output = Command::new(&binary)
        .output()
        .expect("Failed to run compiled binary");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(stdout.trim(), "42");
}

#[test]
fn test_missing_module_reports_all_searched_paths() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(dir.join("test.rk"), "mod missing\nmain = -> 0\n").unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());
    let result = rock_lib::compile(&config);
    let diagnostics = result.expect_err("missing module should fail");
    let text = diagnostics
        .0
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let _ = std::fs::remove_dir_all(&dir);

    assert!(text.contains("missing.rk"), "diagnostics were {text}");
    assert!(text.contains("missing/mod.rk"), "diagnostics were {text}");
}

#[test]
fn test_circular_module_load_reports_diagnostic() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(dir.join("test.rk"), "mod a\nmain = -> 0\n").unwrap();
    std::fs::write(dir.join("a.rk"), "mod b\n").unwrap();
    std::fs::write(dir.join("b.rk"), "mod a\n").unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());
    let result = rock_lib::compile(&config);
    let diagnostics = result.expect_err("cycle should fail");
    let text = diagnostics
        .0
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        text.contains("Circular module load"),
        "diagnostics were {text}"
    );
    assert!(text.contains("a.rk"), "diagnostics were {text}");
    assert!(text.contains("b.rk"), "diagnostics were {text}");
}

#[test]
fn test_module_glob_import_and_export() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(
        dir.join("math_impl.rk"),
        "\
add = a, b -> a + b

double = x -> x * 2

< add
< double
",
    )
    .unwrap();

    std::fs::write(
        dir.join("api.rk"),
        "\
mod math_impl

< math_impl::*
",
    )
    .unwrap();

    std::fs::write(
        dir.join("test.rk"),
        "\
mod api
> api::*

main = ->
    add 2, 5 .println!
    double 9 .println!
    0
",
    )
    .unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());

    rock_lib::compile(&config).expect("Compilation failed");

    let binary = dir.join("test");
    let output = Command::new(&binary)
        .output()
        .expect("Failed to run compiled binary");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let _ = std::fs::remove_dir_all(&dir);

    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "7");
    assert_eq!(lines[1], "18");
}

#[test]
fn test_id_owned_hir_preserves_same_name_module_calls() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(
        dir.join("left.rk"),
        "\
value = -> 11

< value
",
    )
    .unwrap();
    std::fs::write(
        dir.join("right.rk"),
        "\
value = -> 31

< value
",
    )
    .unwrap();
    std::fs::write(
        dir.join("test.rk"),
        "\
mod left
mod right

main = ->
    left::value! + right::value! .println!
    0
",
    )
    .unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());
    rock_lib::compile(&config).expect("Compilation failed");

    let binary = dir.join("test");
    let output = Command::new(&binary)
        .output()
        .expect("Failed to run compiled binary");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(stdout.trim(), "42");
}

#[test]
fn test_module_with_structs() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    // Write the math module with a struct and functions
    std::fs::write(
        dir.join("geometry.rk"),
        "\
struct Point
    < x: I64
    < y: I64

make_point = a, b ->
    Point
        x: a
        y: b

distance_sq = p ->
    p.x * p.x + p.y * p.y

< Point
< make_point
< distance_sq
",
    )
    .unwrap();

    // Write the main file
    std::fs::write(
        dir.join("test.rk"),
        "\
mod geometry
> geometry::Point
> geometry::make_point
> geometry::distance_sq

main = ->
    p = make_point 3, 4
    p.x.println!
    p.y.println!
    distance_sq p .println!
    0
",
    )
    .unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());

    rock_lib::compile(&config).expect("Compilation failed");

    let binary = dir.join("test");
    let output = Command::new(&binary)
        .output()
        .expect("Failed to run compiled binary");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let _ = std::fs::remove_dir_all(&dir);

    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "3"); // p.x
    assert_eq!(lines[1], "4"); // p.y
    assert_eq!(lines[2], "25"); // 3^2 + 4^2 = 25
}

#[test]
fn test_array_indexing() {
    let output = compile_and_run(
        r#"
main = ->
    arr = [10, 20, 30, 40, 50]
    arr[0] .println!
    arr[2] .println!
    arr[4] .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "30");
    assert_eq!(lines[2], "50");
}

#[test]
fn test_array_for_loop() {
    let output = compile_and_run(
        r#"
main = ->
    arr = [1, 2, 3, 4, 5]
    sum = 0
    for x in arr
        sum = sum + x
    sum.println!
    0
"#,
    );
    assert_eq!(output.trim(), "15");
}

#[test]
fn test_string_concat() {
    let output = compile_and_run(
        r#"
main = ->
    a = "Hello, "
    b = "World!"
    c = string_concat a, b
    c.println!
    0
"#,
    );
    assert_eq!(output.trim(), "Hello, World!");
}

#[test]
fn test_string_add_operator_concats_owned_string_and_borrowed_str() {
    let output = compile_and_run(
        r#"
main = ->
    (String::from_str "Hello, ") + "World!" .println!
    "Hello, " + "World!" .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["Hello, World!"; 2]);
}

#[test]
fn test_struct_field_assignment() {
    let output = compile_and_run(
        r#"
struct Point
    < x: I64
    < y: I64

main = ->
    p = Point
        x: 10
        y: 20
    p.x.println!
    p.x = 42
    p.x.println!
    p.y.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10"); // original x
    assert_eq!(lines[1], "42"); // modified x
    assert_eq!(lines[2], "20"); // y unchanged
}

#[test]
fn test_array_index_assignment() {
    let output = compile_and_run(
        r#"
main = ->
    mut arr = [10, 20, 30]
    arr[1] .println!
    arr[1] = 99
    arr[1] .println!
    arr[0] .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "20"); // original arr[1]
    assert_eq!(lines[1], "99"); // modified arr[1]
    assert_eq!(lines[2], "10"); // arr[0] unchanged
}

#[test]
fn test_slice_indexing() {
    let output = compile_and_run(
        r#"
read_second: &[I64] -> I64
read_second = slice -> slice[1]

main = ->
    arr = [7, 8, 9]
    read_second &arr .println!
    0
"#,
    );

    assert_eq!(output.trim(), "8");
}

#[test]
fn test_native_ranges_borrow_slice_views() {
    let output = compile_and_run(
        r#"
inspect: &[I64] -> ()
inspect = slice !->
    (~ArrayLen slice).println!
    slice[0].println!

inspect_ranges: &[I64] -> ()
inspect_ranges = slice !->
    inspect (&slice[..2])
    inspect (&slice[1..])
    inspect (&slice[1..3])
    inspect (&slice[..=1])
    inspect (&slice[1..=2])
    inspect (&slice[..])
    selected = 1..3
    inspect (&slice[selected])

main = ->
    values = [10, 20, 30, 40]
    inspect_ranges (&values)
    inspect (&values[..2])
    0
"#,
    );

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        ["2", "10", "3", "20", "2", "20", "2", "10", "2", "20", "4", "10", "2", "20", "2", "10"]
    );
}

#[test]
fn test_inclusive_native_range_for_loop_includes_end() {
    let output = compile_and_run(
        r#"
main = ->
    mut total = 0
    for value in 1..=3
        total = total + value
    total.println!
    0
"#,
    );

    assert_eq!(output.trim(), "6");
}

#[test]
fn test_slice_index_assignment() {
    let output = compile_and_run(
        r#"
write_second: &mut [I64] -> I64
write_second = input ->
    mut slice = input
    slice[1] = 42
    0

main = ->
    mut values = [7, 8, 9]
    write_second (&mut values)
    values[1].println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_native_mutable_range_updates_original_slice() {
    let output = compile_and_run(
        r#"
update_middle: &mut [I64] -> ()
update_middle = slice !->
    mut middle = &mut slice[1..3]
    middle[0] = 77

main = ->
    mut values = [10, 20, 30, 40]
    update_middle (&mut values)
    values[1].println!
    values[2].println!
    0
"#,
    );

    assert_eq!(output.lines().collect::<Vec<_>>(), ["77", "30"]);
}

#[test]
fn test_native_ranges_index_vec_views() {
    let output = compile_and_run(
        r#"
inspect: &[I64] -> ()
inspect = slice !->
    (~ArrayLen slice).println!
    slice[0].println!

update_prefix: &mut (Vec I64) -> ()
update_prefix = values !->
    mut prefix: &mut [I64] = &mut (*values)[..2]
    prefix[0] = 55

main = ->
    mut values: Vec I64 = Vec::new!
    values.push 10
    values.push 20
    values.push 30
    inspect (&values[1..])
    update_prefix (&mut values)
    values[0].println!
    0
"#,
    );

    assert_eq!(output.lines().collect::<Vec<_>>(), ["2", "20", "55"]);
}

#[test]
fn test_native_range_out_of_bounds_traps() {
    let (stdout, success) = compile_and_run_with_status(
        r#"
main = ->
    values = [10, 20, 30]
    slice = &values[1..4]
    (~ArrayLen *slice).println!
    0
"#,
    );

    assert!(!success);
    assert!(
        stdout.contains("range out of bounds"),
        "stdout was {stdout:?}"
    );
}

#[test]
fn test_fixed_array_index_negative_traps() {
    let (stdout, success) = compile_and_run_with_status(
        r#"
main = ->
    arr = [7, 8, 9]
    arr[(0 - 1)].println!
    0
"#,
    );

    assert!(!success);
    assert!(
        stdout.contains("index out of bounds"),
        "stdout was {stdout:?}"
    );
}

#[test]
fn test_fixed_array_index_upper_bound_traps() {
    let (stdout, success) = compile_and_run_with_status(
        r#"
main = ->
    arr = [7, 8, 9]
    arr[3].println!
    0
"#,
    );

    assert!(!success);
    assert!(
        stdout.contains("index out of bounds"),
        "stdout was {stdout:?}"
    );
}

#[test]
fn test_slice_index_negative_traps() {
    let (stdout, success) = compile_and_run_with_status(
        r#"
read_negative: &[I64] -> I64
read_negative = slice -> slice[(0 - 1)]

main = ->
    arr = [7, 8, 9]
    read_negative &arr .println!
    0
"#,
    );

    assert!(!success);
    assert!(
        stdout.contains("index out of bounds"),
        "stdout was {stdout:?}"
    );
}

#[test]
fn test_slice_index_upper_bound_traps() {
    let (stdout, success) = compile_and_run_with_status(
        r#"
read_upper: &[I64] -> I64
read_upper = slice -> slice[3]

main = ->
    arr = [7, 8, 9]
    read_upper &arr .println!
    0
"#,
    );

    assert!(!success);
    assert!(
        stdout.contains("index out of bounds"),
        "stdout was {stdout:?}"
    );
}

#[test]
fn test_type_conversion() {
    let output = compile_and_run(
        r#"
main = ->
    x = 42
    f = to_float x
    f.println!
    y = 3.7
    i = to_int y
    i.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "42"); // int to float (printed with %g)
    assert_eq!(lines[1], "3"); // float to int (truncates)
}

#[test]
fn test_array_len() {
    let output = compile_and_run(
        r#"
main = ->
    arr = [10, 20, 30, 40, 50]
    ~ArrayLen arr .println!
    empty = [1]
    ~ArrayLen empty .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "5");
    assert_eq!(lines[1], "1");
}

#[test]
fn test_array_len_accepts_borrowed_slice() {
    let output = compile_and_run(
        r#"
sum_array: &[I64] -> I64
sum_array = arr ->
    total = 0
    i = 0
    len = ~ArrayLen arr
    while i < len
        total = total + arr[i]
        i = i + 1
    total

main = ->
    arr = [1, 2, 3, 4]
    sum_array &arr .println!
    0
"#,
    );

    assert_eq!(output.trim(), "10");
}

#[test]
fn test_boolean_printing() {
    let output = compile_and_run(
        r#"
main = ->
    true.println!
    false.println!
    3 > 2 .println!
    3 < 2 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "true");
    assert_eq!(lines[1], "false");
    assert_eq!(lines[2], "true");
    assert_eq!(lines[3], "false");
}

#[test]
fn test_comprehensive_program() {
    // Tests: structs, methods, enums, match, arrays, loops, functions, recursion, strings
    let output = compile_and_run(
        r#"
struct Vec2
    < x: I64
    < y: I64

impl Vec2
    @dot = other ->
        @x * other.x + @y * other.y

    @magnitude_sq = ->
        @x * @x + @y * @y

abs = n ->
    if n < 0 then 0 - n else n

gcd = a, b ->
    if b == 0 then a
    else gcd b, (a % b)

sum_array: &[I64] -> I64
sum_array = arr ->
    total = 0
    i = 0
    while i < (~ArrayLen arr)
        total = total + arr[i]
        i = i + 1
    total

main = ->
    v1 = Vec2
        x: 3
        y: 4
    v2 = Vec2
        x: 1
        y: 2

    v1.dot v2 .println!
    v1.magnitude_sq!.println!

    gcd 48, 18 .println!

    arr = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
    sum_array &arr .println!

    abs 42 .println!
    abs (0 - 7) .println!

    s1 = string_concat "Rock", " "
    greeting = s1.concat (String::from_str "lang!")
    greeting.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "11"); // dot product: 3*1 + 4*2 = 11
    assert_eq!(lines[1], "25"); // magnitude_sq: 9 + 16 = 25
    assert_eq!(lines[2], "6"); // gcd(48, 18) = 6
    assert_eq!(lines[3], "55"); // sum 1..10 = 55
    assert_eq!(lines[4], "42"); // abs(42) = 42
    assert_eq!(lines[5], "7"); // abs(-7) = 7
    assert_eq!(lines[6], "Rock lang!"); // string concat
}

#[test]
fn test_closure_capture() {
    let output = compile_and_run(
        r#"
main = ->
    base = 100
    f = x -> x + base
    f 5 .println!
    f 20 .println!
    offset = 10
    g = y -> y * 2 + offset
    g 3 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "105"); // 5 + 100
    assert_eq!(lines[1], "120"); // 20 + 100
    assert_eq!(lines[2], "16"); // 3*2 + 10
}

#[test]
fn test_closure_capture_still_works_without_codegen_rebinding() {
    let output = compile_and_run(
        r#"
main = ->
    x = 1
    f = -> x.println!
    f!
    0
"#,
    );

    assert_eq!(output.trim(), "1");
}

#[test]
fn test_stdlib_thread_spawn_join_returns_value() {
    let output = compile_and_run(
        r#"
> stdlib::thread::spawn

main = ->
    match spawn (-> 42)
        Result::Ok handle =>
            match handle.join!
                Result::Ok value => value.println!
                Result::Err _ => "join failed".println!
        Result::Err _ => "spawn failed".println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_stdlib_thread_handle_drop_detaches_without_waiting() {
    let output = compile_and_run(
        r#"
> stdlib::thread::sleep_ms
> stdlib::thread::spawn

launch = ->
    match spawn (->
        sleep_ms 500
        "worker finished".println!
        return)
        Result::Ok _ => 0
        Result::Err _ => 1

main = ->
    code = launch!
    "handle dropped".println!
    code
"#,
    );

    assert_eq!(output.trim(), "handle dropped");
}

#[test]
fn test_stdlib_detached_thread_drops_unobserved_result() {
    let output = compile_and_run(
        r#"
> stdlib::thread::sleep_ms
> stdlib::thread::spawn

struct DetachedResult

impl Drop for DetachedResult
    ~@drop = ->
        "detached result dropped".println!
        return

launch = ->
    match spawn (-> DetachedResult)
        Result::Ok _ => 0
        Result::Err _ => 1

main = ->
    code = launch!
    sleep_ms 100
    code
"#,
    );

    assert_eq!(output.trim(), "detached result dropped");
}

#[test]
fn test_unit_name_is_not_a_primitive_type_spelling() {
    compile_should_fail(
        r#"
identity: Unit -> Unit
identity = value -> value

main = -> 0
"#,
        "unknown type 'Unit'",
    );
}

#[test]
fn test_stdlib_thread_spawn_accepts_owned_send_capture() {
    let output = compile_and_run(
        r#"
> stdlib::thread::spawn

struct Owned
    < value: I64

impl Owned
    ~@take = -> self.value

main = ->
    owned = Owned
        value: 73
    match spawn (-> owned.take!)
        Result::Ok handle =>
            match handle.join!
                Result::Ok value => value.println!
                Result::Err _ => "join failed".println!
        Result::Err _ => "spawn failed".println!
    0
"#,
    );

    assert_eq!(output.trim(), "73");
}

#[test]
fn test_stdlib_thread_spawn_rejects_borrowed_capture() {
    compile_should_fail(
        r#"
> stdlib::thread::spawn

main = ->
    value = 41
    handle = spawn (-> value + 1)
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_stdlib_thread_spawn_rejects_borrowed_capture_after_branch_merge() {
    compile_should_fail(
        r#"
> stdlib::thread::spawn

main = ->
    value = 41
    task = if true then (-> value + 1) else (-> 0)
    handle = spawn task
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_stdlib_thread_spawn_rejects_borrowed_method_value() {
    compile_should_fail(
        r#"
> stdlib::thread::spawn

struct Value
    < number: I64

impl Value
    @read = -> self.number

main = ->
    value = Value
        number: 41
    task = value.read
    handle = spawn task
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_stdlib_thread_spawn_rejects_downstream_send_impl() {
    compile_should_fail(
        r#"
> stdlib::thread::spawn

struct FakeSend
    < pointer: *I64

impl Send for FakeSend

impl FakeSend
    ~@consume = -> 1

main = ->
    value = FakeSend
        pointer: 0 as *I64
    handle = spawn (-> value.consume!)
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_stdlib_thread_spawn_accepts_struct_callable_impl() {
    let output = compile_and_run(
        r#"
> stdlib::thread::spawn

struct Task

impl FnOnce (), I64 for Task
    type Output = I64
    ~@call_once = _ -> 64

main = ->
    match spawn Task
        Result::Ok handle =>
            match handle.join!
                Result::Ok value => value.println!
                Result::Err _ => "join failed".println!
        Result::Err _ => "spawn failed".println!
    0
"#,
    );

    assert_eq!(output.trim(), "64");
}

#[test]
fn test_stdlib_arc_mutex_synchronizes_threaded_updates() {
    let output = compile_and_run(
        r#"
> stdlib::arc::Arc
> stdlib::sync::Mutex
> stdlib::thread::spawn

struct Counter
    < value: I64

impl Counter
    ^@increment = -> self.value = self.value + 1

struct Worker
    < counter: Arc (Mutex Counter)
    < iterations: I64

impl Worker
    ~@run: I64
    ~@run = ->
        mut i: I64 = 0
        while i < self.iterations
            mut guard = self.counter.lock!
            guard.get_mut!.increment!
            i = i + 1
        i

main = ->
    counter = Arc::new (Mutex::new (Counter
        value: 0))
    first = Worker
        counter: counter.clone!
        iterations: 1000
    second = Worker
        counter: counter.clone!
        iterations: 1000
    first_handle = spawn (-> first.run!)
    second_handle = spawn (-> second.run!)
    match first_handle
        Result::Ok handle => result = handle.join!
        Result::Err _ => "first spawn failed".println!
    match second_handle
        Result::Ok handle => result = handle.join!
        Result::Err _ => "second spawn failed".println!
    guard = counter.lock!
    guard.get!.value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "2000");
}

#[test]
fn test_stdlib_mutex_guard_drop_unlocks_mutex() {
    let output = compile_and_run(
        r#"
> stdlib::sync::Mutex

set_value: &Mutex I64 -> ()
set_value = mutex ->
    mut guard = mutex.lock!
    *guard = 41
    return

main = ->
    mutex = Mutex::new 0
    set_value (&mutex)
    guard = mutex.lock!
    guard.println!
    0
"#,
    );

    assert_eq!(output.trim(), "41");
}

#[test]
fn test_stdlib_mutex_guard_is_not_send() {
    compile_should_fail(
        r#"
> stdlib::sync::Mutex
> stdlib::thread::spawn

main = ->
    mutex = Mutex::new 7
    guard = mutex.lock!
    handle = spawn (-> guard.get!)
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_stdlib_thread_spawn_rejects_reference_hidden_in_moved_struct() {
    compile_should_fail(
        r#"
> stdlib::thread::spawn

struct Borrowed
    < value: &I64

impl Borrowed
    ~@read = -> *self.value

main = ->
    value = 7
    borrowed = Borrowed
        value: &value
    handle = spawn (-> borrowed.read!)
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_stdlib_mutex_try_lock_observes_guard_lifetime() {
    let output = compile_and_run(
        r#"
> stdlib::sync::Mutex

lock_once: &Mutex I64 -> ()
lock_once = mutex ->
    guard = mutex.lock!
    match mutex.try_lock!
        Option::Some _ => "unexpected lock".println!
        Option::None => "locked".println!
    return

main = ->
    mutex = Mutex::new 9
    lock_once (&mutex)
    match mutex.try_lock!
        Option::Some guard => guard.get!.println!
        Option::None => "still locked".println!
    0
"#,
    );

    assert_eq!(output.trim().lines().collect::<Vec<_>>(), ["locked", "9"]);
}

#[test]
fn test_stdlib_arc_drops_owned_value_once() {
    let (output, success) = compile_and_run_with_status(
        r#"
> stdlib::arc::Arc

main = ->
    value = Arc::new (String::from_str "shared")
    other = value.clone!
    value.println!
    other.println!
    0
"#,
    );

    assert!(success);
    assert_eq!(
        output.trim().lines().collect::<Vec<_>>(),
        ["shared", "shared"]
    );
}

#[test]
fn test_atomic_intrinsics_require_unsafe() {
    compile_should_fail(
        r#"
main = ->
    ptr = 0 as *U64
    previous = ~AtomicU64Exchange ptr, (1 as U64)
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_atomic_intrinsics_require_exact_u64_operands() {
    compile_should_fail(
        r#"
main = ->
    ptr = 0 as *U64
    previous = unsafe ~AtomicU64FetchAdd ptr, (1 as U8)
    0
"#,
        "expected argument type U64, found U8",
    );
}

#[test]
fn test_atomic_intrinsics_require_exact_argument_count() {
    compile_should_fail(
        r#"
main = ->
    ptr = 0 as *U64
    previous = unsafe ~AtomicU64FetchSub ptr
    0
"#,
        "expects exactly 2 arguments",
    );
}

#[test]
fn test_bubble_sort() {
    let output = compile_and_run(
        r#"
main = ->
    mut arr = [5, 3, 1, 4, 2]
    i = 0
    while i < 4
        j = i + 1
        while j < 5
            if arr[i] > arr[j] then
                temp = arr[i]
                arr[i] = arr[j]
                arr[j] = temp
            j = j + 1
        i = i + 1
    k = 0
    while k < 5
        arr[k] .println!
        k = k + 1
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["1", "2", "3", "4", "5"]);
}

#[test]
fn test_fibonacci_loop() {
    let output = compile_and_run(
        r#"
main = ->
    a = 0
    b = 1
    i = 0
    while i < 10
        a.println!
        temp = a + b
        a = b
        b = temp
        i = i + 1
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(
        lines,
        vec!["0", "1", "1", "2", "3", "5", "8", "13", "21", "34"]
    );
}

#[test]
fn test_nested_function_calls() {
    let output = compile_and_run(
        r#"
square = x -> x * x
double = x -> x * 2
add = x, y -> x + y

main = ->
    add (square 3), (double 5) .println!
    square (double 3) .println!
    double (square 4) .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "19"); // 9 + 10
    assert_eq!(lines[1], "36"); // (3*2)^2 = 36
    assert_eq!(lines[2], "32"); // (4^2)*2 = 32
}

#[test]
fn test_multi_string_concat() {
    let output = compile_and_run(
        r#"
main = ->
    a = "Hello"
    b = ", "
    c = "World"
    d = "!"
    s1 = string_concat a, b
    s2 = s1.concat (String::from_str c)
    result = s2.concat (String::from_str d)
    result.println!
    result.len!.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "Hello, World!");
    assert_eq!(lines[1], "13");
}

#[test]
fn test_method_signature_omits_shared_self_receiver() {
    let (output, success) = compile_and_run_with_status(
        r#"
struct Counter
    < value: I64

impl Counter
    @value: I64
    @value = -> self.value

main = ->
    c = Counter
        value: 7
    c.value! .println!
    0
"#,
    );

    assert!(success);
    assert_eq!(output.trim(), "7");
}

#[test]
fn test_method_signature_omits_mut_and_move_self_receiver() {
    let output = compile_and_run(
        r#"
struct Counter
    < value: I64

impl Counter
    ^@set: I64 -> ()
    ^@set = value ->
        self.value = value
        return

    ~@take: I64
    ~@take = -> self.value

main = ->
    mut c = Counter
        value: 1
    c.set 9
    c.take! .println!
    0
"#,
    );

    assert_eq!(output.trim(), "9");
}

#[test]
fn test_trait_method_signature_omits_self_receiver() {
    let output = compile_and_run(
        r#"
trait Value
    @value: I64

struct Boxed
    < value: I64

impl Value for Boxed
    @value = -> @value

main = ->
    b = Boxed
        value: 11
    b.value! .println!
    0
"#,
    );

    assert_eq!(output.trim(), "11");
}

#[test]
fn test_trait_method_signature_implicit_self_preserves_associated_output() {
    let output = compile_and_run(
        r#"
trait Projector
    type Output
    @project: Self::Output -> Self::Output

struct Id

impl Projector for Id
    type Output = I64
    @project = value -> value

main = ->
    id = Id
    id.project 13 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "13");
}

#[test]
fn test_bang_rejects_method_with_required_argument() {
    compile_should_fail(
        r#"
struct Counter

impl Counter
    @set: I64 -> I64
    @set = value -> value

main = ->
    counter = Counter
    counter.set! 9
    0
"#,
        "expected 1 args, got 0",
    );
}

#[test]
fn test_unsafe_trait_signature_bound_dispatch_requires_unsafe() {
    compile_should_fail(
        r#"
trait Risky
    unsafe @read: I64

struct Secret
    < value: I64

impl Risky for Secret
    @read = -> @value

read_value: T -> I64 where T: Risky
read_value = value -> value.read!

main = ->
    secret = Secret
        value: 7
    value = read_value secret
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_unsafe_trait_signature_current_trait_dispatch_requires_unsafe() {
    compile_should_fail(
        r#"
trait Risky
    unsafe @read: I64
    @read_safely: I64
    @read_safely = -> self.read!

struct Secret
    < value: I64

impl Risky for Secret
    @read = -> @value

main = ->
    secret = Secret
        value: 7
    value = secret.read_safely!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_method_signature_allows_explicit_self_operand_after_implicit_receiver() {
    let output = compile_and_run(
        r#"
trait Same
    @same: Self -> Bool

struct Token
    < value: I64

impl Same for Token
    @same = other -> self.value == other.value

main = ->
    a = Token
        value: 4
    b = Token
        value: 4
    a.same b .println!
    0
"#,
    );

    assert_eq!(output.trim(), "true");
}

#[test]
fn test_trait_basic() {
    let output = compile_and_run(
        r#"
struct Circle
    < radius: I64

struct Rectangle
    < width: I64
    < height: I64

trait Shape
    @area = -> 0

impl Shape for Circle
    @area = -> @radius * @radius * 3

impl Shape for Rectangle
    @area = -> @width * @height

main = ->
    c = Circle
        radius: 5
    r = Rectangle
        width: 4
        height: 6
    c.area!.println!
    r.area!.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "75");
    assert_eq!(lines[1], "24");
}

#[test]
fn test_trait_default_methods() {
    let output = compile_and_run(
        r#"
struct Dog
    < name: I64

struct Cat
    < name: I64

trait Animal
    @speak = -> 0
    @legs = -> 4

impl Animal for Dog
    @speak = -> 1

impl Animal for Cat
    @speak = -> 2

main = ->
    d = Dog
        name: 10
    c = Cat
        name: 20
    d.speak!.println!
    c.speak!.println!
    d.legs!.println!
    c.legs!.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "2");
    assert_eq!(lines[2], "4");
    assert_eq!(lines[3], "4");
}

#[test]
fn test_enum_pattern_matching() {
    let output = compile_and_run(
        r#"
enum Option
    Some I64
    None

unwrap_or = x, default ->
    match x
        Option::Some val => val
        Option::None => default

main = ->
    a = Option::Some 42
    b = Option::None
    r1 = unwrap_or a, 0
    r2 = unwrap_or b, 99
    r1.println!
    r2.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "42");
    assert_eq!(lines[1], "99");
}

#[test]
fn test_enum_multiple_variants() {
    let output = compile_and_run(
        r#"
enum Color
    Red
    Green
    Blue

to_num = c ->
    match c
        Color::Red => 1
        Color::Green => 2
        Color::Blue => 3

main = ->
    r = Color::Red
    g = Color::Green
    b = Color::Blue
    to_num r .println!
    to_num g .println!
    to_num b .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "2");
    assert_eq!(lines[2], "3");
}

#[test]
fn test_unqualified_same_named_enum_variant_pattern_uses_scrutinee_owner() {
    let output = compile_and_run(
        r#"
enum AlphaChoice
    Hit Bool
    Miss

enum DecoyChoiceA
    Hit Bool
    Miss

enum DecoyChoiceB
    Hit Bool
    Miss

enum DecoyChoiceC
    Hit Bool
    Miss

enum DecoyChoiceD
    Hit Bool
    Miss

enum DecoyChoiceE
    Hit Bool
    Miss

enum DecoyChoiceF
    Hit Bool
    Miss

enum DecoyChoiceG
    Hit Bool
    Miss

enum DecoyChoiceH
    Hit Bool
    Miss

enum DecoyChoiceI
    Hit Bool
    Miss

enum DecoyChoiceJ
    Hit Bool
    Miss

enum BetaChoice
    Hit I64
    Miss

score: BetaChoice -> I64
score = choice ->
    match choice
        Hit value => value + 1
        Miss => 0

main = ->
    score (BetaChoice::Hit 41) .println!
    0
"#,
    );
    assert_eq!(output.trim(), "42");
}

#[test]
fn test_unqualified_enum_variant_pattern_rejects_known_non_enum_scrutinee() {
    compile_should_fail(
        r#"
enum Choice
    Hit

main = ->
    match 1
        Hit => 1
        _ => 0
"#,
        "pattern type mismatch",
    );
}

#[test]
fn test_qualified_enum_variant_pattern_rejects_different_enum_with_same_variant_name() {
    compile_should_fail(
        r#"
enum A
    Hit I64
    Miss

enum B
    Hit I64
    Miss

score: A -> I64
score = value ->
    match value
        B::Hit n => n
        A::Hit n => n + 1
        A::Miss => 0

main = ->
    score (A::Hit 41)
"#,
        "pattern type mismatch",
    );
}

#[test]
fn test_unqualified_enum_variant_pattern_rejects_variant_missing_from_known_enum() {
    compile_should_fail(
        r#"
enum A
    Hit

enum B
    OnlyOnB

score: A -> I64
score = value ->
    match value
        OnlyOnB => 1
        Hit => 0

main = ->
    score A::Hit
"#,
        "pattern type mismatch",
    );
}

#[test]
fn test_comprehensive_program_v2() {
    let output = compile_and_run(
        r#"
struct Matrix
    < a: I64
    < b: I64
    < c: I64
    < d: I64

impl Matrix
    @det = -> @a * @d - @b * @c
    @mul = other ->
        Matrix
            a: @a * other.a + @b * other.c
            b: @a * other.b + @b * other.d
            c: @c * other.a + @d * other.c
            d: @c * other.b + @d * other.d

enum Result
    Ok I64
    Err I64

safe_div = a, b ->
    if b == 0
        Result::Err 0
    else
        Result::Ok (a / b)

unwrap = r ->
    match r
        Result::Ok val => val
        Result::Err _ => 0 - 1

gcd = a, b ->
    while b > 0
        t = b
        b = a % b
        a = t
    a

is_prime = n ->
    if n < 2
        0
    else
        i = 2
        result = 1
        while i * i <= n
            if n % i == 0
                result = 0
            i = i + 1
        result

main = ->
    m1 = Matrix
        a: 1
        b: 2
        c: 3
        d: 4
    m2 = Matrix
        a: 5
        b: 6
        c: 7
        d: 8
    m1.det!.println!
    m3 = m1.mul m2
    m3.a.println!
    m3.d.println!
    r1 = safe_div 10, 3
    r2 = safe_div 10, 0
    unwrap r1 .println!
    unwrap r2 .println!
    gcd 48, 18 .println!
    is_prime 7 .println!
    is_prime 15 .println!
    is_prime 97 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "-2"); // det(1,2,3,4) = 1*4-2*3
    assert_eq!(lines[1], "19"); // m3.a = 1*5+2*7
    assert_eq!(lines[2], "50"); // m3.d = 3*6+4*8
    assert_eq!(lines[3], "3"); // 10/3
    assert_eq!(lines[4], "-1"); // safe_div 10,0 -> Err -> -1
    assert_eq!(lines[5], "6"); // gcd(48,18)
    assert_eq!(lines[6], "1"); // is_prime(7)
    assert_eq!(lines[7], "0"); // is_prime(15)
    assert_eq!(lines[8], "1"); // is_prime(97)
}

#[test]
fn test_stdlib_try_protocol_methods() {
    let output = compile_and_run(
        r#"
main: () -> I64
main = ->
    input: Result I64, I64 = Result::Ok 41
    res = input.branch!
    value = match res
        ControlFlow::Continue value => value + 1
        ControlFlow::Break residual => 0
    value.println!
    0
"#,
    );
    assert_eq!(output.trim(), "42");
}

#[test]
fn test_stdlib_into_uses_from_blanket_impl() {
    let output = compile_and_run(
        r#"
struct Small
    < value: I64

struct Big
    < value: I64

impl From Small for Big
    from = small ->
        Big
            value: small.value + 1

main = ->
    small = Small
        value: 41
    big: Big = small.into!
    big.value.println!
    0
"#,
    );
    assert_eq!(output.trim(), "42");
}

#[test]
fn test_stdlib_fixed_u8_arrays_convert_to_string_for_any_length() {
    let output = compile_and_run(
        r#"
main = ->
    short = [82 as U8, 111 as U8, 99 as U8, 107 as U8]
    short_string: String = String::from (&short)
    short_string.println!

    long = [72 as U8, 101 as U8, 108 as U8, 108 as U8, 111 as U8, 44 as U8, 32 as U8, 82 as U8, 111 as U8, 99 as U8, 107 as U8]
    long_string: String = String::from (&long)
    long_string.println!

    from_bytes = [70 as U8, 114 as U8, 111 as U8, 109 as U8]
    from_string: String = String::from (&from_bytes)
    from_string.println!

    empty: [U8; 0] = []
    empty_string: String = String::from (&empty)
    empty_string.len!.println!
    0
"#,
    );

    assert_eq!(output, "Rock\nHello, Rock\nFrom\n0\n");
}

#[test]
fn constructor_trait_selection_option_pure_compiles() {
    compile_should_pass(
        r#"
trait Applicative for F _
    pure: A -> F A

enum Maybe T
    None
    Some T

impl Applicative for Maybe
    pure = value -> Maybe::Some value

main = ->
    value = Maybe::pure 42
    0
"#,
    );
}

#[test]
fn constructor_trait_selection_generic_call_specializes_before_mir() {
    compile_should_pass(
        r#"
trait Applicative for F _
    pure: A -> F A

enum Maybe T
    None
    Some T

impl Applicative for Maybe
    pure = value -> Maybe::Some value

repure: F I64 -> F I64 where F _: Applicative
repure = ignored -> F::pure 42

main = ->
    value = repure (Maybe::Some 1)
    0
"#,
    );
}

#[test]
fn constructor_trait_selection_result_section_specializes_before_llvm() {
    compile_should_pass(
        r#"
trait Applicative for F _
    pure: A -> F A

enum Outcome T, E
    Ok T
    Err E

struct IoError

impl Applicative for Outcome _, E
    pure = value -> Outcome::Ok value

repure: Outcome I64, E -> Outcome I64, E
repure = ignored -> (Outcome _, E)::pure 42

main = ->
    input: Outcome I64, IoError = Outcome::Ok 1
    value = repure input
    0
"#,
    );
}

#[test]
fn constructor_trait_selection_rejects_bare_binary_constructor_for_unary_trait() {
    compile_should_fail(
        r#"
trait Applicative for F _
    pure: A -> F A

enum Outcome T, E
    Ok T
    Err E

impl Applicative for Outcome _, E
    pure = value -> Outcome::Ok value

main = ->
    value = Outcome::pure 42
    0
"#,
        "has kind Type -> Type -> Type",
    );
}

#[test]
fn test_try_custom_carrier_uses_try_and_from_residual() {
    let output = compile_and_run(
        r#"
enum MyFlow T
    Value T
    Stop I64

enum MyResidual
    Stop I64

impl Try for MyFlow T
    type Output = T
    type Residual = MyResidual

    ~@branch = ->
        match self
            MyFlow::Value value => ControlFlow::Continue value
            MyFlow::Stop code => ControlFlow::Break (MyResidual::Stop code)

impl FromResidual MyResidual for MyFlow T
    from_residual = residual ->
        match residual
            MyResidual::Stop code => MyFlow::Stop code

next: Bool -> MyFlow I64
next = ok ->
    if ok
        MyFlow::Value 41
    else
        MyFlow::Stop 7

compute: Bool -> MyFlow I64
compute = ok ->
    value = next ok?
    MyFlow::Value (value + 1)

main = ->
    success = compute true
    failure = compute false
    match success
        MyFlow::Value value => value.println!
        MyFlow::Stop code => code.println!
    match failure
        MyFlow::Value value => value.println!
        MyFlow::Stop code => code.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["42", "7"]);
}

#[test]
fn test_try_cross_residual_conversion_uses_distinct_return_carrier() {
    let output = compile_and_run(
        r#"
enum Inner T
    Value T
    Stop I64

enum Outer T
    Value T
    Stop I64

enum InnerResidual
    Stop I64

impl Try for Inner T
    type Output = T
    type Residual = InnerResidual

    ~@branch = ->
        match self
            Inner::Value value => ControlFlow::Continue value
            Inner::Stop code => ControlFlow::Break (InnerResidual::Stop code)

impl FromResidual InnerResidual for Outer T
    from_residual = residual ->
        match residual
            InnerResidual::Stop code => Outer::Stop code

source = ok ->
    if ok
        Inner::Value 41
    else
        Inner::Stop 7

compute = ok ->
    value = source ok?
    Outer::Value (value + 1)

main = ->
    match compute true
        Outer::Value value => value.println!
        Outer::Stop code => code.println!
    match compute false
        Outer::Value value => value.println!
        Outer::Stop code => code.println!
    0
"#,
    );

    assert_eq!(output.trim().lines().collect::<Vec<_>>(), vec!["42", "7"]);
}

#[test]
fn test_try_uses_complete_renamed_language_item_protocol() {
    let exit_code = compile_and_run_without_stdlib(
        r#"
lang control_flow
< enum Flow B, C
    lang break
    Stop B
    lang continue
    Next C

lang try
< trait Carrier
    lang output
    type Value
    lang residual
    type Remainder
    lang branch
    ~@split: Flow Self::Remainder, Self::Value

lang from_residual
< trait Recover R
    lang method
    recover: R -> Self

enum Halt
    Halt I64

enum Maybe T
    Present T
    Absent

impl Carrier for Maybe T
    type Value = T
    type Remainder = Halt

    ~@split = ->
        match self
            Maybe::Present value => Flow::Next value
            Maybe::Absent => Flow::Stop (Halt::Halt 7)

impl Recover Halt for Maybe T
    recover = residual ->
        match residual
            Halt::Halt _ => Maybe::Absent

next: Bool -> Maybe I64
next = ok ->
    if ok
        Maybe::Present 42
    else
        Maybe::Absent

compute: Bool -> Maybe I64
compute = ok ->
    value = next ok?
    Maybe::Present value

main = ->
    success = compute true
    failure = compute false
    match success
        Maybe::Present value =>
            match failure
                Maybe::Present _ => 1
                Maybe::Absent => value
        Maybe::Absent => 2
"#,
    );

    assert_eq!(exit_code, 42);
}

#[test]
fn test_try_does_not_use_unmarked_conventional_protocol_names() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    let source_path = dir.join(format!("no_std_{id}.rk"));
    fs::write(
        &source_path,
        r#"
enum ControlFlow B, C
    Break B
    Continue C

trait Try
    type Output
    type Residual
    ~@branch: ControlFlow Self::Residual, Self::Output

trait FromResidual R
    from_residual: R -> Self

enum Maybe T
    Some T
    None

main = ->
    carrier = Maybe::Some 41
    value = carrier?
    value
"#,
    )
    .unwrap();

    let mut config = test_config(source_path, dir);
    config.extern_artifacts.clear();
    config.no_std = true;
    config.no_prelude = true;
    let diagnostics = rock_lib::compile(&config).expect_err("unmarked protocol must not lower '?'");
    assert!(diagnostics.0.iter().any(|diagnostic| {
        diagnostic.message == "Cannot use '?' because the Try language-item protocol is unavailable"
    }));
}

#[test]
fn test_try_result_error_conversion_uses_from() {
    let output = compile_and_run(
        r#"
enum SmallError
    Bad I64

enum BigError
    Wrapped I64

impl From SmallError for BigError
    from = err ->
        match err
            SmallError::Bad code => BigError::Wrapped (code + 1)

fail: Result I64, SmallError
fail = -> Result::Err (SmallError::Bad 6)

compute: Result I64, BigError
compute = ->
    value = fail!?
    Result::Ok value

main = ->
    result = compute!
    match result
        Result::Ok value => value.println!
        Result::Err err =>
            match err
                BigError::Wrapped code => code.println!
    0
"#,
    );
    assert_eq!(output.trim(), "7");
}

#[test]
fn test_try_result_success_path_unwraps() {
    let output = compile_and_run(
        r#"
maybe_value: Bool -> Result I64, I64
maybe_value = ok ->
    if ok
        Result::Ok 41
    else
        Result::Err 7

compute: Bool -> Result I64, I64
compute = ok ->
    value = maybe_value ok?
    Result::Ok (value + 1)

main = ->
    result = compute true
    match result
        Result::Ok value => value.println!
        Result::Err err => (err + 0).println!
    0
"#,
    );
    assert_eq!(output.trim(), "42");
}

#[test]
fn test_try_result_error_path_returns_early() {
    let output = compile_and_run(
        r#"
maybe_value: Bool -> Result I64, I64
maybe_value = ok ->
    if ok
        Result::Ok 41
    else
        Result::Err 7

compute: Bool -> Result I64, I64
compute = ok ->
    value = maybe_value ok?
    Result::Ok (value + 1)

main = ->
    result = compute false
    match result
        Result::Ok value => value.println!
        Result::Err err => err.println!
    0
"#,
    );
    assert_eq!(output.trim(), "7");
}

#[test]
fn test_try_in_instance_method_uses_declared_method_return_type() {
    let output = compile_and_run(
        r#"
maybe_value: Bool -> Result I64, I64
maybe_value = ok ->
    if ok
        Result::Ok 41
    else
        Result::Err 7

struct Worker

impl Worker
    @compute: Bool -> Result I64, I64
    @compute = ok ->
        value = maybe_value ok?
        Result::Ok (value + 1)

main = ->
    worker = Worker
    success = worker.compute true
    failure = worker.compute false
    match success
        Result::Ok value => value.println!
        Result::Err err => err.println!
    match failure
        Result::Ok value => value.println!
        Result::Err err => err.println!
    0
"#,
    );

    assert_eq!(output.trim().lines().collect::<Vec<_>>(), vec!["42", "7"]);
}

#[test]
fn test_try_in_signature_backed_trait_default_uses_declared_return_type() {
    let output = compile_and_run(
        r#"
maybe_value: Bool -> Result I64, I64
maybe_value = ok ->
    if ok
        Result::Ok 41
    else
        Result::Err 7

struct Worker

trait Computes
    @compute: Bool -> Result I64, I64
    @compute = ok ->
        value = maybe_value ok?
        Result::Ok (value + 1)

impl Computes for Worker

main = ->
    worker = Worker
    success = worker.compute true
    failure = worker.compute false
    match success
        Result::Ok value => value.println!
        Result::Err err => err.println!
    match failure
        Result::Ok value => value.println!
        Result::Err err => err.println!
    0
"#,
    );

    assert_eq!(output.trim().lines().collect::<Vec<_>>(), vec!["42", "7"]);
}

#[test]
fn test_try_option_some_path_unwraps() {
    let output = compile_and_run(
        r#"
maybe_value: Bool -> Option I64
maybe_value = ok ->
    if ok
        Option::Some 41
    else
        Option::None

compute: Bool -> Option I64
compute = ok ->
    value = maybe_value ok?
    Option::Some (value + 1)

main = ->
    result = compute true
    match result
        Option::Some value => value.println!
        Option::None => 0.println!
    0
"#,
    );
    assert_eq!(output.trim(), "42");
}

#[test]
fn test_try_option_none_path_returns_early() {
    let output = compile_and_run(
        r#"
maybe_value: Bool -> Option I64
maybe_value = ok ->
    if ok
        Option::Some 41
    else
        Option::None

compute: Bool -> Option I64
compute = ok ->
    value = maybe_value ok?
    Option::Some (value + 1)

main = ->
    result = compute false
    match result
        Option::Some value => value.println!
        Option::None => 7.println!
    0
"#,
    );
    assert_eq!(output.trim(), "7");
}

#[test]
fn test_try_non_carrier_reports_diagnostic() {
    compile_should_fail(
        r#"
main = ->
    value = 1?
    value
"#,
        "Cannot use '?' on non-carrier type",
    );
}

#[test]
fn test_signatureless_method_receiver_is_constrained_by_call_site() {
    let output = compile_and_run(
        r#"
run_mode = mode -> mode.as_str!

main = ->
    mode = String::from_str "listen"
    run_mode &mode .println!
    0
"#,
    );

    assert_eq!(output.trim(), "listen");
}

#[test]
fn test_signatureless_higher_order_helper_is_instantiated_at_two_targets() {
    let output = compile_and_run(
        r#"
apply = value, mapper -> mapper value

increment = value -> value + 1
identity_bool = value ->
    if value
        true
    else
        false

main = ->
    first = apply 41, increment
    second = apply true, identity_bool
    first.println!
    second.println!
    0
"#,
    );

    assert_eq!(
        output.trim().lines().collect::<Vec<_>>(),
        vec!["42", "true"]
    );
}

#[test]
fn test_signatureless_curried_partial_application_propagates_nested_return() {
    compile_should_pass(
        r#"
enum Wrapped
    Value I64
    Empty

impl Wrapped
    ~@unwrap_or_zero = ->
        match self
            Wrapped::Value value => value
            Wrapped::Empty => 0

wrap = state, value ~>
    Wrapped::Value (state + value)

apply = value, mapper -> mapper value

main = ->
    wrapped = apply 42, (wrap 0)
    wrapped.unwrap_or_zero!.println!
    0
"#,
    );
}

#[test]
fn test_signatureless_callback_infers_mut_reference_parameter() {
    let output = compile_and_run(
        r#"
dispatch = mut target, callback ->
    callback target
    0

main = ->
    mut value = 0
    dispatch &mut value, (slot -> *slot = 41)
    value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "41");
}

#[test]
fn test_signatureless_forward_try_and_hkt_map_are_inferred() {
    let output = compile_and_run(
        r#"
finish_connection = ->
    value = source!?
    wrapped = if true
        Result::Ok value
    else
        Result::Err 0
    wrapped.map (item -> item + 1)

source = ->
    if true
        Result::Ok 41
    else
        Result::Err 0

main = ->
    result = finish_connection!
    match result
        Result::Ok value => value.println!
        Result::Err err => err.println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_signatureless_chained_hkt_operators_defer_unknown_constructor_heads() {
    let output = compile_and_run(
        r#"
source = ->
    if true
        Result::Ok 41
    else
        Result::Err 0
convert_error = value -> value == 0

finish = ->
    total = 1
    source!
        <!> convert_error
        <&> _ -> total + 41

main = ->
    finish!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "Ok(42)");
}

#[test]
fn test_signatureless_deferred_method_argument_uses_call_parameter_type() {
    let output = compile_and_run(
        r#"
struct Left
struct Right

impl Left
    @make = -> 41

impl Right
    @make = -> true

consume: I64 -> Result I64, I64
consume = value -> Result::Ok value

wrapper = value ->
    result = consume value.make!
    result

main = ->
    wrapper Left .println!
    0
"#,
    );
    assert_eq!(output.trim(), "Ok(41)");
}

#[test]
fn test_signatureless_deferred_method_constrains_argument_from_concrete_parameter() {
    let output = compile_and_run(
        r#"
struct Sink

impl Sink
    @write: I64 -> I64
    @write = value -> value

    @write_bytes: &mut [U8] -> I64
    @write_bytes = _ -> 42

struct Wrapper
    < sink: Sink

impl Wrapper
    @forward = value -> self.sink.write value

    @forward_bytes = ->
        mut bytes = [0; 4]
        self.sink.write_bytes &mut bytes

forward_unknown = sink ->
    mut bytes = [0; 4]
    sink.write_bytes &mut bytes

main = ->
    wrapper = Wrapper
        sink: Sink
    wrapper.forward 41 .println!
    wrapper.forward_bytes!.println!
    forward_unknown Sink .println!
    0
"#,
    );

    assert_eq!(
        output.trim().lines().collect::<Vec<_>>(),
        vec!["41", "42", "42"]
    );
}

#[test]
fn test_signatureless_ambiguous_method_still_reports_diagnostic() {
    compile_should_fail(
        r#"
trait First
    @ping: () -> I64

trait Second
    @ping: () -> I64

struct Service

impl First for Service
    @ping = -> 1

impl Second for Service
    @ping = -> 2

run = value -> value.ping!

main = ->
    run Service
    0
"#,
        "Ambiguous selection",
    );
}

#[test]
fn test_signatureless_mutual_recursion_remains_monomorphic() {
    let output = compile_and_run(
        r#"
even = value ->
    if value == 0
        true
    else
        odd (value - 1)

odd = value ->
    if value == 0
        false
    else
        even (value - 1)

main = ->
    even 10 .println!
    odd 10 .println!
    0
"#,
    );

    assert_eq!(
        output.trim().lines().collect::<Vec<_>>(),
        vec!["true", "false"]
    );
}

#[test]
fn test_signatureless_direct_recursion_registers_specialization_before_body() {
    let output = compile_and_run(
        r#"
count_down = value ->
    if value == 0
        42
    else
        count_down (value - 1)

main = ->
    count_down 3 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_try_in_inferred_return_function_is_inferred_successfully() {
    let output = compile_and_run(
        r#"
fallible: () -> Result I64, I64
fallible = -> Result::Ok 41

compute = ->
    value = fallible!?
    if true
        Result::Ok (value + 1)
    else
        Result::Err 0

main = ->
    result = compute!
    match result
        Result::Ok value => value.println!
        Result::Err err => err.println!
    0
"#,
    );
    assert_eq!(output.trim(), "42");
}

#[test]
fn test_try_inferred_later_defined_helper_is_materialized_after_body_unification() {
    let output = compile_and_run(
        r#"
compute = ->
    value = later true?
    if true
        Result::Ok (value + 1)
    else
        Result::Err 0

later = ok ->
    if ok
        Result::Ok 41
    else
        Result::Err 7

main = ->
    result = compute!
    match result
        Result::Ok value => value.println!
        Result::Err err => err.println!
    0
"#,
    );
    assert_eq!(output.trim(), "42");
}

#[test]
fn test_try_inferred_impl_method_materializes_before_method_generalization() {
    let output = compile_and_run(
        r#"
maybe_value: Bool -> Result I64, I64
maybe_value = ok ->
    if ok
        Result::Ok 41
    else
        Result::Err 7

struct Worker

impl Worker
    @compute = ok ->
        value = maybe_value ok?
        if true
            Result::Ok (value + 1)
        else
            Result::Err 0

main = ->
    worker = Worker
    result = worker.compute true
    match result
        Result::Ok value => value.println!
        Result::Err err => err.println!
    0
"#,
    );
    assert_eq!(output.trim(), "42");
}

#[test]
fn test_try_inferred_function_generalizes_unrelated_parameter() {
    let output = compile_and_run(
        r#"
fallible: () -> Result I64, I64
fallible = -> Result::Ok 41

compute = ignored ->
    value = fallible!?
    if true
        Result::Ok (value + 1)
    else
        Result::Err 0

main = ->
    first = compute 1
    second = compute true
    match first
        Result::Ok value => value.println!
        Result::Err err => err.println!
    match second
        Result::Ok value => value.println!
        Result::Err err => err.println!
    0
"#,
    );
    assert_eq!(output.trim().lines().collect::<Vec<_>>(), vec!["42", "42"]);
}

#[test]
fn test_try_deferred_non_carrier_reports_diagnostic() {
    compile_should_fail(
        r#"
compute = ->
    value = later!?
    if true
        Result::Ok value
    else
        Result::Err 0

later = -> 1

main = ->
    compute!
    0
"#,
        "Cannot use '?' on non-carrier type",
    );
}

#[test]
fn test_try_deferred_from_residual_reports_diagnostic() {
    compile_should_fail(
        r#"
maybe: () -> Option I64
maybe = -> Option::None

compute = ->
    value = maybe!?
    if true
        Result::Ok value
    else
        Result::Err 0

main = ->
    compute!
    0
"#,
        "FromResidual",
    );
}

#[test]
fn test_try_deferred_unsafe_branch_requires_unsafe_block() {
    compile_should_fail(
        r#"
enum Carrier T
    Value T
    Stop I64

enum Residual
    Stop I64

impl Try for Carrier T
    type Output = T
    type Residual = Residual

    unsafe ~@branch = ->
        match self
            Carrier::Value value => ControlFlow::Continue value
            Carrier::Stop code => ControlFlow::Break (Residual::Stop code)

impl FromResidual Residual for Carrier T
    from_residual = residual ->
        match residual
            Residual::Stop code => Carrier::Stop code

compute = ->
    value = source!?
    if true
        Carrier::Value value
    else
        Carrier::Stop 0

source: () -> Carrier I64
source = -> Carrier::Value 41

main = ->
    compute!
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_try_deferred_unsafe_branch_is_allowed_inside_unsafe_block() {
    let output = compile_and_run(
        r#"
enum Carrier T
    Value T
    Stop I64

enum Residual
    Stop I64

impl Try for Carrier T
    type Output = T
    type Residual = Residual

    unsafe ~@branch = ->
        match self
            Carrier::Value value => ControlFlow::Continue value
            Carrier::Stop code => ControlFlow::Break (Residual::Stop code)

impl FromResidual Residual for Carrier T
    from_residual = residual ->
        match residual
            Residual::Stop code => Carrier::Stop code

compute = ->
    value = unsafe
        source!?
    if true
        Carrier::Value value
    else
        Carrier::Stop 0

source: () -> Carrier I64
source = -> Carrier::Value 41

main = ->
    compute!
    0
"#,
    );
    assert_eq!(output.trim(), "");
}

#[test]
fn test_try_in_lambda_uses_lambda_return_type_not_outer_function() {
    let output = compile_and_run(
        r#"
maybe = -> Option::Some 41

outer: () -> Result I64, I64
outer = ->
    convert = ->
        value = maybe!?
        if true
            Option::Some value
        else
            Option::None
    ignored = convert!
    Result::Ok 42

main = ->
    result = outer!
    match result
        Result::Ok value => value.println!
        Result::Err err => err.println!
    0
"#,
    );
    assert_eq!(output.trim(), "42");
}

#[test]
fn test_try_generic_carrier_is_rejected_without_ambiguous_selection() {
    compile_should_fail(
        r#"
compute = value ->
    extracted = value?
    extracted

main = -> 0
"#,
        "carrier type is unresolved or generic",
    );
}

#[test]
fn test_try_option_to_result_without_from_residual_reports_diagnostic() {
    compile_should_fail(
        r#"
maybe: Option I64
maybe = -> Option::None

main: Result I64, I64
main = ->
    value = maybe!?
    Result::Ok value
"#,
        "FromResidual",
    );
}

#[test]
fn test_higher_order_functions() {
    let output = compile_and_run(
        r#"
double = x -> x * 2
triple = x -> x * 3

apply = f, x -> f x

main = ->
    apply double, 5 .println!
    apply triple, 5 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "15");
}

#[test]
fn test_higher_order_function_with_capturing_lambda() {
    let output = compile_and_run(
        r#"
apply = f, x -> f x

main = ->
    base = 10
    apply (y -> y + base), 5 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "15");
}

#[test]
fn test_curried_function_partial_application() {
    let output = compile_and_run(
        r#"
add = a, b ~> a + b

main = ->
    inc = add 1
    inc 2 .println!
    add 1, 2 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "3");
    assert_eq!(lines[1], "3");
}

#[test]
fn test_curried_function_with_explicit_signature() {
    let output = compile_and_run(
        r#"
add: I64 -> I64 -> I64
add = a, b ~> a + b

main = ->
    inc = add 2
    inc 5 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "7");
}

#[test]
fn test_curried_function_try_uses_terminal_return_type() {
    let output = compile_and_run(
        r#"
add_result: I64 -> Result I64, I64 -> Result I64, I64
add_result = base, input ~>
    value = input?
    Result::Ok (base + value)

main = ->
    add_ten = add_result 10
    value = (add_ten (Result::Ok 2)).fold
        _ -> 0
        result -> result
    value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "12");
}

#[test]
fn test_zero_arg_method_is_value_until_bang_call() {
    let output = compile_and_run(
        r#"
struct Counter
    < value: I64

impl Counter
    @get = -> self.value

main = ->
    counter = Counter
        value: 21
    get = counter.get
    get!.println!
    counter.get!.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "21");
    assert_eq!(lines[1], "21");
}

#[test]
fn test_curried_method_partial_application() {
    let output = compile_and_run(
        r#"
struct Counter
    < value: I64

impl Counter
    @add = x, y ~> self.value + x + y

main = ->
    counter = Counter
        value: 10
    add_one = counter.add 1
    add_one 2 .println!
    counter.add 1, 2 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "13");
    assert_eq!(lines[1], "13");
}

#[test]
fn test_mut_and_move_self_receivers() {
    let output = compile_and_run(
        r#"
struct Counter
    < value: I64

struct Holder
    < value: I64

impl Counter
    ^@read = -> 41

impl Holder
    ~@consume = -> 7

main = ->
    mut counter = Counter
        value: 41
    holder = Holder
        value: 7
    counter.read!.println!
    holder.consume!.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "41");
    assert_eq!(lines[1], "7");
}

#[test]
fn test_recursive_functions() {
    let output = compile_and_run(
        r#"
factorial = n ->
    if n <= 1
        1
    else
        n * (factorial (n - 1))

fib = n ->
    if n <= 1
        n
    else
        (fib (n - 1)) + (fib (n - 2))

main = ->
    factorial 5 .println!
    factorial 10 .println!
    fib 10 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "120");
    assert_eq!(lines[1], "3628800");
    assert_eq!(lines[2], "55");
}

#[test]
fn test_logical_operators() {
    let output = compile_and_run(
        r#"
main = ->
    a = 5
    b = 10
    if a > 3 && b < 20
        1.println!
    else
        0.println!
    if a > 10 || b > 5
        1.println!
    else
        0.println!
    if a > 10 && b > 100
        1.println!
    else
        0.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1"); // 5>3 && 10<20 = true
    assert_eq!(lines[1], "1"); // 5>10 || 10>5 = true
    assert_eq!(lines[2], "0"); // 5>10 && 10>100 = false
}

#[test]
fn test_logical_operators_short_circuit_rhs_bounds_checks() {
    let (output, success) = compile_and_run_with_status(
        r#"
main = ->
    arr = [1]
    if false && arr[1] == 0
        99.println!
    else
        1.println!
    if true || arr[1] == 0
        2.println!
    else
        99.println!
    0
"#,
    );

    assert!(
        success,
        "short-circuited RHS should not evaluate bounds checks"
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["1", "2"]);
}

#[test]
fn test_tuples() {
    let output = compile_and_run(
        r#"
swap = a, b -> (b, a)

main = ->
    t = (1, 2, 3)
    t.0.println!
    t.1.println!
    t.2.println!
    s = swap 10, 20
    s.0.println!
    s.1.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "2");
    assert_eq!(lines[2], "3");
    assert_eq!(lines[3], "20");
    assert_eq!(lines[4], "10");
}

#[test]
fn test_collatz() {
    let output = compile_and_run(
        r#"
collatz_steps = n ->
    steps = 0
    while n > 1
        if n % 2 == 0
            n = n / 2
        else
            n = n * 3 + 1
        steps = steps + 1
    steps

main = ->
    collatz_steps 1 .println!
    collatz_steps 6 .println!
    collatz_steps 27 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "0");
    assert_eq!(lines[1], "8");
    assert_eq!(lines[2], "111");
}

#[test]
fn test_vec_get_returns_optional_reference() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 10
    v.push 20

    match (v.get 0)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    match (v.get 5)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["10", "-1"]);
}

#[test]
fn test_vec_get_borrow_blocks_later_mutation() {
    compile_should_fail(
        r#"
main = ->
    mut v = Vec::new!
    v.push 1
    saved = v.get 0
    v.push 2
    match saved
        Option::Some val => *val .println!
        Option::None => 0.println!
    0
"#,
        "borrow",
    );
}

#[test]
fn vec_deref_does_not_return_reference_to_temporary_slice() {
    compile_should_fail(
        r#"
> stdlib::vec::Vec

bad: Vec I64 -> &I64
bad = v -> &v.len!

main = ->
    mut v = Vec::new!
    v.push 1
    rr = bad v
    0
"#,
        "temporary",
    );
}

#[test]
fn test_vec_push() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 1
    v.push 2
    v.push 3
    v.len! .println!

    match (v.get 0)
        Option::Some val => *val .println!
        Option::None => 0.println!
    match (v.get 1)
        Option::Some val => *val .println!
        Option::None => 0.println!
    match (v.get 2)
        Option::Some val => *val .println!
        Option::None => 0.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "3");
    assert_eq!(lines[1], "1");
    assert_eq!(lines[2], "2");
    assert_eq!(lines[3], "3");
}

#[test]
fn test_stdlib_vec_show_formats_owned_elements() {
    let output = compile_and_run(
        r#"
main = ->
    mut values: Vec String = Vec::new!
    values.push (String::from_str "alpha")
    values.push (String::from_str "two words")
    values.println!
    0
"#,
    );

    assert_eq!(output.trim(), "[alpha, two words]");
}

#[test]
fn test_stdlib_vec_rejects_zero_sized_elements_on_push() {
    let (_output, success) = compile_and_run_with_status(
        r#"
struct Empty

main = ->
    mut v = Vec::new!
    v.push Empty
    0
"#,
    );

    assert!(!success, "Vec::push should reject zero-sized elements");
}

#[test]
fn test_stdlib_vec_new_as_slice_uses_non_null_buffer() {
    let output = compile_and_run(
        r#"
main = ->
    v: Vec I64 = Vec::new!
    slice = v.as_slice!
    ((~ArrPtr (*slice)) as I64) != 0 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "true");
}

#[test]
fn test_stdlib_vec_push_uses_non_null_buffer() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 1
    slice = v.as_slice!
    ((~ArrPtr (*slice)) as I64) != 0 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "true");
}

#[test]
fn test_stdlib_vec_index_works() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 4
    v.push 7
    v[1].println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_stdlib_vec_index_out_of_bounds_traps() {
    let (_output, success) = compile_and_run_with_status(
        r#"
main = ->
    mut v = Vec::new!
    v.push 10
    v[1].println!
    0
"#,
    );

    assert!(!success, "Vec index out of bounds should trap");
}

#[test]
fn test_stdlib_vec_shared_binding_index_works() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 4
    v.push 7
    shared = v
    shared[1].println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_hash_map_insert_get_len_and_contains() {
    let output = compile_and_run(
        r#"
main = ->
    mut map = HashMap::new!
    map.len! .println!
    map.insert 10, 100
    map.insert 20, 200
    map.len! .println!
    ten = 10
    twenty = 20
    thirty = 30
    map.contains_key &ten .println!
    map.contains_key &thirty .println!

    match (map.get &ten)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    match (map.get &twenty)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    match (map.get &thirty)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["0", "2", "true", "false", "100", "200", "-1"],);
}

#[test]
fn test_stdlib_hash_map_rejects_zero_sized_values_on_insert() {
    let (_output, success) = compile_and_run_with_status(
        r#"
struct Empty

main = ->
    mut map = HashMap::new!
    map.insert 1, Empty
    0
"#,
    );

    assert!(!success, "HashMap::insert should reject zero-sized values");
}

#[test]
fn test_stdlib_hash_map_rejects_zero_sized_keys_on_insert() {
    let (_output, success) = compile_and_run_with_status(
        r#"
struct Empty

impl Eq for Empty
    @== = other -> true
    @!= = other -> false

impl Hash for Empty
    @hash = -> 0

main = ->
    mut map = HashMap::new!
    map.insert Empty, 1
    0
"#,
    );

    assert!(!success, "HashMap::insert should reject zero-sized keys");
}

#[test]
fn test_generic_eq_bound_dispatches_binary_operator() {
    let output = compile_and_run(
        r#"
struct Box T
    < value: T

same_boxed: Box T -> Box T -> Bool where T: Eq
same_boxed = left, right -> left.value == right.value

main = ->
    a = Box
        value: 7
    a2 = Box
        value: 7
    b = Box
        value: 7
    c = Box
        value: 9
    same_boxed a, b .println!
    same_boxed a2, c .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["true", "false"]);
}

#[test]
fn test_generic_eq_bound_borrows_rhs_without_move() {
    let output = compile_and_run(
        r#"
struct Tracked
    < id: I64

impl Eq for Tracked
    @== = other -> self.id == (*other).id
    @!= = other -> self.id != (*other).id

same_then_use_right: T -> T -> T where T: Eq
same_then_use_right = left, right ->
    if left == right
        1
    else
        0
    right

main = ->
    left = Tracked
        id: 7
    right = Tracked
        id: 7
    result = same_then_use_right left, right
    result.id.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["7"]);
}

#[test]
fn test_ord_operator_borrows_rhs_without_move() {
    let output = compile_and_run(
        r#"
struct Tracked
    < id: I64

impl Eq for Tracked
    @== = other -> self.id == (*other).id
    @!= = other -> self.id != (*other).id

impl Ord for Tracked
    @< = other -> self.id < (*other).id
    @<= = other -> self.id <= (*other).id
    @> = other -> self.id > (*other).id
    @>= = other -> self.id >= (*other).id

less_then_use_right: T -> T -> T where T: Ord
less_then_use_right = left, right ->
    if left < right
        1
    else
        0
    right

main = ->
    left = Tracked
        id: 3
    right = Tracked
        id: 9
    result = less_then_use_right left, right
    result.id.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["9"]);
}

#[test]
fn test_generic_eq_operator_uses_explicit_symbol_bound() {
    compile_should_pass(
        r#"
trait SameNameEquals
    @==: Self -> Bool

struct Box T
    < value: T

bad_eq: Box T -> Box T -> Bool where T: SameNameEquals
bad_eq = left, right -> left.value == right.value

main = ->
    0
"#,
    );
}

#[test]
fn test_generic_eq_operator_rejects_unrelated_rhs_type() {
    compile_should_fail(
        r#"
bad_eq: T -> U -> Bool where T: Eq
bad_eq = left, right -> left == right

main = ->
    bad_eq 1, true .println!
    0
"#,
        "Type mismatch",
    );
}

#[test]
fn test_hash_map_insert_overwrites_without_growing_len() {
    let output = compile_and_run(
        r#"
main = ->
    mut map = HashMap::new!
    map.insert 1, 10
    map.insert 1, 99
    map.len! .println!
    one = 1

    match (map.get &one)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["1", "99"]);
}

#[test]
fn test_hash_map_handles_collisions_and_growth() {
    let output = compile_and_run(
        r#"
main = ->
    mut map = HashMap::new!
    map.insert 1, 10
    map.insert 9, 90
    map.insert 17, 170
    map.insert 25, 250
    map.insert 33, 330
    map.insert 41, 410
    map.insert 49, 490

    map.len! .println!
    one = 1
    nine = 9
    forty_nine = 49

    match (map.get &one)
        Option::Some val => *val .println!
        Option::None => -1 .println!
    match (map.get &nine)
        Option::Some val => *val .println!
        Option::None => -1 .println!
    match (map.get &forty_nine)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["7", "10", "90", "490"]);
}

#[test]
fn test_hash_map_distinguishes_unequal_keys_with_same_hash() {
    let output = compile_and_run(
        r#"
struct BadKey
    < id: I64

impl Eq for BadKey
    @== = other -> self.id == (*other).id
    @!= = other -> self.id != (*other).id

impl Hash for BadKey
    @hash = -> 1

main = ->
    mut map = HashMap::new!
    map.insert (BadKey
        id: 1), 10
    map.insert (BadKey
        id: 2), 20
    map.len! .println!

    one = BadKey
        id: 1
    match (map.get &one)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    two = BadKey
        id: 2
    match (map.get &two)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["2", "10", "20"]);
}

#[test]
fn test_hash_map_private_storage_fields_are_rejected() {
    compile_should_fail(
        r#"
main = ->
    map = HashMap::new!
    x = map.raw_len
    0
"#,
        "Field 'raw_len' of struct 'HashMap' is private",
    );
}

#[test]
fn test_hash_map_str_keys() {
    let output = compile_and_run(
        r#"
main = ->
    mut map = HashMap::new!
    map.insert "red", 1
    map.insert "blue", 2
    map.insert "red", 3
    map.len! .println!
    red = "red"
    green = "green"

    match (map.get &red)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    match (map.get &green)
        Option::Some val => *val .println!
        Option::None => -1 .println!

    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["2", "3", "-1"]);
}

#[test]
fn test_stdlib_hash_map_drops_keys_and_values() {
    let output = compile_and_run(
        r#"
struct TrackedKey
    < id: I64
    < drop_id: I64

impl Eq for TrackedKey
    @== = other -> self.id == (*other).id
    @!= = other -> self.id != (*other).id

impl Hash for TrackedKey
    @hash = -> (*self).id

impl Drop for TrackedKey
    ~@drop = ->
        self.drop_id.println!
        return

struct TrackedValue
    < id: I64

impl Drop for TrackedValue
    ~@drop = ->
        self.id.println!
        return

main = ->
    mut map = HashMap::new!
    map.insert (TrackedKey
        id: 1
        drop_id: 101), (TrackedValue
        id: 10)
    map.insert (TrackedKey
        id: 2
        drop_id: 102), (TrackedValue
        id: 20)
    0
"#,
    );

    let mut lines: Vec<i64> = output
        .trim()
        .lines()
        .map(|line| line.parse::<i64>().unwrap())
        .collect();
    lines.sort();
    assert_eq!(lines, vec![10, 20, 101, 102]);
}

#[test]
fn test_stdlib_hash_map_overwrite_drops_old_value_and_unused_key_once() {
    let output = compile_and_run(
        r#"
struct TrackedKey
    < id: I64
    < drop_id: I64

impl Eq for TrackedKey
    @== = other -> self.id == (*other).id
    @!= = other -> self.id != (*other).id

impl Hash for TrackedKey
    @hash = -> (*self).id

impl Drop for TrackedKey
    ~@drop = ->
        self.drop_id.println!
        return

struct TrackedValue
    < id: I64

impl Drop for TrackedValue
    ~@drop = ->
        self.id.println!
        return

main = ->
    mut map = HashMap::new!
    map.insert (TrackedKey
        id: 1
        drop_id: 101), (TrackedValue
        id: 10)
    map.insert (TrackedKey
        id: 1
        drop_id: 102), (TrackedValue
        id: 20)
    0
"#,
    );

    let lines: Vec<i64> = output
        .trim()
        .lines()
        .map(|line| line.parse::<i64>().unwrap())
        .collect();
    assert_eq!(lines, vec![10, 102, 20, 101]);
}

#[test]
fn test_stdlib_hash_map_get_and_contains_borrow_probe_key_without_consuming() {
    let output = compile_and_run(
        r#"
struct TrackedKey
    < id: I64
    < drop_id: I64

impl Eq for TrackedKey
    @== = other -> self.id == (*other).id
    @!= = other -> self.id != (*other).id

impl Hash for TrackedKey
    @hash = -> (*self).id

impl Drop for TrackedKey
    ~@drop = ->
        self.drop_id.println!
        return

struct TrackedValue
    < id: I64

impl Drop for TrackedValue
    ~@drop = ->
        self.id.println!
        return

main = ->
    mut map = HashMap::new!
    map.insert (TrackedKey
        id: 1
        drop_id: 101), (TrackedValue
        id: 10)

    get_probe = TrackedKey
        id: 1
        drop_id: 201
    match (map.get &get_probe)
        Option::Some val => (*val).id.println!
        Option::None => -1 .println!
    get_probe.id.println!

    contains_probe = TrackedKey
        id: 1
        drop_id: 202
    map.contains_key &contains_probe .println!
    contains_probe.id.println!

    missing_probe = TrackedKey
        id: 2
        drop_id: 203
    match (map.get &missing_probe)
        Option::Some val => (*val).id.println!
        Option::None => -1 .println!
    missing_probe.id.println!

    second_get_probe = TrackedKey
        id: 1
        drop_id: 204
    match (map.get &second_get_probe)
        Option::Some val => (*val).id.println!
        Option::None => -1 .println!
    second_get_probe.id.println!

    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(
        lines,
        vec!["10", "1", "true", "1", "-1", "2", "10", "1", "204", "203", "202", "201", "10", "101"]
    );
}

#[test]
fn test_stdlib_hash_map_growth_drops_moved_keys_and_values_once() {
    let output = compile_and_run(
        r#"
struct TrackedKey
    < id: I64
    < drop_id: I64

impl Eq for TrackedKey
    @== = other -> self.id == (*other).id
    @!= = other -> self.id != (*other).id

impl Hash for TrackedKey
    @hash = -> (*self).id

impl Drop for TrackedKey
    ~@drop = ->
        self.drop_id.println!
        return

struct TrackedValue
    < id: I64

impl Drop for TrackedValue
    ~@drop = ->
        self.id.println!
        return

main = ->
    mut map = HashMap::new!
    map.insert (TrackedKey
        id: 1
        drop_id: 101), (TrackedValue
        id: 10)
    map.insert (TrackedKey
        id: 2
        drop_id: 102), (TrackedValue
        id: 20)
    map.insert (TrackedKey
        id: 3
        drop_id: 103), (TrackedValue
        id: 30)
    map.insert (TrackedKey
        id: 4
        drop_id: 104), (TrackedValue
        id: 40)
    map.insert (TrackedKey
        id: 5
        drop_id: 105), (TrackedValue
        id: 50)
    map.insert (TrackedKey
        id: 6
        drop_id: 106), (TrackedValue
        id: 60)
    map.insert (TrackedKey
        id: 7
        drop_id: 107), (TrackedValue
        id: 70)
    0
"#,
    );

    let mut lines: Vec<i64> = output
        .trim()
        .lines()
        .map(|line| line.parse::<i64>().unwrap())
        .collect();
    lines.sort();
    assert_eq!(
        lines,
        vec![10, 20, 30, 40, 50, 60, 70, 101, 102, 103, 104, 105, 106, 107],
    );
}

#[test]
fn test_vec_as_slice_index_works_without_stored_slice_field() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 10
    v.push 20
    slice = v.as_slice!
    slice[1] .println!
    0
"#,
    );

    assert_eq!(output.trim(), "20");
}

#[test]
fn test_vec_as_slice_borrows_existing_buffer() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 11
    v.push 22
    slice = v.as_slice!
    slice[1] .println!
    0
"#,
    );

    assert_eq!(output.trim(), "22");
}

#[test]
fn test_vec_as_slice_borrow_blocks_later_mutation() {
    compile_should_fail(
        r#"
main = ->
    mut v = Vec::new!
    v.push 1
    slice = v.as_slice!
    v.push 2
    slice[0] .println!
    0
"#,
        "borrow",
    );
}

#[test]
fn test_string_len_method() {
    let output = compile_and_run(
        r#"
main = ->
    s = String::from_str "hello"
    s.len! .println!
    0
"#,
    );

    assert_eq!(output.trim(), "5");
}

#[test]
fn test_move_non_copy_value_out_of_shared_reference_is_rejected() {
    compile_should_fail(
        r#"
main = ->
    s = String::from_str "hello"
    r = &s
    moved = *r
    0
"#,
        "Cannot move non-copy value",
    );
}

#[test]
fn test_borrowed_enum_match_payload_binding_is_shared_reference() {
    let output = compile_and_run(
        r#"
show_option: &Option String -> I64
show_option = opt ->
    match *opt
        Option::Some value => value.len!
        Option::None => 0

main = ->
    value = Option::Some (String::from_str "hello")
    show_option &value .println!
    show_option &value .println!
    0
"#,
    );

    assert_eq!(output.trim(), "5\n5");
}

#[test]
fn test_borrowed_enum_match_payload_field_projection_uses_referent() {
    let output = compile_and_run(
        r#"
struct Boxed
    < text: String

show_boxed: &Option Boxed -> I64
show_boxed = opt ->
    match *opt
        Option::Some value => value.text.len!
        Option::None => 0

main = ->
    value = Option::Some (Boxed
        text: String::from_str "hello")
    show_boxed &value .println!
    show_boxed &value .println!
    0
"#,
    );

    assert_eq!(output.trim(), "5\n5");
}

#[test]
fn test_borrowed_enum_match_reference_payload_binding_uses_payload_type() {
    let output = compile_and_run(
        r#"
read_option_ref: &Option &I64 -> I64
read_option_ref = opt ->
    match *opt
        Option::Some value => *value
        Option::None => 0

main = ->
    number = 42
    value = Option::Some (&number)
    read_option_ref &value .println!
    read_option_ref &value .println!
    0
"#,
    );

    assert_eq!(output.trim(), "42\n42");
}

#[test]
fn test_copy_non_cleanup_struct_out_of_shared_reference_is_rejected() {
    compile_should_fail(
        r#"
struct Pair
    < x: I64

main = ->
    p = Pair
        x: 1
    r = &p
    copied = *r
    0
"#,
        "Cannot move non-copy value",
    );
}

#[test]
fn test_vec_raw_len_field_access_is_rejected_outside_impl() {
    compile_should_fail(
        r#"
main = ->
    mut v = Vec::new!
    v.push 1
    x = v.raw_len
    0
"#,
        "Field 'raw_len' of struct 'Vec' is private",
    );
}

#[test]
fn test_string_raw_len_field_access_is_rejected_outside_impl() {
    compile_should_fail(
        r#"
main = ->
    s = String::from_str "hello"
    x = s.raw_len
    0
"#,
        "Field 'raw_len' of struct 'String' is private",
    );
}

#[test]
fn test_generic_function_argument_in_monomorphized_method_call() {
    let output = compile_and_run(
        r#"
identity = x -> x

struct Box T
    value: T

impl Box T
    new = value ->
        Box
            value: value

    @apply = f -> f self.value

main = ->
    b = Box::new 21
    b.apply identity .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "21");
}

#[test]
fn test_generic_instance_call_edges_for_function_and_method() {
    let output = compile_and_run(
        r#"
identity = x -> x

struct Holder T
    value: T

impl Holder T
    new = value ->
        Holder
            value: value

    @apply = f -> f self.value

main = ->
    value = identity 42
    h = Holder::new value
    h.apply identity .println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_generic_method_call_preserves_receiver_specialization() {
    let output = compile_and_run(
        r#"
struct Box T
    value: T

impl Box T
    new = value ->
        Box
            value: value

    @get = -> @value

main = ->
    ib = Box::new 41
    bb = Box::new true
    if bb.get!
        ib.get!.println!
    else
        0.println!
    0
"#,
    );

    assert_eq!(output.trim(), "41");
}

fn compile_to_llvm_ir(source: &str) -> String {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();
    let source_path = dir.join(format!("test_{}.rk", id));
    std::fs::write(&source_path, source).unwrap();
    let mut config = test_config(source_path.clone(), dir.clone());
    config.no_link = true;
    config.emit_llvm = true;
    config.emit_object = Some(dir.join(format!("test_{}.o", id)));

    rock_lib::compile(&config).expect("Compilation failed");

    let module_name = source_path.file_stem().unwrap().to_string_lossy();
    let llvm = std::fs::read_to_string(dir.join(format!("{}.ll", module_name)))
        .expect("Expected LLVM IR output");
    let _ = std::fs::remove_dir_all(&dir);
    llvm
}

fn symbol_fragment_bytes(fragment: &str) -> String {
    let mut mangled = String::new();
    for byte in fragment.as_bytes() {
        mangled.push_str(&format!("{byte:02x}"));
    }
    mangled
}

fn defined_symbol_mentions_fragment(line: &str, fragment: &str) -> bool {
    line.contains(fragment) || line.contains(&symbol_fragment_bytes(fragment))
}

#[test]
fn test_instance_dce_does_not_emit_unused_current_function() {
    let llvm = compile_to_llvm_ir(
        r#"
used = -> 1
unused = -> 2

main = ->
    used! .println!
    0
"#,
    );

    let defined_symbols: Vec<&str> = llvm
        .lines()
        .filter(|line| line.starts_with("define "))
        .collect();
    let used_symbol = "__rock_fn_d0_";
    let unused_symbol = "__rock_fn_d1_";
    assert!(
        defined_symbols
            .iter()
            .any(|line| defined_symbol_mentions_fragment(line, used_symbol)),
        "expected used function definition in LLVM IR: {defined_symbols:?}"
    );
    assert!(
        !defined_symbols
            .iter()
            .any(|line| defined_symbol_mentions_fragment(line, unused_symbol)),
        "unused function should not be defined in LLVM IR: {defined_symbols:?}"
    );
}

#[test]
fn test_instance_dce_does_not_emit_unused_generic_specialization() {
    let llvm = compile_to_llvm_ir(
        r#"
identity = x -> x

use_i64 = -> identity 7
use_i32 = -> identity (8 as I32)

main = ->
    use_i64! .println!
    0
"#,
    );

    let defined_symbols: Vec<&str> = llvm
        .lines()
        .filter(|line| line.starts_with("define "))
        .collect();
    let identity_specializations: Vec<&str> = defined_symbols
        .iter()
        .copied()
        .filter(|line| defined_symbol_mentions_fragment(line, "__rock_fn_d0_"))
        .collect();
    assert_eq!(
        identity_specializations.len(),
        1,
        "only the reachable identity specialization should be defined: {defined_symbols:?}"
    );
    let unused_caller_symbol = "__rock_fn_d2_";
    assert!(
        !defined_symbols
            .iter()
            .any(|line| defined_symbol_mentions_fragment(line, unused_caller_symbol)),
        "unused caller should not be defined in LLVM IR: {defined_symbols:?}"
    );
}

#[test]
fn test_generic_function_argument_app_ir_does_not_define_stdlib_object_symbols() {
    let llvm = compile_to_llvm_ir(
        r#"
identity = x -> x

struct Box T
    value: T

impl Box T
    new = value ->
        Box
            value: value

    @apply = f -> f self.value

main = ->
    b = Box::new 21
    b.apply identity .println!
    0
"#,
    );
    let offending_defs: Vec<&str> = llvm
        .lines()
        .filter(|line| line.starts_with("define ") && line.contains("@stdlib__String_none_new"))
        .collect();
    assert!(
        offending_defs.is_empty(),
        "app IR unexpectedly defines stdlib object-backed symbols: {:?}",
        offending_defs
    );
}

#[test]
fn test_array_print() {
    let output = compile_and_run(
        r#"
main = ->
    // Test 1: Simple integer array
    arr = [1, 2, 3, 4, 5]
    arr.println!

    // Test 2: Array with different values
    numbers = [10, 20, 30]
    numbers.println!

    // Test 3: Empty array
    empty: [I64; 0] = []
    empty.println!

    // Test 4: Single element
    single = [42]
    single.println!

    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "[1, 2, 3, 4, 5]");
    assert_eq!(lines[1], "[10, 20, 30]");
    assert_eq!(lines[2], "[]");
    assert_eq!(lines[3], "[42]");
}

#[test]
fn test_int_to_string() {
    let output = compile_and_run(
        r#"
main = ->
    s = int_to_string 42
    s.println!
    num_str = int_to_string 100
    result = (String::from_str "Value: ").concat (num_str.clone!)
    result.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "42");
    assert_eq!(lines[1], "Value: 100");
}

#[test]
fn test_stdlib_conversions_return_owned_strings() {
    let (output, success) = compile_and_run_with_status(
        r#"
main = ->
    i = int_to_string 12345
    i.println!
    i.len!.println!

    neg = int_to_string (-7)
    neg.println!
    neg.len!.println!

    f = float_to_string 3.5
    f.println!
    f.len! > 0 .println!

    string_to_int "42" .println!
    string_to_float "2.5" .println!
    0
"#,
    );
    assert!(success, "program failed with stdout: {output:?}");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "12345");
    assert_eq!(lines[1], "5");
    assert_eq!(lines[2], "-7");
    assert_eq!(lines[3], "2");
    assert_eq!(lines[4], "3.5");
    assert_eq!(lines[5], "true");
    assert_eq!(lines[6], "42");
    assert_eq!(lines[7], "2.5");
}

#[test]
fn test_stdlib_string_helpers_return_owned_values() {
    let (output, success) = compile_and_run_with_status(
        r#"
main = ->
    joined: String = string_concat "Rock", "Lang"
    joined.println!
    joined.len!.println!

    bytes = [82 as U8, 111 as U8, 99 as U8, 107 as U8]
    slice_in = &bytes
    sub: Vec U8 = byte_substr slice_in, 1, 2
    sub.len!.println!
    slice = sub.as_slice!
    slice[0] as I64 .println!
    slice[1] as I64 .println!
    0
"#,
    );
    assert!(success, "program failed with stdout: {output:?}");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["RockLang", "8", "2", "111", "99"]);
}

#[test]
fn test_stdlib_string_add_operators() {
    let src = r#"
main = ->
    suffix = String::from_str "!"
    prefix = "Hello, " + "Rock"
    a = prefix + " language"
    b = "Greeting: " + a
    c = b + suffix
    c.println!
    0
"#;

    let output = compile_and_run(src);
    assert_eq!(output, "Greeting: Hello, Rock language!\n");
}

#[test]
fn test_stdlib_eq_default_not_equal() {
    let src = r#"
main = ->
    1 != 2 .println!
    1 != 1 .println!
    "abc" != "abd" .println!
    "abc" != "abc" .println!
    0
"#;

    let output = compile_and_run(src);
    assert_eq!(output, "true\nfalse\ntrue\nfalse\n");
}

#[test]
fn test_stdlib_repeated_primitive_show_println_drops_temporaries() {
    let (output, success) = compile_and_run_with_status(
        r#"
main = ->
    i = 0
    while i < 5
        (100 + i).show!.println!
        (i as F64).show!.println!
        true.show!.println!
        'x'.show!.println!
        i = i + 1
    0
"#,
    );
    assert!(success, "program failed with stdout: {output:?}");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines.len(), 20);
    assert_eq!(lines[0], "100");
    assert_eq!(lines[1], "0");
    assert_eq!(lines[2], "true");
    assert_eq!(lines[3], "x");
    assert_eq!(lines[16], "104");
    assert_eq!(lines[18], "true");
    assert_eq!(lines[19], "x");
}

#[test]
fn test_struct_method_with_struct_param() {
    let output = compile_and_run(
        r#"
struct Vec2
    < x: I64
    < y: I64

impl Vec2
    @add = other ->
        Vec2
            x: @x + other.x
            y: @y + other.y
    @dot = other -> @x * other.x + @y * other.y
    @magnitude_sq = -> @x * @x + @y * @y

main = ->
    v1 = Vec2
        x: 3
        y: 4
    v2 = Vec2
        x: 1
        y: 2
    v2_for_dot = Vec2
        x: 1
        y: 2
    v3 = v1.add v2
    v3.x.println!
    v3.y.println!
    v1.dot v2_for_dot .println!
    v1.magnitude_sq!.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "4");
    assert_eq!(lines[1], "6");
    assert_eq!(lines[2], "11");
    assert_eq!(lines[3], "25");
}

#[test]
fn test_string_operations_advanced() {
    let output = compile_and_run(
        r#"
repeat_str = s, n ->
    result = String::from_str ""
    i = 0
    while i < n
        result = result.concat (String::from_str s)
        i = i + 1
    result

main = ->
    stars = repeat_str "*", 5
    stars.println!
    stars.len!.println!
    num_str = int_to_string 42
    msg = (String::from_str "Count: ").concat (num_str.clone!)
    msg.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "*****");
    assert_eq!(lines[1], "5");
    assert_eq!(lines[2], "Count: 42");
}

#[test]
fn test_showcase_example() {
    let output = compile_example("showcase");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines.len(), 14);
    assert_eq!(lines[0], "4"); // v3.x = 3+1
    assert_eq!(lines[1], "6"); // v3.y = 4+2
    assert_eq!(lines[2], "11"); // v1.dot(v2) = 3*1+4*2
    assert_eq!(lines[3], "25"); // magnitude_sq = 9+16
    assert_eq!(lines[4], "75"); // circle area = 5*5*3
    assert_eq!(lines[5], "16"); // square area = 4*4
    assert_eq!(lines[6], "3628800"); // factorial(10)
    assert_eq!(lines[7], "1"); // is_prime(97)
    assert_eq!(lines[8], "150"); // sum_array
    assert_eq!(lines[9], "5"); // array_len
    assert_eq!(lines[10], "Result: 11"); // string concat
    assert_eq!(lines[11], "42"); // abs(-42)
    assert_eq!(lines[12], "5"); // min(5,10)
    assert_eq!(lines[13], "10"); // max(5,10)
}

#[test]
fn test_builtin_functions() {
    let output = compile_and_run(
        r#"
main = ->
    abs (0 - 5) .println!
    abs 3 .println!
    min 10, 20 .println!
    max 10, 20 .println!
    min (0 - 5), 3 .println!
    max (0 - 5), 3 .println!
    s = float_to_string 3.14
    s.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "5");
    assert_eq!(lines[1], "3");
    assert_eq!(lines[2], "10");
    assert_eq!(lines[3], "20");
    assert_eq!(lines[4], "-5");
    assert_eq!(lines[5], "3");
    assert_eq!(lines[6], "3.14");
}

#[test]
fn test_string_builtins() {
    let output = compile_and_run(
        r#"
extern malloc: I64 -> *U8

main = ->
    s = "Hello, World!"
    string_len s .println!
    string_find s, "World" .println!
    string_find s, "xyz" .println!
    string_contains s, "Hello" .println!
    string_contains s, "xyz" .println!
    buf = malloc 8
    unsafe
        *buf = 97
        *(~PtrOffset buf, 1) = 98
        *(~PtrOffset buf, 2) = 0
        *(~PtrOffset buf, 3) = 99
        *(~PtrOffset buf, 4) = 100
        *(~PtrOffset buf, 5) = 101
        *(~PtrOffset buf, 6) = 102
        *(~PtrOffset buf, 7) = 0
    bounded = unsafe ~BorrowStr buf, 7
    string_find bounded, "def" .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "13");
    assert_eq!(lines[1], "7");
    assert_eq!(lines[2], "-1");
    assert_eq!(lines[3], "1");
    assert_eq!(lines[4], "0");
    assert_eq!(lines[5], "4");
}

#[test]
fn test_closures() {
    let output = compile_and_run(
        r#"
main = ->
    x = 10
    add_x = a -> a + x
    add_x 5 .println!
    add_x 20 .println!
    mul = a, b -> a * b
    mul 3, 7 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "15"); // 5 + 10
    assert_eq!(lines[1], "30"); // 20 + 10
    assert_eq!(lines[2], "21"); // 3 * 7
}

#[test]
fn test_nested_structs() {
    let output = compile_and_run(
        r#"
struct Point
    < x: I64
    < y: I64

struct Rect
    < origin: Point
    < width: I64
    < height: I64

impl Rect
    @area = -> @width * @height

main = ->
    r = Rect
        origin: Point
            x: 10
            y: 20
        width: 100
        height: 50
    r.area! .println!
    r.origin.x.println!
    r.origin.y.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "5000");
    assert_eq!(lines[1], "10");
    assert_eq!(lines[2], "20");
}

#[test]
fn test_method_chaining() {
    let output = compile_and_run(
        r#"
struct Counter
    < value: I64

impl Counter
    @inc = ->
        Counter
            value: @value + 1
    @dec = ->
        Counter
            value: @value - 1
    @get = -> @value

main = ->
    c = Counter
        value: 0
    c = c.inc!
    c = c.inc!
    c = c.inc!
    c = c.dec!
    c.get! .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "2");
}

#[test]
fn test_enum_unwrap() {
    let output = compile_and_run(
        r#"
enum Option
    Some I64
    None

unwrap_or = opt, default ->
    match opt
        Option::Some val => val
        Option::None => default

main = ->
    a = Option::Some 42
    b = Option::None
    unwrap_or a, 0 .println!
    unwrap_or b, 99 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "42");
    assert_eq!(lines[1], "99");
}

#[test]
fn test_string_equality() {
    let output = compile_and_run(
        r#"
extern malloc: I64 -> *U8

main = ->
    a = "hello"
    b = "hello"
    c = "world"
    if a == b
        1.println!
    else
        0.println!
    if a == c
        1.println!
    else
        0.println!
    left_buf = malloc 5
    right_buf = malloc 5
    unsafe
        *left_buf = 97
        *(~PtrOffset left_buf, 1) = 98
        *(~PtrOffset left_buf, 2) = 0
        *(~PtrOffset left_buf, 3) = 99
        *(~PtrOffset left_buf, 4) = 0
        *right_buf = 97
        *(~PtrOffset right_buf, 1) = 98
        *(~PtrOffset right_buf, 2) = 0
        *(~PtrOffset right_buf, 3) = 100
        *(~PtrOffset right_buf, 4) = 0
    left = unsafe ~BorrowStr left_buf, 4
    right = unsafe ~BorrowStr right_buf, 4
    if left == right
        1.println!
    else
        0.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1"); // "hello" == "hello"
    assert_eq!(lines[1], "0"); // "hello" != "world"
    assert_eq!(lines[2], "0"); // "ab\0c" != "ab\0d"
}

#[test]
fn test_float_arithmetic() {
    let output = compile_and_run(
        r#"
main = ->
    x = 3.14
    y = 2.0
    x + y .println!
    x * y .println!
    x - y .println!
    x / y .println!
    sqrt x .println!
    to_int x .println!
    z = to_float 42
    z.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "5.14");
    assert_eq!(lines[1], "6.28");
    assert_eq!(lines[2], "1.14");
    assert_eq!(lines[3], "1.57");
    assert!(lines[4].starts_with("1.772")); // sqrt(3.14)
    assert_eq!(lines[5], "3"); // to_int(3.14) truncates
    assert_eq!(lines[6], "42"); // to_float(42) prints as 42 with %g
}

#[test]
fn test_map_array() {
    let output = compile_and_run(
        r#"
main = ->
    arr = [1, 2, 3, 4, 5]
    i = 0
    while i < (~ArrayLen arr)
        arr[i] * 2 .println!
        i = i + 1
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines.len(), 5);
    assert_eq!(lines[0], "2");
    assert_eq!(lines[1], "4");
    assert_eq!(lines[2], "6");
    assert_eq!(lines[3], "8");
    assert_eq!(lines[4], "10");
}

#[test]
fn test_for_loop_sum() {
    let output = compile_and_run(
        r#"
main = ->
    total = 0
    for x in [10, 20, 30, 40, 50]
        total = total + x
    total.println!
    0
"#,
    );
    assert_eq!(output.trim(), "150");
}

#[test]
fn test_string_parsing() {
    let output = compile_and_run(
        r#"
main = ->
    n = string_to_int "12345"
    n.println!
    n + 1 .println!
    f = string_to_float "3.14"
    f.println!
    string_to_int "-42" .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "12345");
    assert_eq!(lines[1], "12346");
    assert_eq!(lines[2], "3.14");
    assert_eq!(lines[3], "-42");
}

#[test]
fn test_tuple_destructuring() {
    let output = compile_and_run(
        r#"
swap = a, b -> (b, a)

main = ->
    (x, y) = swap 3, 7
    x.println!
    y.println!
    (a, b, c) = (10, 20, 30)
    a.println!
    b.println!
    c.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "7");
    assert_eq!(lines[1], "3");
    assert_eq!(lines[2], "10");
    assert_eq!(lines[3], "20");
    assert_eq!(lines[4], "30");
}

#[test]
fn test_matrix_multiply() {
    let output = compile_and_run(
        r#"
struct Matrix
    < a: I64
    < b: I64
    < c: I64
    < d: I64

impl Matrix
    @det = -> @a * @d - @b * @c
    @trace = -> @a + @d
    @mul = other ->
        Matrix
            a: @a * other.a + @b * other.c
            b: @a * other.b + @b * other.d
            c: @c * other.a + @d * other.c
            d: @c * other.b + @d * other.d

main = ->
    m1 = Matrix
        a: 1
        b: 2
        c: 3
        d: 4
    m2 = Matrix
        a: 5
        b: 6
        c: 7
        d: 8
    m1.det! .println!
    m1.trace! .println!
    m3 = m1.mul m2
    m3.a.println!
    m3.b.println!
    m3.c.println!
    m3.d.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "-2"); // det(1,2;3,4) = 1*4-2*3 = -2
    assert_eq!(lines[1], "5"); // trace = 1+4
    assert_eq!(lines[2], "19"); // m3.a = 1*5+2*7
    assert_eq!(lines[3], "22"); // m3.b = 1*6+2*8
    assert_eq!(lines[4], "43"); // m3.c = 3*5+4*7
    assert_eq!(lines[5], "50"); // m3.d = 3*6+4*8
}

#[test]
fn test_bubble_sort_algorithm() {
    let output = compile_and_run(
        r#"
main = ->
    mut arr = [5, 3, 8, 1, 9, 2, 7, 4, 6, 10]
    len = ~ArrayLen arr
    i = 0
    while i < len
        j = 0
        while j < len - 1 - i
            if arr[j] > arr[j + 1]
                temp = arr[j]
                arr[j] = arr[j + 1]
                arr[j + 1] = temp
            j = j + 1
        i = i + 1
    k = 0
    while k < len
        arr[k].println!
        k = k + 1
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    for i in 0..10 {
        assert_eq!(lines[i], (i + 1).to_string());
    }
}

#[test]
fn test_fibonacci() {
    let output = compile_and_run(
        r#"
fib = n ->
    if n <= 1
        n
    else
        (fib (n - 1)) + (fib (n - 2))

main = ->
    i = 0
    while i < 10
        fib i .println!
        i = i + 1
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    let expected = ["0", "1", "1", "2", "3", "5", "8", "13", "21", "34"];
    for (i, exp) in expected.iter().enumerate() {
        assert_eq!(lines[i], *exp);
    }
}

#[test]
fn test_fizzbuzz() {
    let output = compile_and_run(
        r#"
fizzbuzz = n ->
    if n % 15 == 0
        "FizzBuzz".println!
    else if n % 3 == 0
        "Fizz".println!
    else if n % 5 == 0
        "Buzz".println!
    else
        n.println!

main = ->
    i = 1
    while i <= 15
        fizzbuzz i
        i = i + 1
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[2], "Fizz");
    assert_eq!(lines[4], "Buzz");
    assert_eq!(lines[14], "FizzBuzz");
}

#[test]
fn test_string_escape_sequences() {
    let output = compile_and_run(
        r#"
extern malloc: I64 -> *U8

main = ->
    "Hello\tWorld".println!
    "Line1\nLine2".println!
    "Quote: \"hi\"".println!
    "Backslash: \\".println!
    buf = malloc 6
    unsafe
        *buf = 97
        *(~PtrOffset buf, 1) = 98
        *(~PtrOffset buf, 2) = 0
        *(~PtrOffset buf, 3) = 99
        *(~PtrOffset buf, 4) = 100
        *(~PtrOffset buf, 5) = 0
    bounded = unsafe ~BorrowStr buf, 5
    bounded.println!
    0
"#,
    );
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines[0], "Hello\tWorld");
    assert_eq!(lines[1], "Line1");
    assert_eq!(lines[2], "Line2");
    assert_eq!(lines[3], "Quote: \"hi\"");
    assert_eq!(lines[4], "Backslash: \\");
    assert_eq!(lines[5].as_bytes(), b"ab\0cd");
}

#[test]
fn test_enum_match_strings() {
    let output = compile_and_run(
        r#"
enum Color
    Red
    Green
    Blue

color_name = c ->
    match c
        Color::Red => "red"
        Color::Green => "green"
        Color::Blue => "blue"

main = ->
    color_name (Color::Red) .println!
    color_name (Color::Green) .println!
    color_name (Color::Blue) .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "red");
    assert_eq!(lines[1], "green");
    assert_eq!(lines[2], "blue");
}

#[test]
fn test_sieve_of_eratosthenes() {
    let output = compile_and_run(
        r#"
main = ->
    limit = 30
    mut sieve = Vec::new!
    i = 0
    while i < limit
        sieve.push 1
        i = i + 1
    sieve.set 0, 0
    sieve.set 1, 0
    i = 2
    while i * i < limit
        is_prime = match (sieve.get i)
            Option::Some val => *val
            Option::None => 0
        if is_prime == 1
            j = i * i
            while j < limit
                sieve.set j, 0
                j = j + i
        i = i + 1
    count = 0
    i = 2
    while i < limit
        is_prime = match (sieve.get i)
            Option::Some val => *val
            Option::None => 0
        if is_prime == 1
            count = count + 1
        i = i + 1
    count.println!
    0
"#,
    );
    assert_eq!(output.trim(), "10"); // primes below 30: 2,3,5,7,11,13,17,19,23,29
}

#[test]
fn test_negative_numbers() {
    let output = compile_and_run(
        r#"
main = ->
    x = -42
    x.println!
    y = 10
    z = -y
    z.println!
    abs (0 - 7) .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "-42");
    assert_eq!(lines[1], "-10");
    assert_eq!(lines[2], "7");
}

#[test]
fn test_gcd_lcm() {
    let output = compile_and_run(
        r#"
gcd = a, b ->
    if b == 0
        a
    else
        gcd b, (a % b)

lcm = a, b -> a / (gcd a, b) * b

main = ->
    gcd 48, 18 .println!
    lcm 12, 18 .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "6");
    assert_eq!(lines[1], "36");
}

#[test]
fn test_break_continue() {
    let output = compile_and_run(
        r#"
main = ->
    i = 0
    while i < 100
        if i == 42
            break
        i = i + 1
    i.println!

    count = 0
    j = 0
    while j < 20
        j = j + 1
        if j % 3 == 0
            continue
        count = count + 1
    count.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "42");
    assert_eq!(lines[1], "14");
}

#[test]
fn test_string_building() {
    let output = compile_and_run(
        r#"
main = ->
    result = String::from_str ""
    i = 1
    while i <= 5
        if i > 1
            result = result.concat (String::from_str ", ")
        num_str = int_to_string i
        result = result.concat (num_str.clone!)
        i = i + 1
    result.println!
    0
"#,
    );
    assert_eq!(output.trim(), "1, 2, 3, 4, 5");
}

#[test]
fn test_trait_with_defaults() {
    let output = compile_and_run(
        r#"
struct Dog
    < name: I64

struct Cat
    < name: I64

trait Animal
    @speak = -> 0
    @legs = -> 4

impl Animal for Dog
    @speak = -> 1

impl Animal for Cat
    @speak = -> 2

main = ->
    d = Dog
        name: 1
    c = Cat
        name: 2
    d.speak!.println!
    c.speak!.println!
    d.legs!.println!
    c.legs!.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "2");
    assert_eq!(lines[2], "4");
    assert_eq!(lines[3], "4");
}

// Helper for testing that compilation fails with expected error message
fn compile_should_fail(source: &str, expected_error: &str) {
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

fn compile_should_fail_with_source_diagnostic(source: &str, expected_error: &str) {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let source_path = dir.join("test.rk");
    fs::write(&source_path, source).unwrap();
    let config = test_config(source_path.clone(), dir.clone());

    let diagnostics = rock_lib::compile(&config).expect_err("expected compilation to fail");
    for diagnostic in &diagnostics.0 {
        for forbidden in [
            "struct#",
            "enum#",
            "generic#",
            "trait#",
            "?T",
            "DefId",
            "TypeVarId",
            "GenericParamId",
            "FieldId",
            "VariantId",
            "Local(",
        ] {
            assert!(
                !diagnostic.message.contains(forbidden),
                "source diagnostic exposed compiler identity {forbidden:?}: {}",
                diagnostic.message
            );
        }
    }
    let diagnostic = diagnostics
        .0
        .iter()
        .find(|diagnostic| diagnostic.message.contains(expected_error))
        .unwrap_or_else(|| panic!("expected diagnostic containing {expected_error:?}"));
    let rock_lib::diagnostic::DiagnosticLocation::Source(span) = &diagnostic.location else {
        panic!("source diagnostic should have a source location");
    };
    assert_eq!(span.file_path, source_path);
    assert!(span.start <= span.end);
    let attached = diagnostic
        .source
        .as_ref()
        .expect("source diagnostic should carry its loaded source");
    assert_eq!(attached.text, source);
    assert_eq!(
        attached.origin,
        rock_lib::diagnostic::DiagnosticSourceOrigin::FileSystem
    );

    let _ = fs::remove_dir_all(dir);
}

fn compile_should_fail_with_exact_diagnostic(source: &str, expected_error: &str) {
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

fn compile_should_pass(source: &str) {
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

fn compile_should_pass_without_stdlib(source: &str) {
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

#[test]
fn test_integer_literal_defaults_to_i64() {
    compile_should_pass_without_stdlib(
        r#"main = ->
    value = 42
    0
"#,
    );
}

#[test]
fn test_float_literal_defaults_to_f64() {
    compile_should_pass_without_stdlib(
        r#"main = ->
    value = 4.2
    0
"#,
    );
}

#[test]
fn test_unconstrained_local_lambda_is_rejected() {
    compile_should_fail(
        r#"main = ->
    identity = value -> value
    0
"#,
        "ambiguous type",
    );
}

#[test]
fn test_generic_function_remains_polymorphic_under_strict_finalization() {
    compile_should_pass_without_stdlib(
        r#"identity = value -> value

main = ->
    integer: I64 = identity 42
    boolean: Bool = identity true
    0
"#,
    );
}

#[test]
fn test_public_struct_field_access_succeeds_outside_impl() {
    let output = compile_and_run(
        r#"
struct Point
    < x: I64
    < y: I64

main = ->
    p = Point
        x: 10
        y: 20
    p.x.println!
    p.y.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["10", "20"]);
}

#[test]
fn test_private_struct_field_access_is_rejected_outside_impl() {
    compile_should_fail(
        r#"
struct Point
    < x: I64
    y: I64

impl Point
    make = x, y ->
        Point
            x: x
            y: y

main = ->
    p = Point::make 10, 20
    p.y.println!
    0
"#,
        "Field 'y' of struct 'Point' is private",
    );
}

#[test]
fn test_private_struct_field_access_is_rejected_through_inferred_receiver() {
    compile_should_fail(
        r#"
struct Point
    < x: I64
    y: I64

impl Point
    make = x, y ->
        Point
            x: x
            y: y

get_y = p -> p.y

main = ->
    p = Point::make 10, 20
    get_y p .println!
    0
"#,
        "Field 'y' of struct 'Point' is private",
    );
}

#[test]
fn test_private_struct_field_access_succeeds_inside_own_impl() {
    let output = compile_and_run(
        r#"
struct Point
    < x: I64
    y: I64

impl Point
    make = x, y ->
        Point
            x: x
            y: y
    @sum = -> @x + @y

main = ->
    p = Point::make 10, 20
    p.sum! .println!
    0
"#,
    );

    assert_eq!(output.trim(), "30");
}

#[test]
fn test_private_struct_construction_is_rejected_outside_impl() {
    compile_should_fail(
        r#"
struct Point
    < x: I64
    y: I64

main = ->
    p = Point
        x: 10
        y: 20
    p.x.println!
    0
"#,
        "Cannot construct struct 'Point' because it has private fields",
    );
}

#[test]
fn test_private_struct_construction_succeeds_inside_own_impl() {
    let output = compile_and_run(
        r#"
struct Point
    < x: I64
    y: I64

impl Point
    make = x, y ->
        Point
            x: x
            y: y

main = ->
    p = Point::make 10, 20
    p.x.println!
    0
"#,
    );

    assert_eq!(output.trim(), "10");
}

#[test]
fn test_private_struct_pattern_is_rejected_outside_impl() {
    compile_should_fail(
        r#"
struct Point
    < x: I64
    y: I64

impl Point
    make = x, y ->
        Point
            x: x
            y: y

inspect = p ->
    match p
        Point y: _ => 0

main = ->
    p = Point::make 10, 20
    inspect p .println!
    0
"#,
        "Field 'y' of struct 'Point' is private",
    );
}

#[test]
fn test_private_struct_pattern_succeeds_inside_own_impl() {
    let output = compile_and_run(
        r#"
struct Point
    < x: I64
    y: I64

impl Point
    make = x, y ->
        Point
            x: x
            y: y
    @sum = ->
        match *self
            Point x: x, y: y => x + y

main = ->
    p = Point::make 10, 20
    p.sum! .println!
    0
"#,
    );

    assert_eq!(output.trim(), "30");
}

#[test]
fn test_struct_pattern_rejects_refutable_field_subpatterns() {
    compile_should_fail(
        r#"
struct Point
    < x: I64
    < y: I64

main = ->
    p = Point
        x: 10
        y: 20
    match p
        Point x: 10, y: y => y
"#,
        "Struct field patterns currently only support bindings and wildcards",
    );
}

#[test]
fn test_struct_pattern_rejects_mismatched_struct_type() {
    compile_should_fail(
        r#"
struct Point
    < x: I64

struct Other
    < x: I64

main = ->
    p = Point
        x: 10
    match p
        Other x: x => x
        Point x: x => x + 1
"#,
        "pattern type mismatch",
    );
}

#[test]
fn test_generic_struct_pattern_infers_scrutinee_type() {
    let output = compile_and_run(
        r#"
struct Box T
    < value: T

unwrap = box ->
    match box
        Box value: value => value + 1

main = ->
    unwrap (Box
        value: 10) .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "11");
}

#[test]
fn test_generic_struct_pattern_binds_non_i64_payload() {
    let output = compile_and_run(
        r#"
struct Box T
    < value: T

unwrap = box ->
    match box
        Box value: value => value

main = ->
    unwrap (Box
        value: "hello") .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "hello");
}

#[test]
fn test_trait_default_method_substitutes_trait_generic_struct_pattern_types() {
    let output = compile_and_run(
        r#"
struct Box T
    < value: T

struct Counter
    < seed: I64

trait UnwrapBox T
    @unwrap: Box T -> T
    @unwrap = boxed ->
        match boxed
            Box value: value => value

impl UnwrapBox I64 for Counter

main = ->
    receiver = Counter
        seed: 0
    boxed = Box
        value: 7
    unwrapped = receiver.unwrap boxed
    unwrapped.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "7");
}

#[test]
fn test_all_public_struct_construction_succeeds_outside_impl() {
    compile_should_pass(
        r#"
struct Point
    < x: I64
    < y: I64

main = ->
    p = Point
        x: 10
        y: 20
    p.x.println!
    0
"#,
    );
}

#[test]
fn test_type_error_int_plus_string() {
    compile_should_fail(
        r#"
main = ->
    x = 42 + "hello"
    0
"#,
        "No implementation found for operator '+' on type I64",
    );
}

#[test]
fn test_type_error_float_plus_string() {
    compile_should_fail(
        r#"
main = ->
    x = 3.14 + "world"
    0
"#,
        "No implementation found for operator '+' on type F64",
    );
}

#[test]
fn test_type_error_annotation_mismatch() {
    compile_should_fail(
        r#"
main = ->
    x: I64 = "hello"
    0
"#,
        "Type annotation mismatch",
    );
}

#[test]
fn test_type_error_bool_plus_int() {
    compile_should_fail(
        r#"
main = ->
    x = true + 42
    0
"#,
        "No implementation found for operator '+' on type Bool",
    );
}

#[test]
fn test_explicit_fixed_array_type_syntax_lowers_successfully() {
    let output = compile_and_run(
        r#"
get_first : [I64; 4] -> I64
get_first = arr ->
    arr[0]

main = ->
    arr: [I64; 4] = [10, 20, 30, 40]
    get_first arr .println!
    0
"#,
    );

    assert_eq!(output.trim(), "10");
}

#[test]
fn test_mut_fixed_array_borrow_coerces_to_mut_slice_parameter() {
    let output = compile_and_run(
        r#"
write_first: &mut [I64] -> I64
write_first = s ->
    ptr = (~ArrPtr (*s)) as *I64
    unsafe *ptr = 9
    unsafe *ptr

main = ->
    mut arr = [1, 2, 3]
    r = &mut arr
    write_first r .println!
    arr[0].println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "9");
    assert_eq!(lines[1], "9");
}

#[test]
fn test_mut_fixed_array_borrow_coerces_to_shared_slice_parameter() {
    let output = compile_and_run(
        r#"
sum: &[I64] -> I64
sum = s ->
    (s[0] + s[1]) + s[2]

main = ->
    mut arr = [1, 2, 3]
    r = &mut arr
    sum r .println!
    0
"#,
    );

    assert_eq!(output.trim(), "6");
}

#[test]
fn test_mut_fixed_array_borrow_dispatches_shared_slice_method() {
    let output = compile_and_run(
        r#"
show_mut: &mut [I64] -> String
show_mut = s -> s.show!

main = ->
    mut arr = [1, 2, 3]
    r = &mut arr
    show_mut r .println!
    0
"#,
    );

    assert_eq!(output.trim(), "[1, 2, 3]");
}

#[test]
fn test_mut_array_borrow_directly_dispatches_shared_slice_method() {
    let output = compile_and_run(
        r#"
main = ->
    mut arr = [1, 2, 3]
    r = &mut arr
    r.show!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "[1, 2, 3]");
}

#[test]
fn test_mut_array_borrow_dispatches_custom_shared_slice_method() {
    let output = compile_and_run(
        r#"
trait SliceLen
    @slice_len = -> 0

impl SliceLen for &[T]
    @slice_len = ->
        ~ArrayLen (*self)

main = ->
    mut arr = [1, 2, 3]
    r = &mut arr
    r.slice_len!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "3");
}

#[test]
fn test_string_concat_builtin() {
    let output = compile_and_run(
        r#"
main = ->
    a = "Hello, "
    b = "World!"
    c = string_concat a, b
    c.println!
    d = (string_concat "foo", "bar").concat (String::from_str "baz")
    d.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "Hello, World!");
    assert_eq!(lines[1], "foobarbaz");
}

#[test]
fn test_vec_swap_remove_moves_owned_value() {
    let output = compile_and_run(
        r#"
main = ->
    mut values: Vec String = Vec::new!
    values.push (String::from_str "first")
    values.push (String::from_str "second")
    values.push (String::from_str "third")
    match values.swap_remove 1
        Option::Some value => value.println!
        Option::None => "missing".println!
    values.len!.println!
    values[1].println!
    0
"#,
    );

    assert_eq!(
        output.trim().lines().collect::<Vec<_>>(),
        ["second", "2", "third"]
    );
}

#[test]
fn test_vec_set_get() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 10
    v.push 20
    v.push 30
    v.set 1, 99
    match (v.get 0)
        Option::Some val => *val .println!
        Option::None => 0.println!
    match (v.get 1)
        Option::Some val => *val .println!
        Option::None => 0.println!
    match (v.get 2)
        Option::Some val => *val .println!
        Option::None => 0.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "99");
    assert_eq!(lines[2], "30");
}

#[test]
fn test_stdlib_vec_drops_initialized_elements() {
    let output = compile_and_run(
        r#"
struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    mut v = Vec::new!
    v.push (Tracked
        value: 1)
    v.push (Tracked
        value: 2)
    v.push (Tracked
        value: 3)
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["3", "2", "1"]);
}

#[test]
fn test_stdlib_vec_set_drops_replaced_element_once() {
    let output = compile_and_run(
        r#"
struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    mut v = Vec::new!
    v.push (Tracked
        value: 1)
    v.push (Tracked
        value: 2)
    v.set 0, (Tracked
        value: 9)
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["1", "2", "9"]);
}

#[test]
fn test_stdlib_vec_set_out_of_bounds_drops_ignored_value_once() {
    let output = compile_and_run(
        r#"
struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    mut v = Vec::new!
    v.push (Tracked
        value: 1)
    v.set (-1), (Tracked
        value: 8)
    v.set 3, (Tracked
        value: 9)
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["8", "9", "1"]);
}

#[test]
fn test_stdlib_vec_growth_drops_initialized_elements_once() {
    let output = compile_and_run(
        r#"
struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    mut v = Vec::new!
    v.push (Tracked
        value: 1)
    v.push (Tracked
        value: 2)
    v.push (Tracked
        value: 3)
    v.push (Tracked
        value: 4)
    v.push (Tracked
        value: 5)
    v.push (Tracked
        value: 6)
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["6", "5", "4", "3", "2", "1"]);
}

#[test]
fn test_stdlib_option_methods() {
    let output = compile_and_run(
        r#"
inc = x -> x + 1

to_even = x ->
    if x % 2 == 0
        Option::Some x
    else
        Option::None

main = ->
    some_map = Option::Some 41
    none_map = Option::None
    some_and_then = Option::Some 41
    none_fold = Option::None
    nested = Option::Some (Option::Some 9)
    some_fold = Option::Some 41
    some_ok_or: Result I64, I64 = Option::Some 12 .ok_or 5
    none_ok_or: Result I64, I64 = Option::None .ok_or 7
    some_folded: I64 = some_fold.fold
        0
        x -> x + 1
    none_folded: I64 = none_fold.fold
        7
        x -> x + 1

    (some_map.map inc).unwrap_or 0 .println!
    (none_map.map inc).unwrap_or 0 .println!
    (some_and_then.and_then to_even).unwrap_or 0 .println!
    ((Option::Some 8).and_then to_even).unwrap_or 0 .println!
    (nested.flatten!).unwrap_or 0 .println!
    some_folded.println!
    none_folded.println!
    some_ok_or.fold (x -> x), (x -> x) .println!
    none_ok_or.fold (x -> x), (x -> x) .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["42", "0", "0", "8", "9", "42", "7", "12", "7"]);
}

#[test]
fn test_stdlib_option_inspect_borrows_payload_and_returns_option() {
    let output = compile_and_run(
        r#"
print_ref: &I64 -> I32
print_ref = x -> *x .println!

main = ->
    value = (Option::Some 4).inspect print_ref
    value.unwrap_or 0 .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["4", "4"]);
}

#[test]
fn test_stdlib_option_inspect_owned_string_borrows_and_returns_owned_option() {
    let output = compile_and_run(
        r#"
print_len: &String -> I32
print_len = value -> value.len! .println!

main = ->
    value = Option::Some (String::from_str "hello")
    returned = value.inspect print_len
    returned.show!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "5\nSome(hello)");
}

#[test]
fn test_stdlib_option_inspect_owned_string_consumes_receiver() {
    compile_should_fail(
        r#"
print_len: &String -> I32
print_len = value -> value.len! .println!

main = ->
    value = Option::Some (String::from_str "hello")
    returned = value.inspect print_len
    returned.show!.println!
    value.show!.println!
    0
"#,
        "borrow of moved value",
    );
}

#[test]
fn test_stdlib_result_methods() {
    let output = compile_and_run(
        r#"
inc = x -> x + 1
tag = err -> err + 5
id_i64: I64 -> I64
id_i64 = x -> x
inc1 = x -> x + 1

parse = x ->
    if x > 0
        Result::Ok x
    else
        Result::Err 3

main = ->
    ok_map: Result I64, I64 = Result::Ok 41
    err_map: Result I64, I64 = Result::Err 4
    err_map_err: Result I64, I64 = Result::Err 4
    ok_and_then: Result I64, I64 = Result::Ok 41
    err_or: Result I64, I64 = Result::Err 4
    nested_ok: Result (Result I64, I64), I64 = Result::Ok (Result::Ok 9)
    nested_err: Result (Result I64, I64), I64 = Result::Ok (Result::Err 6)
    ok_fold: Result I64, I64 = Result::Ok 41
    err_fold: Result I64, I64 = Result::Err 4
    (ok_map.map inc).unwrap_or 0 .println!
    (err_map.map inc).unwrap_or 0 .println!
    (err_map_err.map_err tag).fold id_i64, id_i64 .println!
    (ok_and_then.and_then parse).unwrap_or 0 .println!
    ((Result::Ok (-1)).and_then parse).unwrap_or 0 .println!
    (err_or.or (Result::Ok 9)).unwrap_or 0 .println!
    (nested_ok.flatten!).unwrap_or 0 .println!
    (nested_err.flatten!).fold id_i64, id_i64 .println!
    (ok_fold.fold id_i64, inc1) .println!
    (err_fold.fold id_i64, id_i64) .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(
        lines,
        vec!["42", "0", "9", "41", "0", "9", "9", "6", "42", "4"]
    );
}

#[test]
fn test_hkt_functor_option_result_vec() {
    let products =
        rock_lib::products::CompilerProducts::read_artifact_from_path(&stdlib_artifact_path())
            .expect("read stdlib artifact");
    let functional_trait_ids = products
        .interface
        .traits
        .iter()
        .filter_map(|(id, trait_interface)| {
            [
                "Bifunctor",
                "Functor",
                "Applicative",
                "Monad",
                "Foldable",
                "Traversable",
            ]
            .iter()
            .any(|name| trait_interface.name.ends_with(name))
            .then_some(*id)
        })
        .collect::<Vec<_>>();
    assert_eq!(functional_trait_ids.len(), 6);
    let language_items = &products.interface.language_items;
    let language_trait_ids = [
        language_items.sized.as_ref().map(|items| items.trait_id),
        language_items.drop.as_ref().map(|items| items.trait_id),
        language_items.index.as_ref().map(|items| items.trait_id),
        language_items
            .index_mut
            .as_ref()
            .map(|items| items.trait_id),
        language_items.fn_once.as_ref().map(|items| items.trait_id),
        language_items.fn_mut.as_ref().map(|items| items.trait_id),
        language_items.fn_trait.as_ref().map(|items| items.trait_id),
        language_items.send.as_ref().map(|items| items.trait_id),
        language_items.sync.as_ref().map(|items| items.trait_id),
        language_items
            .try_protocol
            .as_ref()
            .map(|items| items.try_trait_id),
        language_items
            .try_protocol
            .as_ref()
            .map(|items| items.from_residual_trait_id),
    ];
    assert!(functional_trait_ids
        .iter()
        .all(|id| !language_trait_ids.contains(&Some(*id))));

    let output = compile_and_run(
        r#"
> stdlib::io::IoError

type IoResult = Result _, IoError

map_any: M -> F A -> F B where F _: Functor, M: FnMut A, B
map_any = mapper, value -> F::Functor::fmap mapper, value

map_result: M -> Result A, E -> Result B, E where M: FnMut A, B
map_result = mapper, value -> (Result _, E)::Functor::fmap mapper, value

inc: I64 -> I64
inc = value -> value + 1

double: I64 -> I64
double = value -> value * 2

make_values: () -> Vec I64
make_values = ->
    mut values = Vec::new!
    values.push 1
    values.push 2
    values.push 3
    values

map_values: Vec I64 -> Vec I64
map_values = values -> map_any double, values

main = ->
    mapped_option: Option I64 = map_any inc, (Option::Some 4)
    mapped_result: Result I64, I64 = map_result inc, (Result::Ok 5)
    mapped_vec: Vec I64 = (make_values!) <&> double
    qualified: Option I64 = Option::Functor::fmap inc, (Option::Some 6)
    fixed_error: Result I64, IoError = (Result _, IoError)::Applicative::pure 8
    alias_error: Result I64, IoError = IoResult::Applicative::pure 9
    nested: Option (Vec I64) = Option::Some (make_values!)
    mapped_nested: Option (Vec I64) = map_any map_values, nested

    mapped_option.show!.println!
    mapped_result.show!.println!
    mapped_vec.show!.println!
    qualified.show!.println!
    fixed_error.show!.println!
    alias_error.show!.println!
    mapped_nested.show!.println!
    0
"#,
    );

    assert_eq!(
        output,
        "Some(5)\nOk(6)\n[2, 4, 6]\nSome(7)\nOk(8)\nOk(9)\nSome([2, 4, 6])\n"
    );
}

#[test]
fn test_hkt_applicative_and_monad_laws() {
    let output = compile_and_run(
        r#"
repure_any: F I64 -> F I64 where F _: Applicative
repure_any = ignored -> F::Applicative::pure 2

ap_any: F (A -> B) -> F A -> F B where F _: Applicative
ap_any = wrapped_function, wrapped_value ->
    F::Applicative::ap wrapped_function, wrapped_value

bind_any: F A -> M -> F B where F _: Monad, M: FnMut A, (F B)
bind_any = value, callback -> F::Monad::bind value, callback

id_i64: I64 -> I64
id_i64 = value -> value

inc: I64 -> I64
inc = value -> value + 1

double: I64 -> I64
double = value -> value * 2

inc_after_double: I64 -> I64
inc_after_double = value -> inc (double value)

apply_five: (I64 -> I64) -> I64
apply_five = function -> function 5

option_add_two: I64 -> Option I64
option_add_two = value -> Option::Some (value + 2)

option_double: I64 -> Option I64
option_double = value -> Option::Some (value * 2)

option_pure: I64 -> Option I64
option_pure = value -> Option::Applicative::pure value

result_add_two: I64 -> Result I64, I64
result_add_two = value -> Result::Ok (value + 2)

result_double: I64 -> Result I64, I64
result_double = value -> Result::Ok (value * 2)

result_pure: I64 -> Result I64, I64
result_pure = value -> (Result _, I64)::Applicative::pure value

main = ->
    generic_pure: Option I64 = repure_any (Option::Some 0)
    generic_function: Option (I64 -> I64) = Option::Some inc
    generic_ap: Option I64 = ap_any generic_function, generic_pure
    generic_bind: Option I64 = bind_any generic_ap, option_add_two

    option_value: Option I64 = Option::Some 3
    option_identity_function: Option (I64 -> I64) = Option::Applicative::pure id_i64
    option_identity_l: Option I64 = Option::Applicative::ap option_identity_function, option_value
    option_identity_r: Option I64 = Option::Some 3

    option_composition_l: Option I64 = Option::Applicative::ap (Option::Some inc_after_double), (Option::Some 3)
    option_composition_inner: Option I64 = Option::Applicative::ap (Option::Some double), (Option::Some 3)
    option_composition_r: Option I64 = Option::Applicative::ap (Option::Some inc), option_composition_inner

    option_inc: Option (I64 -> I64) = Option::Applicative::pure inc
    option_four: Option I64 = Option::Applicative::pure 4
    option_homomorphism_l: Option I64 = Option::Applicative::ap option_inc, option_four
    option_homomorphism_r: Option I64 = Option::Applicative::pure (inc 4)
    option_five: Option I64 = Option::Applicative::pure 5
    option_interchange_l: Option I64 = Option::Applicative::ap (Option::Some inc), option_five
    option_apply_five: Option ((I64 -> I64) -> I64) = Option::Applicative::pure apply_five
    option_interchange_r: Option I64 = Option::Applicative::ap option_apply_five, (Option::Some inc)

    option_six: Option I64 = Option::Applicative::pure 6
    option_left_identity_l: Option I64 = Option::Monad::bind option_six, option_add_two
    option_left_identity_r: Option I64 = option_add_two 6
    option_right_identity_l: Option I64 = Option::Monad::bind (Option::Some 7), option_pure
    option_right_identity_r: Option I64 = Option::Some 7
    option_associativity_inner: Option I64 = Option::Monad::bind (Option::Some 8), option_add_two
    option_associativity_l: Option I64 = Option::Monad::bind option_associativity_inner, option_double
    option_associativity_r: Option I64 = Option::Monad::bind (Option::Some 8), (value -> Option::Monad::bind (option_add_two value), option_double)

    result_value: Result I64, I64 = Result::Ok 3
    result_identity_function: Result (I64 -> I64), I64 = (Result _, I64)::Applicative::pure id_i64
    result_identity_l: Result I64, I64 = (Result _, I64)::Applicative::ap result_identity_function, result_value
    result_identity_r: Result I64, I64 = Result::Ok 3

    result_composition_l: Result I64, I64 = (Result _, I64)::Applicative::ap (Result::Ok inc_after_double), (Result::Ok 3)
    result_composition_inner: Result I64, I64 = (Result _, I64)::Applicative::ap (Result::Ok double), (Result::Ok 3)
    result_composition_r: Result I64, I64 = (Result _, I64)::Applicative::ap (Result::Ok inc), result_composition_inner

    result_homomorphism_l: Result I64, I64 = (Result _, I64)::Applicative::ap (Result::Ok inc), (Result::Ok 4)
    result_homomorphism_r: Result I64, I64 = Result::Ok (inc 4)
    result_interchange_l: Result I64, I64 = (Result _, I64)::Applicative::ap (Result::Ok inc), (Result::Ok 5)
    result_interchange_r: Result I64, I64 = (Result _, I64)::Applicative::ap (Result::Ok apply_five), (Result::Ok inc)

    result_left_identity_l: Result I64, I64 = (Result _, I64)::Monad::bind (Result::Ok 6), result_add_two
    result_left_identity_r: Result I64, I64 = result_add_two 6
    result_right_identity_l: Result I64, I64 = (Result _, I64)::Monad::bind (Result::Ok 7), result_pure
    result_right_identity_r: Result I64, I64 = Result::Ok 7
    result_associativity_l: Result I64, I64 = (Result _, I64)::Monad::bind ((Result _, I64)::Monad::bind (Result::Ok 8), result_add_two), result_double
    result_associativity_r: Result I64, I64 = (Result _, I64)::Monad::bind (Result::Ok 8), (value -> (Result _, I64)::Monad::bind (result_add_two value), result_double)

    first_error: Result (I64 -> I64), I64 = Result::Err 1
    second_error: Result I64, I64 = Result::Err 2
    result_error_precedence: Result I64, I64 = (Result _, I64)::Applicative::ap first_error, second_error

    generic_bind.show!.println!
    option_identity_l.show!.println!
    option_identity_r.show!.println!
    option_composition_l.show!.println!
    option_composition_r.show!.println!
    option_homomorphism_l.show!.println!
    option_homomorphism_r.show!.println!
    option_interchange_l.show!.println!
    option_interchange_r.show!.println!
    option_left_identity_l.show!.println!
    option_left_identity_r.show!.println!
    option_right_identity_l.show!.println!
    option_right_identity_r.show!.println!
    option_associativity_l.show!.println!
    option_associativity_r.show!.println!
    result_identity_l.show!.println!
    result_identity_r.show!.println!
    result_composition_l.show!.println!
    result_composition_r.show!.println!
    result_homomorphism_l.show!.println!
    result_homomorphism_r.show!.println!
    result_interchange_l.show!.println!
    result_interchange_r.show!.println!
    result_left_identity_l.show!.println!
    result_left_identity_r.show!.println!
    result_right_identity_l.show!.println!
    result_right_identity_r.show!.println!
    result_associativity_l.show!.println!
    result_associativity_r.show!.println!
    result_error_precedence.show!.println!
    0
"#,
    );

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec![
            "Some(5)", "Some(3)", "Some(3)", "Some(7)", "Some(7)", "Some(5)", "Some(5)", "Some(6)",
            "Some(6)", "Some(8)", "Some(8)", "Some(7)", "Some(7)", "Some(20)", "Some(20)", "Ok(3)",
            "Ok(3)", "Ok(7)", "Ok(7)", "Ok(5)", "Ok(5)", "Ok(6)", "Ok(6)", "Ok(8)", "Ok(8)",
            "Ok(7)", "Ok(7)", "Ok(20)", "Ok(20)", "Err(1)",
        ]
    );
}

#[test]
fn test_hkt_foldable_and_traversable() {
    let output = compile_and_run(
        r#"
fold_digits: (I64, I64) -> I64
fold_digits = pair -> pair.0 * 10 + pair.1

option_pure_inc: I64 -> Option I64
option_pure_inc = value -> Option::Some (value + 1)

option_pure_double: I64 -> Option I64
option_pure_double = value -> Option::Some (value * 2)

option_pure_double_then_inc: I64 -> Option I64
option_pure_double_then_inc = value -> Option::Some (value * 2 + 1)

result_pure_inc: I64 -> Result I64, I64
result_pure_inc = value -> Result::Ok (value + 1)

option_to_result: Option A -> Result A, I64
option_to_result = value ->
    match value
        Option::Some item => Result::Ok item
        Option::None => Result::Err 99

make_values: () -> Vec I64
make_values = ->
    mut values = Vec::new!
    values.push 1
    values.push 2
    values.push 3
    values

traverse_option_inc: Option I64 -> Option (Option I64)
traverse_option_inc = value -> Option::Traversable::traverse option_pure_inc, value

traverse_result_inc: Result I64, I64 -> Option (Result I64, I64)
traverse_result_inc = value -> (Result _, I64)::Traversable::traverse option_pure_inc, value

traverse_vec_inc: Vec I64 -> Option (Vec I64)
traverse_vec_inc = value -> Vec::Traversable::traverse option_pure_inc, value

option_stop: I64 -> Option I64
option_stop = value ->
    value.println!
    if value == 2
        Option::None
    else
        Option::Some (value + 1)

result_stop: I64 -> Result I64, I64
result_stop = value ->
    value.println!
    if value == 2
        Result::Err 9
    else
        Result::Ok (value + 1)

encode_option_option: Option (Option I64) -> I64
encode_option_option = outer ->
    match outer
        Option::Some inner =>
            match inner
                Option::Some value => value
                Option::None => 0 - 2
        Option::None => 0 - 1

encode_option_result: Option (Result I64, I64) -> I64
encode_option_result = outer ->
    match outer
        Option::Some inner =>
            match inner
                Result::Ok value => value
                Result::Err error => 0 - 100 - error
        Option::None => 0 - 1

encode_option_option_option: Option (Option (Option I64)) -> I64
encode_option_option_option = outer ->
    match outer
        Option::Some middle => encode_option_option middle
        Option::None => 0 - 1

encode_option_option_result: Option (Option (Result I64, I64)) -> I64
encode_option_option_result = outer ->
    match outer
        Option::Some middle => encode_option_result middle
        Option::None => 0 - 1

encode_vec: Vec I64 -> I64
encode_vec = values -> Vec::Foldable::foldl fold_digits, 0, values

encode_option_vec: Option (Vec I64) -> I64
encode_option_vec = outer ->
    match outer
        Option::Some values => encode_vec values
        Option::None => 0 - 1

encode_option_option_vec: Option (Option (Vec I64)) -> I64
encode_option_option_vec = outer ->
    match outer
        Option::Some middle => encode_option_vec middle
        Option::None => 0 - 1

encode_result_option: Result (Option I64), I64 -> I64
encode_result_option = outer ->
    match outer
        Result::Ok inner =>
            match inner
                Option::Some value => value
                Option::None => 0 - 2
        Result::Err error => 0 - 100 - error

encode_result_result: Result (Result I64, I64), I64 -> I64
encode_result_result = outer ->
    match outer
        Result::Ok inner =>
            match inner
                Result::Ok value => value
                Result::Err error => 0 - 100 - error
        Result::Err error => 0 - 200 - error

encode_result_vec: Result (Vec I64), I64 -> I64
encode_result_vec = outer ->
    match outer
        Result::Ok values => encode_vec values
        Result::Err error => 0 - error

main = ->
    fold_option: I64 = Option::Foldable::foldl fold_digits, 0, (Option::Some 4)
    fold_ok_input: Result I64, I64 = Result::Ok 5
    fold_ok: I64 = (Result _, I64)::Foldable::foldl fold_digits, 0, fold_ok_input
    fold_error_input: Result I64, I64 = Result::Err 7
    fold_error: I64 = (Result _, I64)::Foldable::foldl fold_digits, 0, fold_error_input
    fold_vec: I64 = Vec::Foldable::foldl fold_digits, 0, (make_values!)
    mut calls = 0
    captured_step = pair ->
        calls = calls + 1
        pair.0 + pair.1
    captured_fold: I64 = Vec::Foldable::foldl captured_step, 0, (make_values!)
    fold_option.println!
    fold_ok.println!
    fold_error.println!
    fold_vec.println!
    captured_fold.println!
    calls.println!
    option_identity_l: Option (Option I64) = Option::Traversable::traverse option_pure_inc, (Option::Some 1)
    option_identity_r: Option (Option I64) = Option::Applicative::pure (Option::Some 2)
    encode_option_option option_identity_l .println!
    encode_option_option option_identity_r .println!
    result_identity_input: Result I64, I64 = Result::Ok 1
    result_identity_l: Option (Result I64, I64) = (Result _, I64)::Traversable::traverse option_pure_inc, result_identity_input
    result_identity_r: Option (Result I64, I64) = Option::Applicative::pure (Result::Ok 2)
    encode_option_result result_identity_l .println!
    encode_option_result result_identity_r .println!
    vec_identity_l: Option (Vec I64) = Vec::Traversable::traverse (value -> Option::Some value), (make_values!)
    vec_identity_r: Option (Vec I64) = Option::Applicative::pure (make_values!)
    encode_option_vec vec_identity_l .println!
    encode_option_vec vec_identity_r .println!

    option_composition_inner: Option (Option I64) = Option::Traversable::traverse option_pure_double_then_inc, (Option::Some 1)
    option_composition_l: Option (Option (Option I64)) = Option::Some option_composition_inner
    option_first: Option (Option I64) = Option::Traversable::traverse option_pure_double, (Option::Some 1)
    option_composition_r: Option (Option (Option I64)) = Option::Functor::fmap traverse_option_inc, option_first

    result_composition_input_l: Result I64, I64 = Result::Ok 1
    result_composition_inner: Option (Result I64, I64) = (Result _, I64)::Traversable::traverse option_pure_double_then_inc, result_composition_input_l
    result_composition_l: Option (Option (Result I64, I64)) = Option::Some result_composition_inner
    result_composition_input_r: Result I64, I64 = Result::Ok 1
    result_first: Option (Result I64, I64) = (Result _, I64)::Traversable::traverse option_pure_double, result_composition_input_r
    result_composition_r: Option (Option (Result I64, I64)) = Option::Functor::fmap traverse_result_inc, result_first

    vec_composition_inner: Option (Vec I64) = Vec::Traversable::traverse option_pure_double_then_inc, (make_values!)
    vec_composition_l: Option (Option (Vec I64)) = Option::Some vec_composition_inner
    vec_first: Option (Vec I64) = Vec::Traversable::traverse option_pure_double, (make_values!)
    vec_composition_r: Option (Option (Vec I64)) = Option::Functor::fmap traverse_vec_inc, vec_first
    encode_option_option_option option_composition_l .println!
    encode_option_option_option option_composition_r .println!
    encode_option_option_result result_composition_l .println!
    encode_option_option_result result_composition_r .println!
    encode_option_option_vec vec_composition_l .println!
    encode_option_option_vec vec_composition_r .println!

    option_natural_input_l: Option I64 = Option::Some 1
    option_natural_effect: Option (Option I64) = Option::Traversable::traverse option_pure_inc, option_natural_input_l
    option_naturality_l: Result (Option I64), I64 = option_to_result option_natural_effect
    option_natural_input_r: Option I64 = Option::Some 1
    option_naturality_r: Result (Option I64), I64 = Option::Traversable::traverse result_pure_inc, option_natural_input_r

    result_natural_input_l: Result I64, I64 = Result::Ok 1
    result_natural_effect: Option (Result I64, I64) = (Result _, I64)::Traversable::traverse option_pure_inc, result_natural_input_l
    result_naturality_l: Result (Result I64, I64), I64 = option_to_result result_natural_effect
    result_natural_input_r: Result I64, I64 = Result::Ok 1
    result_naturality_r: Result (Result I64, I64), I64 = (Result _, I64)::Traversable::traverse result_pure_inc, result_natural_input_r

    vec_natural_effect: Option (Vec I64) = Vec::Traversable::traverse option_pure_inc, (make_values!)
    vec_naturality_l: Result (Vec I64), I64 = option_to_result vec_natural_effect
    vec_naturality_r: Result (Vec I64), I64 = Vec::Traversable::traverse result_pure_inc, (make_values!)
    encode_result_option option_naturality_l .println!
    encode_result_option option_naturality_r .println!
    encode_result_result result_naturality_l .println!
    encode_result_result result_naturality_r .println!
    encode_result_vec vec_naturality_l .println!
    encode_result_vec vec_naturality_r .println!

    "app-option".println!
    vec_app_option: Option (Vec I64) = Vec::Traversable::traverse option_stop, (make_values!)
    encode_option_vec vec_app_option .println!
    "monad-option".println!
    vec_monad_option: Option (Vec I64) = Vec::Traversable::traverse_m option_stop, (make_values!)
    encode_option_vec vec_monad_option .println!
    "app-result".println!
    vec_app_result: Result (Vec I64), I64 = Vec::Traversable::traverse result_stop, (make_values!)
    encode_result_vec vec_app_result .println!
    "monad-result".println!
    vec_monad_result: Result (Vec I64), I64 = Vec::Traversable::traverse_m result_stop, (make_values!)
    encode_result_vec vec_monad_result .println!

    "container-short".println!
    none_input: Option I64 = Option::None
    none_traverse: Option (Option I64) = Option::Traversable::traverse_m option_stop, none_input
    failed_input: Result I64, I64 = Result::Err 7
    failed_traverse: Option (Result I64, I64) = (Result _, I64)::Traversable::traverse_m option_stop, failed_input
    encode_option_option none_traverse .println!
    encode_option_result failed_traverse .println!
    0
"#,
    );

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec![
            "4",
            "5",
            "0",
            "123",
            "6",
            "3",
            "2",
            "2",
            "2",
            "2",
            "123",
            "123",
            "3",
            "3",
            "3",
            "3",
            "357",
            "357",
            "2",
            "2",
            "2",
            "2",
            "234",
            "234",
            "app-option",
            "1",
            "2",
            "3",
            "-1",
            "monad-option",
            "1",
            "2",
            "-1",
            "app-result",
            "1",
            "2",
            "3",
            "-9",
            "monad-result",
            "1",
            "2",
            "-9",
            "container-short",
            "-2",
            "-107",
        ]
    );
}

#[test]
fn test_stdlib_traversable_sequence() {
    let output = compile_and_run(
        r#"
main = ->
    mut effects: Vec (Option I64) = Vec::new!
    effects.push (Option::Some 4)
    effects.push (Option::Some 5)
    sequenced: Option (Vec I64) = sequence effects
    match sequenced
        Option::Some values =>
            values.len!.println!
            match (values.get 0)
                Option::Some value => *value .println!
                Option::None => 0.println!
        Option::None => 0.println!

    mut failed_effects: Vec (Option I64) = Vec::new!
    failed_effects.push (Option::Some 1)
    failed_effects.push Option::None
    failed_effects.push (Option::Some 3)
    failed: Option (Vec I64) = sequence failed_effects
    match failed
        Option::Some _ => 1.println!
        Option::None => -1 .println!
    0
"#,
    );

    assert_eq!(output.lines().collect::<Vec<_>>(), vec!["2", "4", "-1"]);
}

#[test]
fn test_stdlib_vec_eager_map_move_only_preserves_order() {
    let output = compile_and_run(
        r#"
string_len: String -> I64
string_len = value -> value.len!

print_value: I64 -> ()
print_value = value ->
    value.println!
    return

main = ->
    mut values: Vec String = Vec::new!
    values.push (String::from_str "a")
    values.push (String::from_str "bbb")
    mapped: Vec I64 = values.map string_len
    mapped.for_each_owned print_value
    0
"#,
    );

    assert_eq!(output.lines().collect::<Vec<_>>(), vec!["1", "3"]);
}

#[test]
fn test_stdlib_for_each_ranges_and_mutable_capture() {
    let output = compile_and_run(
        r#"
main = !->
    mut digits: I64 = 0
    for_each 1..=3, number !->
        digits = digits * 10 + number
    digits.println!
    for_each 4..6, number !-> number.println!
    for_each 5..5, number !-> number.println!
    for_each 5..3, number !-> number.println!
    for_each 5..=3, number !-> number.println!
    for_each 7..=7, number !-> number.println!
    for_each 9223372036854775806..=9223372036854775807, number !-> number.println!
    mut visits: I64 = 0
    for_each 0..100000, _ !->
        visits = visits + 1
    visits.println!
"#,
    );

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec![
            "123",
            "4",
            "5",
            "7",
            "9223372036854775806",
            "9223372036854775807",
            "100000"
        ]
    );
}

#[test]
fn test_stdlib_for_each_owned_and_borrowed_collections() {
    let output = compile_and_run(
        r#"
main = !->
    mut words: Vec String = Vec::new!
    words.push String::from_str "first"
    words.push String::from_str "second"
    for_each &words, word !-> word.println!
    words.len!.println!
    for_each words, word !-> word.println!

    values = [3, 4, 5]
    view: &[I64] = &values[..]
    for_each view, value !-> (*value).println!
    values[0].println!
"#,
    );

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec!["first", "second", "2", "first", "second", "3", "4", "5", "3"]
    );
}

#[test]
fn test_stdlib_for_each_book_fizzbuzz() {
    let source = include_str!("../../docs/examples/fizzbuzz.rk");
    let chapter = include_str!("../../docs/src/getting-started/first-project.md");
    let listing = chapter
        .split_once("```rock\n")
        .unwrap()
        .1
        .split_once("```")
        .unwrap()
        .0;
    assert_eq!(listing.trim(), source.trim());
    let expected = (1..=30)
        .map(|number| match (number % 3, number % 5) {
            (0, 0) => "FizzBuzz".to_string(),
            (0, _) => "Fizz".to_string(),
            (_, 0) => "Buzz".to_string(),
            _ => number.to_string(),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        compile_and_run(source).lines().collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn test_stdlib_for_each_hkt_foldable_carriers() {
    let output = compile_and_run(
        r#"
struct One T
    < value: T

impl Foldable for One
    foldl = mut step, initial, value ->
        step.call_mut (initial, value.value)

main = !->
    for_each (Option::Some 7), number !-> number.println!
    absent: Option I64 = Option::None
    for_each absent, number !-> number.println!
    success: Result I64, I64 = Result::Ok 9
    failure: Result I64, I64 = Result::Err 99
    for_each success, number !-> number.println!
    for_each failure, number !-> number.println!
    one = One
        value: String::from_str "custom foldable"
    for_each one, text !-> text.println!
"#,
    );
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec!["7", "9", "custom foldable"]
    );
}

#[test]
fn test_stdlib_for_each_rejects_open_ended_ranges() {
    for range in ["(..)", "(1..)", "(..3)", "(..=3)"] {
        let (output, success) = compile_and_run_with_status(&format!(
            "main = !->\n    for_each {range}, number !-> number.println!\n"
        ));
        assert!(!success, "range {range} should fail");
        assert_eq!(output.trim(), "for_each requires a bounded range");
    }
}

#[test]
fn test_stdlib_for_each_drops_owned_items_and_callback_once() {
    let output = compile_and_run(
        r#"
struct Item
    < id: I64

impl Drop for Item
    ~@drop = !-> self.id.println!

struct Counter
    < total: I64

impl FnMut Item, () for Counter
    type Output = ()
    ^@call_mut = item !->
        self.total = self.total + item.id

impl Drop for Counter
    ~@drop = !-> self.total.println!

main = !->
    mut items: Vec Item = Vec::new!
    first = Item
        id: 2
    second = Item
        id: 3
    items.push first
    items.push second
    counter = Counter
        total: 0
    for_each items, counter
"#,
    );
    assert_eq!(output.lines().collect::<Vec<_>>(), vec!["2", "3", "5"]);
}

#[test]
fn test_stdlib_for_each_consumes_owned_source() {
    compile_should_fail(
        r#"
main = !->
    mut values: Vec I64 = Vec::new!
    values.push 1
    for_each values, value !-> value.println!
    values.len!.println!
"#,
        "borrow of moved value",
    );
}

#[test]
fn test_stdlib_for_each_rejects_non_unit_action() {
    compile_should_fail(
        r#"
main = !->
    for_each 1..=3, value -> value
"#,
        "does not implement trait",
    );
}

#[test]
fn test_stdlib_vec_map_ref_preserves_source() {
    let output = compile_and_run(
        r#"
string_len: &String -> I64
string_len = value -> value.len!

print_value: I64 -> ()
print_value = value ->
    value.println!
    return

main = ->
    mut values: Vec String = Vec::new!
    values.push (String::from_str "aa")
    values.push (String::from_str "bbbb")
    mapped: Vec I64 = values.map_ref string_len
    mapped.for_each_owned print_value
    values.len!.println!
    0
"#,
    );

    assert_eq!(output.lines().collect::<Vec<_>>(), vec!["2", "4", "2"]);
}

#[test]
fn test_stdlib_vec_map_ref_contextually_types_closure_parameter() {
    let output = compile_and_run(
        r#"
print_value: I64 -> ()
print_value = value ->
    value.println!
    return

main = ->
    mut values: Vec String = Vec::new!
    values.push (String::from_str "aa")
    values.push (String::from_str "bbbb")
    mapped: Vec I64 = values.map_ref (value -> value.len!)
    mapped.for_each_owned print_value
    values.len!.println!
    0
"#,
    );

    assert_eq!(output.lines().collect::<Vec<_>>(), vec!["2", "4", "2"]);
}

#[test]
fn test_stdlib_vec_try_for_each_short_circuits() {
    let output = compile_and_run(
        r#"
visit: &I64 -> Result (), I64
visit = value ->
    item = *value
    item.println!
    if item == 2
        Result::Err 9
    else
        unit: () = make_unit!
        Result::Ok unit

make_unit: () -> ()
make_unit = -> return

main = ->
    mut values: Vec I64 = Vec::new!
    values.push 1
    values.push 2
    values.push 3
    result: Result (), I64 = values.try_for_each visit
    match result
        Result::Ok unit => 0.println!
        Result::Err error => error.println!
    values.len!.println!
    0
"#,
    );

    assert_eq!(output.lines().collect::<Vec<_>>(), vec!["1", "2", "9", "3"]);
}

#[test]
fn test_stdlib_vec_eager_operations_preserve_order_and_are_stack_safe() {
    let output = compile_and_run(
        r#"
double: I64 -> I64
double = value -> value * 2

print_ref: &I64 -> ()
print_ref = value ->
    *value .println!
    return

print_owned: I64 -> ()
print_owned = value ->
    value.println!
    return

is_odd: &I64 -> Bool
is_odd = value -> *value % 2 == 1

keep_even: I64 -> Option I64
keep_even = value ->
    if value % 2 == 0
        Option::Some (value * 10)
    else
        Option::None

try_double: I64 -> Result I64, I64
try_double = value ->
    value.println!
    if value == 3
        Result::Err 99
    else
        Result::Ok (value * 2)

main = ->
    mut empty: Vec I64 = Vec::new!
    mapped_empty: Vec I64 = empty.map double
    mapped_empty.len!.println!

    mut single: Vec I64 = Vec::new!
    single.push 7
    mapped_single: Vec I64 = single.map double
    mapped_single.for_each print_ref

    mut borrowed: Vec I64 = Vec::new!
    borrowed.push 1
    borrowed.push 2
    borrowed.push 3
    borrowed.for_each print_ref
    borrowed.len!.println!

    mut stateful: Vec I64 = Vec::new!
    stateful.push 1
    stateful.push 2
    stateful.push 3
    total = 0
    stateful.for_each_owned (value -> total = total + value)
    total.println!

    mut owned: Vec I64 = Vec::new!
    owned.push 4
    owned.push 5
    owned.for_each_owned print_owned

    mut retained: Vec I64 = Vec::new!
    retained.push 1
    retained.push 2
    retained.push 3
    retained.push 4
    retained.retain is_odd
    retained.for_each print_ref

    mut filtered: Vec I64 = Vec::new!
    filtered.push 1
    filtered.push 2
    filtered.push 3
    filtered.push 4
    kept: Vec I64 = filtered.filter is_odd
    kept.for_each print_ref

    mut filter_mapped: Vec I64 = Vec::new!
    filter_mapped.push 1
    filter_mapped.push 2
    filter_mapped.push 3
    filter_mapped.push 4
    transformed: Vec I64 = filter_mapped.filter_map keep_even
    transformed.for_each print_ref

    mut fallible: Vec I64 = Vec::new!
    fallible.push 1
    fallible.push 2
    fallible.push 3
    fallible.push 4
    match (fallible.try_map try_double)
        Result::Ok values => values.len!.println!
        Result::Err error => error.println!

    mut large: Vec I64 = Vec::new!
    i = 0
    while i < 10000
        large.push i
        i = i + 1
    doubled: Vec I64 = large.map double
    doubled.len!.println!
    0
"#,
    );

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec![
            "0", "14", "1", "2", "3", "3", "6", "4", "5", "1", "3", "1", "3", "20", "40", "1", "2",
            "3", "99", "10000",
        ]
    );
}

#[test]
fn test_stdlib_vec_eager_operations_drop_inputs_and_outputs_once() {
    let output = compile_and_run(
        r#"
struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

transform: Tracked -> Tracked
transform = value ->
    Tracked
        value: value.value + 10

main = ->
    mut values: Vec Tracked = Vec::new!
    values.push (Tracked
        value: 1)
    values.push (Tracked
        value: 2)
    values.push (Tracked
        value: 3)
    mapped: Vec Tracked = values.map transform
    mapped.len!.println!
    0
"#,
    );

    let mut lines = output.lines().collect::<Vec<_>>();
    lines.sort_unstable();
    assert_eq!(lines, vec!["1", "11", "12", "13", "2", "3", "3"]);
}

#[test]
fn test_stdlib_vec_filtering_and_try_map_drop_each_value_once() {
    let output = compile_and_run(
        r#"
struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

is_odd: &Tracked -> Bool
is_odd = value -> value.value % 2 == 1

filter_transform: Tracked -> Option Tracked
filter_transform = value ->
    if value.value % 2 == 1
        Option::Some (Tracked
            value: value.value + 100)
    else
        Option::None

try_transform: Tracked -> Result Tracked, I64
try_transform = value ->
    0 - value.value .println!
    if value.value == 502
        Result::Err 9
    else
        Result::Ok (Tracked
            value: value.value + 100)

main = ->
    mut retained: Vec Tracked = Vec::new!
    retained.push (Tracked
        value: 101)
    retained.push (Tracked
        value: 102)
    retained.push (Tracked
        value: 103)
    retained.retain is_odd

    mut filtered: Vec Tracked = Vec::new!
    filtered.push (Tracked
        value: 201)
    filtered.push (Tracked
        value: 202)
    filtered.push (Tracked
        value: 203)
    kept: Vec Tracked = filtered.filter is_odd

    mut filter_mapped: Vec Tracked = Vec::new!
    filter_mapped.push (Tracked
        value: 301)
    filter_mapped.push (Tracked
        value: 302)
    filter_mapped.push (Tracked
        value: 303)
    transformed: Vec Tracked = filter_mapped.filter_map filter_transform

    mut fallible: Vec Tracked = Vec::new!
    fallible.push (Tracked
        value: 501)
    fallible.push (Tracked
        value: 502)
    fallible.push (Tracked
        value: 503)
    fallible.push (Tracked
        value: 504)
    match (fallible.try_map try_transform)
        Result::Ok values => values.len!.println!
        Result::Err error => error.println!
    0
"#,
    );

    let mut values = output
        .lines()
        .map(|line| line.parse::<i64>().unwrap())
        .collect::<Vec<_>>();
    values.sort_unstable();
    assert_eq!(
        values,
        vec![
            -502, -501, 9, 101, 102, 103, 201, 202, 203, 301, 302, 303, 401, 403, 501, 502, 503,
            504, 601,
        ]
    );
}

#[test]
fn test_stdlib_option_result_fp_operators() {
    let output = compile_and_run(
        r#"
inc = x -> x + 1

keep_even_opt = x ->
    if x % 2 == 0
        Option::Some x
    else
        Option::None

keep_even_res = x ->
    if x % 2 == 0
        Result::Ok x
    else
        Result::Err 1

main = ->
    some = Option::Some 4
    none: Option I64 = Option::None
    fallback_none: Option I64 = Option::None
    some_for_result: Option I64 = Option::Some 12
    none_for_result: Option I64 = Option::None
    wrapped_inc: Option (I64 -> I64) = Option::Some inc
    ok: Result I64, I64 = Result::Ok 4
    err: Result I64, I64 = Result::Err 7
    mapped_ok: Result I64, I64 = Result::Ok 4
    mapped_ok_reverse: Result I64, I64 = Result::Ok 4
    fallback_err: Result I64, I64 = Result::Err 7
    wrapped_res_inc: Result (I64 -> I64), I64 = Result::Ok inc

    (some >>= keep_even_opt).unwrap_or 0 .println!
    (none >>= keep_even_opt).unwrap_or 0 .println!
    (inc <$> Option::Some 4).unwrap_or 0 .println!
    ((Option::Some 4) <&> inc).unwrap_or 0 .println!
    (wrapped_inc <*> Option::Some 4).unwrap_or 0 .println!
    (fallback_none <|> Option::Some 9).unwrap_or 0 .println!
    (some_for_result !> 5).fold (x -> x), (x -> x) .println!
    (none_for_result !> 7).fold (x -> x), (x -> x) .println!

    (ok >>= keep_even_res).unwrap_or 0 .println!
    (err >>= keep_even_res).unwrap_or 0 .println!
    (inc <$> mapped_ok).unwrap_or 0 .println!
    (mapped_ok_reverse <&> inc).unwrap_or 0 .println!
    (wrapped_res_inc <*> Result::Ok 4).unwrap_or 0 .println!
    (fallback_err <|> Result::Ok 9).unwrap_or 0 .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(
        lines,
        vec!["4", "0", "5", "5", "5", "9", "12", "7", "4", "0", "5", "5", "5", "9"]
    );
}

#[test]
fn test_stdlib_result_bifunctor_and_error_operator() {
    let output = compile_and_run(
        r#"
> stdlib::bifunctor::first
> stdlib::bifunctor::second

inc = value -> value + 1
double = value -> value * 2
add_ten = value -> value + 10
triple = value -> value * 3
double_after_inc = value -> double (inc value)
triple_after_add_ten = value -> triple (add_ten value)
identity = value -> value

main = ->
    mapped_ok: Result I64, I64 = bimap inc, add_ten, (Result::Ok 4)
    mapped_err: Result I64, I64 = bimap inc, add_ten, (Result::Err 7)
    first_ok: Result I64, I64 = first inc, (Result::Ok 4)
    second_err: Result I64, I64 = second add_ten, (Result::Err 7)
    operator_err: Result I64, I64 = (Result::Err 7) <!> add_ten

    identity_ok: Result I64, I64 = bimap identity, identity, (Result::Ok 4)
    identity_err: Result I64, I64 = bimap identity, identity, (Result::Err 7)

    composed_ok_l: Result I64, I64 = bimap double_after_inc, triple_after_add_ten, (Result::Ok 4)
    composed_ok_r: Result I64, I64 = bimap double, triple, (bimap inc, add_ten, (Result::Ok 4))
    composed_err_l: Result I64, I64 = bimap double_after_inc, triple_after_add_ten, (Result::Err 7)
    composed_err_r: Result I64, I64 = bimap double, triple, (bimap inc, add_ten, (Result::Err 7))

    mapped_ok.show!.println!
    mapped_err.show!.println!
    first_ok.show!.println!
    second_err.show!.println!
    operator_err.show!.println!
    identity_ok.show!.println!
    identity_err.show!.println!
    composed_ok_l.show!.println!
    composed_ok_r.show!.println!
    composed_err_l.show!.println!
    composed_err_r.show!.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(
        lines,
        vec![
            "Ok(5)", "Err(17)", "Ok(5)", "Err(17)", "Err(17)", "Ok(4)", "Err(7)", "Ok(10)",
            "Ok(10)", "Err(51)", "Err(51)",
        ]
    );
}

#[test]
fn test_stdlib_show_default_println() {
    let src = r#"
struct Named
    < value: I64

impl Show for Named
    @show = -> "Named(" + (int_to_string self.value) + ")"

main = ->
    n = Named
        value: 7
    result: Result I64, I64 = Result::Ok 4
    n.println!
    Option::Some 3 .println!
    result.println!
    0
"#;

    let output = compile_and_run(src);
    assert_eq!(output, "Named(7)\nSome(3)\nOk(4)\n");
}

#[test]
fn test_stdlib_option_result_show_prints_payloads() {
    let output = compile_and_run(
        r#"
> stdlib::option::Option
> stdlib::result::Result
> stdlib::io::IoError

main = ->
    none: Option I64 = Option::None
    ok_41: Result I64, I64 = Result::Ok 41
    err_4: Result I64, I64 = Result::Err 4
    ok_5: Result I64, I64 = Result::Ok 5
    (Option::Some 3).show!.println!
    none.show!.println!
    ok_41.show!.println!
    err_4.show!.println!
    ok_5.println!
    err_io_a: Result I64, IoError = Result::Err (IoError::Os (0 as I32))
    err_io_b: Result I64, IoError = Result::Err (IoError::Os (0 as I32))
    err_io_a.show!.println!
    err_io_b.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(
        lines,
        vec![
            "Some(3)",
            "None",
            "Ok(41)",
            "Err(4)",
            "Ok(5)",
            "Err(IoError)",
            "Err(IoError)"
        ]
    );
}

#[test]
fn test_stdlib_option_show_owned_string_is_non_consuming() {
    let output = compile_and_run(
        r#"
main = ->
    value = Option::Some (String::from_str "hello")
    value.show!.println!
    value.show!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "Some(hello)\nSome(hello)");
}

#[test]
fn test_stdlib_result_show_owned_string_is_non_consuming() {
    let output = compile_and_run(
        r#"
main = ->
    ok: Result String, String = Result::Ok (String::from_str "hello")
    err: Result String, String = Result::Err (String::from_str "oops")
    ok.show!.println!
    ok.show!.println!
    err.show!.println!
    err.show!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "Ok(hello)\nOk(hello)\nErr(oops)\nErr(oops)");
}

#[test]
fn test_stdlib_option_map_owned_string_consumes_receiver() {
    compile_should_fail(
        r#"
main = ->
    value = Option::Some (String::from_str "hello")
    mapped = value.map (s -> s.len!)
    mapped.unwrap_or 0 .println!
    again = value.show!
    again.println!
    0
"#,
        "borrow of moved value",
    );
}

#[test]
fn test_stdlib_result_map_owned_string_consumes_receiver() {
    compile_should_fail(
        r#"
main = ->
    value: Result String, String = Result::Ok (String::from_str "hello")
    mapped = value.map (s -> s.len!)
    mapped.unwrap_or 0 .println!
    again = value.show!
    again.println!
    0
"#,
        "borrow of moved value",
    );
}

#[test]
fn test_stdlib_result_map_err_owned_string_consumes_receiver() {
    compile_should_fail(
        r#"
main = ->
    value: Result String, String = Result::Err (String::from_str "oops")
    mapped = value.map_err (s -> s.len!)
    (mapped.fold (n -> n), (s -> s.len!)) .println!
    again = value.show!
    again.println!
    0
"#,
        "borrow of moved value",
    );
}

#[test]
fn test_stdlib_option_bind_owned_string_forwards_move_receiver() {
    let output = compile_and_run(
        r#"
bind: String -> Option I64
bind = s -> Option::Some (s.len!)

main = ->
    value = Option::Some (String::from_str "hello")
    mapped = value >>= bind
    mapped.unwrap_or 0 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "5");
}

#[test]
fn test_stdlib_result_bind_owned_string_forwards_move_receiver() {
    let output = compile_and_run(
        r#"
bind: String -> Result I64, String
bind = s -> Result::Ok (s.len!)

main = ->
    value: Result String, String = Result::Ok (String::from_str "hello")
    mapped = value >>= bind
    mapped.unwrap_or 0 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "5");
}

#[test]
fn test_result_method_sections_preserve_unresolved_payload_until_materialization() {
    let output = compile_and_run(
        r#"
> stdlib::result::Result

struct Source
    < value: I64

struct Sink
    < value: I64

impl Source
    @next: Result Sink, I64
    @next = -> Result::Ok (Sink
        value: self.value)

impl Sink
    @finish: Result I64, I64
    @finish = -> Result::Ok self.value

main = ->
    source = Source
        value: 42
    initial: Result Source, I64 = Result::Ok source
    match (initial >>= (.next!) >>= (.finish!))
        Result::Ok result =>
            result.println!
            0
        Result::Err _ => 1
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_stdlib_pipe_operator_function() {
    let output = compile_and_run(
        r#"
inc = x -> x + 1

main = ->
    41 |> inc .println!
    (((Option::Some 41) <&> inc).unwrap_or 0) .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["42", "42"]);
}

#[test]
fn test_module_local_pipe_operator_shadows_prelude() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();

    std::fs::write(
        dir.join("ops.rk"),
        "\
|> = x, f -> 43

< |>
",
    )
    .unwrap();

    std::fs::write(
        dir.join("test.rk"),
        "\
mod ops
> ops::|>

inc = x -> x + 1

main = ->
    41 |> inc .println!
    0
",
    )
    .unwrap();

    let config = test_config(dir.join("test.rk"), dir.clone());
    rock_lib::compile(&config).expect("Compilation failed");

    let binary = dir.join("test");
    let output = Command::new(&binary)
        .output()
        .expect("Failed to run compiled binary");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(stdout.trim(), "43");
}

#[test]
fn test_root_pipe_operator_shadows_prelude_with_same_param_names() {
    let output = compile_and_run(
        r#"
|> = x, f -> 43
inc = x -> x + 1

main = ->
    41 |> inc .println!
    0
"#,
    );

    assert_eq!(output.trim(), "43");
}

#[test]
fn test_application_binds_before_infix() {
    let output = compile_and_run(
        r#"
double = x -> x * 2

main = ->
    complete_argument = double 2 + 2
    complete_argument.println!
    application_first = (double 2) + 2
    application_first.println!
    double 2 + 2 .println!
    Option::Some 2 <&> (+ 2) .unwrap_or 0 .println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["8", "6", "8", "4"]);
}

#[test]
fn test_vec_get_option() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 10
    v.push 20
    match (v.get 0)
        Option::Some val => *val .println!
        Option::None => "None".println!
    match (v.get 5)
        Option::Some val => *val .println!
        Option::None => "None".println!
    match (v.get 1)
        Option::Some val => *val .println!
        Option::None => 99.println!
    match (v.get 99)
        Option::Some val => *val .println!
        Option::None => 99.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "None");
    assert_eq!(lines[2], "20");
    assert_eq!(lines[3], "99");
}

#[test]
fn test_range_for_loop() {
    let output = compile_and_run(
        r#"
main = ->
    for i in 0..5
        i.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines.len(), 5);
    assert_eq!(lines[0], "0");
    assert_eq!(lines[4], "4");
}

#[test]
fn test_range_sum() {
    let output = compile_and_run(
        r#"
main = ->
    sum = 0
    for i in 1..101
        sum = sum + i
    sum.println!
    0
"#,
    );
    assert_eq!(output.trim(), "5050");
}

#[test]
fn test_range_with_variable() {
    let output = compile_and_run(
        r#"
main = ->
    n = 5
    total = 0
    for i in 0..n
        total = total + i
    total.println!
    0
"#,
    );
    assert_eq!(output.trim(), "10");
}

#[test]
fn test_range_nested() {
    let output = compile_and_run(
        r#"
main = ->
    count = 0
    for i in 0..3
        for j in 0..4
            count = count + 1
    count.println!
    0
"#,
    );
    assert_eq!(output.trim(), "12");
}

#[test]
fn test_comments() {
    let output = compile_and_run(
        r#"
// top level comment
// another top level comment
main = ->
    // comment in body
    x = 42
    // comment between statements
    y = x + 8
    // comment before result
    y.println!
    // trailing comment
    0
"#,
    );
    assert_eq!(output.trim(), "50");
}

#[test]
fn test_str_indexing() {
    compile_should_fail(
        r#"
main = ->
    s = "hello"
    s[0].println!
    0
"#,
        "cannot index Str by integer",
    );
}

#[test]
fn test_as_cast_int_to_float() {
    // `x as F64` — integer to float
    let output = compile_and_run(
        r#"
main = ->
    x = 42
    f = x as F64
    f.println!
    0
"#,
    );
    assert_eq!(output.trim(), "42");
}

#[test]
fn test_as_cast_float_to_int() {
    // `f as I64` — float to int (truncates)
    let output = compile_and_run(
        r#"
main = ->
    f = 3.7
    i = f as I64
    i.println!
    0
"#,
    );
    assert_eq!(output.trim(), "3");
}

#[test]
fn test_as_cast_int_narrowing() {
    // `val as I32` — i64 narrowed to i32
    let output = compile_and_run(
        r#"
main = ->
    x = 1000
    y = x as I32
    y.println!
    0
"#,
    );
    assert_eq!(output.trim(), "1000");
}

#[test]
fn test_as_cast_u8_to_char() {
    // byte as Char — same as old ~U8ToChar
    let output = compile_and_run(
        r#"
main = ->
    bytes = [104 as U8]
    c = bytes[0] as Char
    c.println!
    0
"#,
    );
    assert_eq!(output.trim(), "h");
}

#[test]
fn test_char_show_literal() {
    let output = compile_and_run(
        r#"
main = ->
    'A'.println!
    0
"#,
    );
    assert_eq!(output.trim(), "A");
}

#[test]
fn test_as_cast_in_binop() {
    // `a + b as I64` — cast binds to RHS only (same-type cast verifies precedence)
    let output = compile_and_run(
        r#"
main = ->
    a = 10
    b = 5
    result = a + b as I64
    result.println!
    0
"#,
    );
    assert_eq!(output.trim(), "15");
}

#[test]
fn test_as_cast_ptr_to_int() {
    // Pointer-to-int cast: pointer value cast to I64 should be non-zero
    let output = compile_and_run(
        r#"
main = ->
    s = "hello"
    p = str_raw_ptr s
    i = p as I64
    if i > 0
        "ok".println!
    0
"#,
    );
    assert_eq!(output.trim(), "ok");
}

#[test]
fn test_type_conversion_via_as() {
    // `to_float` and `to_int` now use `as` under the hood
    let output = compile_and_run(
        r#"
main = ->
    x = 7
    f = to_float x
    f.println!
    y = 2.9
    i = to_int y
    i.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "7");
    assert_eq!(lines[1], "2");
}

#[test]
fn test_str_literal_index_out_of_bounds_is_rejected_as_str_indexing() {
    compile_should_fail(
        r#"
main = ->
    c = "hello"[10]
    c.println!
    0
"#,
        "cannot index Str by integer",
    );
}

#[test]
fn test_str_variable_indexing_is_rejected() {
    compile_should_fail(
        r#"
main = ->
    s = "hello"
    i = 3
    s[i].println!
    0
"#,
        "cannot index Str by integer",
    );
}

#[test]
fn test_ptr_deref_and_store() {
    let output = compile_and_run(
        r#"
main = ->
    x: I64 = 42
    ptr: *I64 = &x as *I64
    val = unsafe *ptr
    unsafe *ptr = 100
    val2 = unsafe *ptr
    val2.println!
    0
"#,
    );
    assert_eq!(output.trim(), "100");
}

#[test]
fn test_ptr_arithmetic_add() {
    // ~PtrOffset advances by n elements (element-stride, not byte-stride)
    let output = compile_and_run(
        r#"
extern malloc: I64 -> *U8

main = ->
    buf: *I64 = (malloc 24) as *I64
    unsafe *(~PtrOffset buf, 0) = 10
    unsafe *(~PtrOffset buf, 1) = 20
    unsafe *(~PtrOffset buf, 2) = 30
    a = unsafe *(~PtrOffset buf, 0)
    b = unsafe *(~PtrOffset buf, 1)
    c = unsafe *(~PtrOffset buf, 2)
    a.println!
    b.println!
    c.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "20");
    assert_eq!(lines[2], "30");
}

#[test]
fn test_ptr_arithmetic_sub() {
    // ~PtrOffset with a negative offset moves backwards by n elements
    let output = compile_and_run(
        r#"
extern malloc: I64 -> *U8

main = ->
    buf: *I64 = (malloc 24) as *I64
    unsafe
        *(~PtrOffset buf, 0) = 100
        *(~PtrOffset buf, 1) = 200
        end = ~PtrOffset buf, 1
        val = *(~PtrOffset end, -1)
        val.println!
    0
"#,
    );
    assert_eq!(output.trim(), "100");
}

#[test]
fn test_ptr_index_read() {
    // ptr[idx] reads the element at offset idx
    let output = compile_and_run(
        r#"
extern malloc: I64 -> *U8

main = ->
    buf: *I64 = (malloc 24) as *I64
    unsafe *(~PtrOffset buf, 0) = 7
    unsafe *(~PtrOffset buf, 1) = 8
    unsafe *(~PtrOffset buf, 2) = 9
    a = unsafe buf[0]
    b = unsafe buf[1]
    c = unsafe buf[2]
    a.println!
    b.println!
    c.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "7");
    assert_eq!(lines[1], "8");
    assert_eq!(lines[2], "9");
}

#[test]
fn test_ptr_index_write() {
    // ptr[idx] = val writes the element at offset idx
    let output = compile_and_run(
        r#"
extern malloc: I64 -> *U8

main = ->
    mut buf: *I64 = (malloc 24) as *I64
    unsafe buf[0] = 11
    unsafe buf[1] = 22
    unsafe buf[2] = 33
    a = unsafe buf[0]
    b = unsafe buf[1]
    c = unsafe buf[2]
    a.println!
    b.println!
    c.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "11");
    assert_eq!(lines[1], "22");
    assert_eq!(lines[2], "33");
}

#[test]
fn test_ptr_index_roundtrip() {
    // Mix of ptr[idx] reads and writes, including overwrite
    let output = compile_and_run(
        r#"
extern malloc: I64 -> *U8

main = ->
    mut buf: *I64 = (malloc 16) as *I64
    unsafe buf[0] = 1
    unsafe buf[1] = 2
    unsafe buf[0] = unsafe buf[0] + unsafe buf[1]
    unsafe buf[0].println!
    0
"#,
    );
    assert_eq!(output.trim(), "3");
}

#[test]
fn test_raw_slice_pointer_index_read() {
    let output = compile_and_run(
        r#"
read_second: &[I64] -> I64
read_second = s ->
    ptr: *[I64] = s as *[I64]
    unsafe ptr[1]

main = ->
    arr = [7, 8, 9]
    read_second &arr .println!
    0
"#,
    );

    assert_eq!(output.trim(), "8");
}

#[test]
fn test_raw_slice_pointer_index_write() {
    let output = compile_and_run(
        r#"
write_second: &[I64] -> I64
write_second = s ->
    mut ptr: *[I64] = s as *[I64]
    unsafe ptr[1] = 42
    s[1]

main = ->
    arr = [7, 8, 9]
    write_second &arr .println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_raw_slice_pointer_index_write_out_of_bounds_traps() {
    let (stdout, success) = compile_and_run_with_status(
        r#"
write_oob: &[I64] -> I64
write_oob = s ->
    mut ptr: *[I64] = s as *[I64]
    unsafe ptr[999] = 42
    0

main = ->
    arr = [7, 8, 9]
    write_oob &arr .println!
    0
"#,
    );

    assert!(!success);
    assert!(
        stdout.contains("index out of bounds"),
        "stdout was {stdout:?}"
    );
}

#[test]
fn test_raw_slice_pointer_index_read_requires_unsafe() {
    compile_should_fail(
        r#"
read_first: &[I64] -> I64
read_first = s ->
    ptr: *[I64] = s as *[I64]
    ptr[0]

main = ->
    arr = [7, 8, 9]
    read_first &arr .println!
    0
"#,
        "unsafe",
    );
}

#[test]
fn test_raw_slice_pointer_index_write_requires_unsafe() {
    compile_should_fail(
        r#"
write_first: &[I64] -> I64
write_first = s ->
    ptr: *[I64] = s as *[I64]
    ptr[0] = 42
    0

main = ->
    arr = [7, 8, 9]
    write_first &arr .println!
    0
"#,
        "unsafe",
    );
}

#[test]
fn safe_custom_pointer_index_impl_allows_read_and_write_outside_unsafe() {
    let exit_code = compile_and_run_without_stdlib(
        r#"lang index
< trait ReadAt Key
    lang output
    type ReadValue
    lang method
    @read_at: Key -> &Self::ReadValue

lang index_mut
< trait WriteAt Key
    lang output
    type WriteValue
    lang method
    ^@write_at: Key -> &mut Self::WriteValue

impl ReadAt I64 for *I64
    type ReadValue = I64
    @read_at = i -> unsafe &*(~PtrOffset *self, i)

impl WriteAt I64 for *I64
    type WriteValue = I64
    ^@write_at = i -> unsafe &mut *(~PtrOffset *self, i)

main = ->
    mut value = 1
    mut ptr: *I64 = &value as *I64
    ptr[0] = 9
    ptr[0]
"#,
    );

    assert_eq!(exit_code, 9);
}

#[test]
fn test_raw_slice_pointer_cast_to_int_is_rejected() {
    compile_should_fail(
        r#"
read_addr: &[I64] -> I64
read_addr = s ->
    ptr: *[I64] = s as *[I64]
    ptr as I64

main = ->
    arr = [7, 8, 9]
    read_addr &arr .println!
    0
"#,
        "Cannot cast fat raw slice pointer to integer",
    );
}

#[test]
fn test_int_to_raw_slice_pointer_cast_is_rejected() {
    compile_should_fail(
        r#"
main = ->
    addr: I64 = 0
    ptr = addr as *[I64]
    unsafe ptr[0]
"#,
        "Cannot cast integer to fat raw slice pointer",
    );
}

#[test]
fn test_raw_slice_pointer_arithmetic_is_rejected() {
    compile_should_fail(
        r#"
advance: &[I64] -> I64
advance = s ->
    ptr: *[I64] = s as *[I64]
    next = unsafe ptr + 1
    unsafe next[0] as I64

main = ->
    arr = [7, 8, 9]
    advance &arr .println!
    0
"#,
        "No implementation found for operator '+' on type *[I64]",
    );
}

#[test]
fn test_generic_impl_pointer_operator_rejects_unsized_instantiation() {
    compile_should_fail_with_source_diagnostic(
        r#"
struct PtrBox T
    < ptr: *T

impl PtrBox T
    unsafe @+ = offset -> @ptr + offset

main = ->
    arr = [7, 8, 9]
    ptr: *[I64] = (&arr) as *[I64]
    boxed = PtrBox
        ptr: ptr
    unsafe boxed + 1
    0
"#,
        "No implementation found for operator '+' on type PtrBox [I64]",
    );
}

#[test]
fn test_ptr_add_requires_unsafe() {
    compile_should_fail(
        r#"extern malloc: I64 -> *U8

main = ->
    buf: *I64 = (malloc 16) as *I64
    p = buf + 1
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_ptr_sub_requires_unsafe() {
    compile_should_fail(
        r#"extern malloc: I64 -> *U8

main = ->
    buf: *I64 = (malloc 16) as *I64
    p = unsafe ~PtrOffset buf, 1
    q = p - 1
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_ptr_add_sub_work_inside_unsafe_block() {
    let output = compile_and_run(
        r#"extern malloc: I64 -> *U8
extern free: *U8 -> ()

main = ->
    buf: *I64 = (malloc 24) as *I64
    unsafe
        *buf = 10
        *(buf + 1) = 20
        *(buf + 2) = 30
        *(buf + 1) .println!
        *(buf + 2 - 1) .println!
    free (buf as *U8)
    0
"#,
    );

    assert_eq!(output.trim(), "20\n20");
}

#[test]
fn test_local_sized_shadow_keeps_pointer_operator_bound_canonical() {
    let output = compile_and_run(
        r#"extern malloc: I64 -> *U8
extern free: *U8 -> ()

trait Sized

struct PtrBox T
    < ptr: *T

impl PtrBox T
    unsafe @offset = delta -> @ptr + delta

main = ->
    buf: *I64 = (malloc 16) as *I64
    unsafe
        *buf = 10
        *(buf + 1) = 42
        boxed = PtrBox
            ptr: buf
        ptr = boxed.offset 1
        *ptr .println!
    free (buf as *U8)
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_local_sized_shadow_where_clause_is_not_builtin_sized() {
    compile_should_fail(
        r#"trait Sized

struct Only

impl Sized for Only

requires_local_sized: T -> () where T: Sized
requires_local_sized = _ -> return

main = ->
    requires_local_sized 1
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn marked_renamed_sized_trait_drives_implicit_bounds_by_id() {
    let exit_code = compile_and_run_without_stdlib(
        r#"lang sized
< trait StaticLayout

struct Boxed T
    < value: T

requires_layout: T -> I64 where T: StaticLayout
requires_layout = _ -> 42

main = ->
    boxed = Boxed
        value: 1
    requires_layout boxed
"#,
    );

    assert_eq!(exit_code, 42);
}

#[test]
fn test_raw_pointer_write_read_round_trip() {
    let output = compile_and_run(
        r#"
extern malloc: I64 -> *U8
extern free: *U8 -> ()

main = ->
    ptr: *I64 = (malloc 8) as *I64
    unsafe *ptr = 42
    value = unsafe *ptr
    free (ptr as *U8)
    value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_drop_in_place_drops_raw_pointer_value() {
    let output = compile_and_run(
        r#"
extern malloc: I64 -> *U8
extern free: *U8 -> ()

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    ptr: *Tracked = (malloc 8) as *Tracked
    unsafe *ptr = Tracked
        value: 9
    unsafe drop_in_place ptr
    free (ptr as *U8)
    0
"#,
    );

    assert_eq!(output.trim(), "9");
}

#[test]
fn test_stdlib_box_drops_inner_value() {
    let output = compile_and_run(
        r#"
> stdlib::box_type::Box
> stdlib::drop::Drop

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    boxed = Box::new (Tracked
        value: 7)
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_stdlib_box_rejects_zero_sized_value() {
    let (_output, success) = compile_and_run_with_status(
        r#"
> stdlib::box_type::Box

struct Empty

main = ->
    boxed = Box::new Empty
    0
"#,
    );

    assert!(!success, "Box::new should reject zero-sized values");
}

#[test]
fn test_stdlib_box_new_uses_non_null_pointer() {
    let output = compile_and_run(
        r#"
> stdlib::box_type::Box

main = ->
    boxed = Box::new 42
    (boxed.as_ptr! as I64) != 0 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "true");
}

#[test]
fn test_inherent_drop_method_is_not_automatic_cleanup() {
    let output = compile_and_run(
        r#"
struct NotDrop
    < value: I64

impl NotDrop
    @drop = ->
        self.value.println!
        return

main = ->
    value = NotDrop
        value: 99
    value.drop!
    0
"#,
    );

    assert_eq!(output.trim(), "99");
}

#[test]
fn test_stdlib_box_drops_aggregate_field_value() {
    let output = compile_and_run(
        r#"
> stdlib::box_type::Box
> stdlib::drop::Drop

struct Tracked
    < value: I64

struct Wrapper
    < tracked: Tracked

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    boxed = Box::new (Wrapper
        tracked: Tracked
            value: 8)
    0
"#,
    );

    assert_eq!(output.trim(), "8");
}

#[test]
fn test_drop_order_runs_owner_before_field_cleanup() {
    let output = compile_and_run(
        r#"
> stdlib::drop::Drop

struct Child
    < value: I64

struct Owner
    < child: Child

impl Drop for Child
    ~@drop = ->
        self.value.println!
        return

impl Drop for Owner
    ~@drop = ->
        self.child.value + 100 .println!
        return

main = ->
    owner = Owner
        child: Child
            value: 4
    0
"#,
    );

    assert_eq!(output.trim().lines().collect::<Vec<_>>(), vec!["104", "4"]);
}

#[test]
fn test_branch_move_drops_value_once_after_merge() {
    let output = compile_and_run(
        r#"
> stdlib::drop::Drop

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

branch = cond, value ->
    a = Tracked
        value: value
    if cond
        b = a
    0

main = ->
    branch true, 1
    branch false, 2
    0
"#,
    );

    assert_eq!(output.trim().lines().collect::<Vec<_>>(), vec!["1", "2"]);
}

#[test]
fn test_field_assignment_drops_old_field_before_overwrite() {
    let output = compile_and_run(
        r#"
> stdlib::drop::Drop

struct Tracked
    < value: I64

struct Wrapper
    < tracked: Tracked

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    mut wrapper = Wrapper
        tracked: Tracked
            value: 1
    wrapper.tracked = Tracked
        value: 2
    0
"#,
    );

    assert_eq!(output.trim().lines().collect::<Vec<_>>(), vec!["1", "2"]);
}

#[test]
fn test_tuple_array_and_enum_payload_cleanup_drop_elements() {
    let output = compile_and_run(
        r#"
> stdlib::drop::Drop

struct Tracked
    < value: I64

enum MaybeTracked
    Some Tracked
    None

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    tuple = (Tracked
        value: 3, Tracked
        value: 4)
    array = [Tracked
        value: 5, Tracked
        value: 6]
    some = MaybeTracked::Some (Tracked
        value: 7)
    none = MaybeTracked::None
    0
"#,
    );

    let mut lines: Vec<&str> = output.trim().lines().collect();
    lines.sort();
    assert_eq!(lines, vec!["3", "4", "5", "6", "7"]);
}

#[test]
fn test_expression_statement_temporary_is_dropped() {
    let output = compile_and_run(
        r#"
> stdlib::drop::Drop

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

make = value -> Tracked
    value: value

main = ->
    make 8
    0
"#,
    );

    assert_eq!(output.trim(), "8");
}

#[test]
fn test_borrowed_temporary_drops_when_call_expression_finishes() {
    let output = compile_and_run(
        r#"
> stdlib::drop::Drop

struct Holder
    < value: I64

impl Drop for Holder
    ~@drop = ->
        2.println!
        return

make_holder = ->
    Holder
        value: 1

use_holder: &Holder -> ()
use_holder = holder ->
    holder.value.println!
    return

main = ->
    use_holder (&(make_holder!))
    3.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["1", "2", "3"]);
}

#[test]
fn test_return_expression_temporary_drops_before_returning_to_caller() {
    let output = compile_and_run(
        r#"
> stdlib::drop::Drop

struct Holder
    < value: I64

impl Drop for Holder
    ~@drop = ->
        2.println!
        return

make_holder = ->
    Holder
        value: 1

read_holder: &Holder -> I64
read_holder = holder -> holder.value

make_value = ->
    return read_holder (&(make_holder!))

main = ->
    make_value! .println!
    3.println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["2", "1", "3"]);
}

#[test]
fn test_owned_string_temporary_lives_through_ffi_call() {
    let output = compile_and_run(
        r#"
> stdlib::libc::puts
> stdlib::string_type::String

main = ->
    puts ((String::from_str "hello").as_ptr!)
    0
"#,
    );

    assert_eq!(output.trim(), "hello");
}

#[test]
fn test_stdlib_has_no_keep_alive_lifetime_workarounds() {
    let stdlib_files = ["convert.rk", "option.rk", "result.rk", "show.rk"];
    let offenders = stdlib_files
        .iter()
        .filter_map(|file| {
            let path = stdlib_path().join(file);
            let source = fs::read_to_string(&path).unwrap();
            source.contains("keep_alive").then_some(file.to_string())
        })
        .collect::<Vec<_>>();

    assert!(
        offenders.is_empty(),
        "stdlib should rely on full-expression temporary lifetimes, found keep_alive in {offenders:?}"
    );
}

#[test]
fn test_unrelated_trait_named_drop_is_not_automatic_cleanup() {
    let output = compile_and_run(
        r#"
trait Drop
    ~@drop: ()

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    value = Tracked
        value: 9
    0
"#,
    );

    assert_eq!(output.trim(), "");
}

#[test]
fn test_unrelated_trait_named_drop_allows_field_moves() {
    let output = compile_and_run(
        r#"
trait Drop
    ~@drop: ()

struct Child
    < value: I64

struct Tracked
    < child: Child

impl Drop for Tracked
    ~@drop = ->
        self.child.value.println!
        return

take_child = tracked -> tracked.child

main = ->
    tracked = Tracked
        child: Child
            value: 9
    child = take_child tracked
    child.value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "9");
}

#[test]
fn test_stdlib_box_move_drops_inner_value_once() {
    let (output, success) = compile_and_run_with_status(
        r#"
> stdlib::box_type::Box
> stdlib::drop::Drop

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    a = Box::new (Tracked
        value: 3)
    b = a
    0
"#,
    );

    assert!(success, "program failed with output {output:?}");
    assert_eq!(output.trim(), "3");
}

#[test]
fn test_returned_stdlib_box_binding_drops_inner_value_once() {
    let (output, success) = compile_and_run_with_status(
        r#"
> stdlib::box_type::Box
> stdlib::drop::Drop

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

make_box: () -> Box Tracked
make_box = -> Box::new (Tracked
    value: 5)

main = ->
    box = make_box!
    0
"#,
    );

    assert!(success, "program should not double-free or abort");
    assert_eq!(output.trim(), "5");
}

#[test]
fn test_method_by_value_owned_arg_drops_once() {
    let (output, success) = compile_and_run_with_status(
        r#"
> stdlib::box_type::Box
> stdlib::drop::Drop

struct Tracked
    < id: I64

impl Drop for Tracked
    ~@drop = ->
        self.id.println!
        return

struct Sink

impl Sink
    @take: Box Tracked -> ()
    @take = value ->
        return

main = ->
    sink = Sink
    sink.take (Box::new (Tracked
        id: 11))
    0
"#,
    );

    assert!(success, "program should not double-free or abort");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["11"]);
}

#[test]
fn test_partial_move_from_direct_drop_type_is_rejected() {
    compile_should_fail(
        r#"
> stdlib::box_type::Box
> stdlib::drop::Drop

struct Tracked
    < id: I64

impl Drop for Tracked
    ~@drop = ->
        self.id.println!
        return

struct Owner
    < value: Box Tracked

impl Drop for Owner
    ~@drop = ->
        99.println!
        return

take_value = owner -> owner.value

main = ->
    owner = Owner
        value: Box::new (Tracked
            id: 1)
    boxed = take_value owner
    0
"#,
        "because it implements Drop",
    );
}

#[test]
fn test_nested_partial_move_from_direct_drop_type_is_rejected() {
    compile_should_fail(
        r#"
> stdlib::box_type::Box
> stdlib::drop::Drop

struct Tracked
    < id: I64

impl Drop for Tracked
    ~@drop = ->
        self.id.println!
        return

struct Inner
    < value: Box Tracked

struct Owner
    < inner: Inner

impl Drop for Owner
    ~@drop = ->
        99.println!
        return

take_value = owner -> owner.inner.value

main = ->
    owner = Owner
        inner: Inner
            value: Box::new (Tracked
                id: 1)
    boxed = take_value owner
    0
"#,
        "because it implements Drop",
    );
}

#[test]
fn test_tuple_partial_move_from_direct_drop_type_is_rejected() {
    compile_should_fail(
        r#"
> stdlib::drop::Drop

struct Child
    < id: I64

struct Owner
    < pair: (Child, I64)

impl Drop for Owner
    ~@drop = ->
        99.println!
        return

make_child = ->
    Child
        id: 1

take_value = owner -> owner.pair.0

main = ->
    owner = Owner
        pair: (make_child!, 2)
    child = take_value owner
    0
"#,
        "because it implements Drop",
    );
}

#[test]
fn test_array_element_move_of_cleanup_value_is_rejected() {
    compile_should_fail(
        r#"
> stdlib::box_type::Box
> stdlib::drop::Drop

struct Tracked
    < id: I64

impl Drop for Tracked
    ~@drop = ->
        self.id.println!
        return

take_first: [Box Tracked; 1] -> Tracked
take_first = values -> *(values[0])

main = ->
    values: [Box Tracked; 1] = [Box::new (Tracked
        id: 1)]
    boxed = take_first values
    0
"#,
        "borrow conflict",
    );
}

#[test]
fn test_stdlib_box_reassignment_drops_old_and_new_values_once() {
    let output = compile_and_run(
        r#"
> stdlib::box_type::Box
> stdlib::drop::Drop

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    box = Box::new (Tracked
        value: 1)
    box = Box::new (Tracked
        value: 2)
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["1", "2"]);
}

#[test]
fn test_by_value_owned_parameter_is_dropped_when_unused() {
    let output = compile_and_run(
        r#"
> stdlib::box_type::Box
> stdlib::drop::Drop

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

consume_unused: Box Tracked -> ()
consume_unused = x ->
    return

main = ->
    consume_unused (Box::new (Tracked
        value: 10))
    0
"#,
    );

    assert_eq!(output.trim(), "10");
}

#[test]
fn test_by_value_owned_parameter_with_local_drop_is_dropped_when_unused() {
    let output = compile_and_run(
        r#"
> stdlib::drop::Drop

struct Tracked
    < value: I64

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

consume_unused: Tracked -> ()
consume_unused = x ->
    return

main = ->
    consume_unused (Tracked
        value: 11)
    0
"#,
    );

    assert_eq!(output.trim(), "11");
}

#[test]
fn test_assigning_moved_out_field_does_not_drop_stale_value() {
    let output = compile_and_run(
        r#"
> stdlib::box_type::Box
> stdlib::drop::Drop

struct Tracked
    < value: I64

struct Wrapper
    < field: Box Tracked

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    mut wrapper = Wrapper
        field: Box::new (Tracked
            value: 1)
    moved = wrapper.field
    wrapper.field = Box::new (Tracked
        value: 2)
    0
"#,
    );

    let mut lines: Vec<&str> = output.trim().lines().collect();
    lines.sort();
    assert_eq!(lines, vec!["1", "2"]);
}

#[test]
fn test_enum_partial_payload_move_drops_unmoved_payload_field() {
    let output = compile_and_run(
        r#"
> stdlib::box_type::Box
> stdlib::drop::Drop

struct Tracked
    < value: I64

enum Pair
    Both (Box Tracked) (Box Tracked)

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    pair = Pair::Both (Box::new (Tracked
        value: 1)), (Box::new (Tracked
        value: 2))
    match pair
        Pair::Both first, _ => 0
    0
"#,
    );

    let mut lines: Vec<&str> = output.trim().lines().collect();
    lines.sort();
    assert_eq!(lines, vec!["1", "2"]);
}

#[test]
fn test_parent_cleanup_skips_moved_field_but_drops_unmoved_sibling() {
    let output = compile_and_run(
        r#"
> stdlib::box_type::Box
> stdlib::drop::Drop

struct Tracked
    < value: I64

struct Wrapper
    < first: Box Tracked
    < second: Box Tracked

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    wrapper = Wrapper
        first: Box::new (Tracked
            value: 1)
        second: Box::new (Tracked
            value: 2)
    moved = wrapper.first
    0
"#,
    );

    let mut lines: Vec<&str> = output.trim().lines().collect();
    lines.sort();
    assert_eq!(lines, vec!["1", "2"]);
}

#[test]
fn test_branch_skipped_field_move_drops_both_fields_after_merge() {
    let output = compile_and_run(
        r#"
> stdlib::box_type::Box
> stdlib::drop::Drop

struct Tracked
    < value: I64

struct Wrapper
    < first: Box Tracked
    < second: Box Tracked

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    wrapper = Wrapper
        first: Box::new (Tracked
            value: 1)
        second: Box::new (Tracked
            value: 2)
    if false
        moved = wrapper.first
    0
"#,
    );

    let mut lines: Vec<&str> = output.trim().lines().collect();
    lines.sort();
    assert_eq!(lines, vec!["1", "2"]);
}

#[test]
fn test_branch_taken_field_move_drops_moved_field_once_and_sibling_after_merge() {
    let output = compile_and_run(
        r#"
> stdlib::box_type::Box
> stdlib::drop::Drop

struct Tracked
    < value: I64

struct Wrapper
    < first: Box Tracked
    < second: Box Tracked

impl Drop for Tracked
    ~@drop = ->
        self.value.println!
        return

main = ->
    wrapper = Wrapper
        first: Box::new (Tracked
            value: 1)
        second: Box::new (Tracked
            value: 2)
    if true
        moved = wrapper.first
    0
"#,
    );

    let mut lines: Vec<&str> = output.trim().lines().collect();
    lines.sort();
    assert_eq!(lines, vec!["1", "2"]);
}

#[test]
fn test_stdlib_raw_buffer_capacity_and_pointer_access() {
    let (output, success) = compile_and_run_with_status(
        r#"
> stdlib::raw_buffer::RawBuffer

main = ->
    empty: RawBuffer I64 = RawBuffer::new!
    empty.cap!.println!

    buf: RawBuffer I64 = unsafe RawBuffer::with_capacity (stdlib::mem::size_of 0), 2
    buf.cap!.println!
    ptr = buf.ptr!
    unsafe *ptr = 41
    unsafe *(~PtrOffset ptr, 1) = 42
    unsafe *ptr .println!
    unsafe *(~PtrOffset ptr, 1) .println!
    0
"#,
    );

    assert!(success, "program failed with output {output:?}");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["0", "2", "41", "42"]);
}

#[test]
fn test_stdlib_raw_buffer_with_capacity_requires_unsafe() {
    compile_should_fail(
        r#"
> stdlib::raw_buffer::RawBuffer

main = ->
    buf = RawBuffer::with_capacity (stdlib::mem::size_of 0), 2
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_stdlib_raw_buffer_as_slice_rejects_len_above_capacity() {
    let (_output, success) = compile_and_run_with_status(
        r#"
> stdlib::raw_buffer::RawBuffer

main = ->
    buf: RawBuffer I64 = RawBuffer::new!
    slice = unsafe buf.as_slice 1
    0
"#,
    );

    assert!(
        !success,
        "RawBuffer::as_slice should reject len above capacity"
    );
}

#[test]
fn test_stdlib_raw_buffer_as_slice_requires_unsafe() {
    compile_should_fail(
        r#"
> stdlib::raw_buffer::RawBuffer

main = ->
    buf: RawBuffer I64 = RawBuffer::new!
    slice = buf.as_slice 0
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_stdlib_raw_buffer_zero_capacity_as_slice_uses_non_null_buffer() {
    let output = compile_and_run(
        r#"
> stdlib::raw_buffer::RawBuffer

main = ->
    buf: RawBuffer I64 = unsafe RawBuffer::with_capacity (stdlib::mem::size_of 0), 0
    slice = unsafe buf.as_slice 0
    ((~ArrPtr (*slice)) as I64) != 0 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "true");
}

#[test]
fn test_global_alloc_requires_unsafe() {
    compile_should_fail(
        r#"
> stdlib::alloc::Global
> stdlib::alloc::Layout

main = ->
    layout = Layout
        size: 8
        align: 1
    ptr = Global::alloc layout
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_drop_in_place_wrapper_requires_unsafe() {
    compile_should_fail(
        r#"
extern malloc: I64 -> *U8

main = ->
    ptr: *I64 = (malloc 8) as *I64
    drop_in_place ptr
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_drop_in_place_rejects_non_pointer_argument() {
    compile_should_fail(
        r#"
main = ->
    unsafe ~DropInPlace 1
    0
"#,
        "DropInPlace expected raw pointer",
    );
}

#[test]
fn test_drop_in_place_requires_one_argument() {
    compile_should_fail(
        r#"
main = ->
    unsafe ~DropInPlace!
    0
"#,
        "DropInPlace requires exactly one argument",
    );
}

#[test]
fn test_drop_in_place_rejects_extra_arguments() {
    compile_should_fail(
        r#"
extern malloc: I64 -> *U8

main = ->
    ptr: *I64 = (malloc 8) as *I64
    unsafe ~DropInPlace ptr, ptr
    0
"#,
        "DropInPlace requires exactly one argument",
    );
}

#[test]
fn test_ptr_index_requires_unsafe() {
    // ptr[idx] outside unsafe block should be a compile error
    compile_should_fail(
        r#"
extern malloc: I64 -> *U8

main = ->
    buf: *I64 = (malloc 8) as *I64
    v = buf[0]
    0
"#,
        "unsafe",
    );
}

#[test]
fn test_unsupported_concrete_indexing_fails_during_lowering() {
    compile_should_fail(
        r#"
main = ->
    value = 42
    x = value[0]
    0
"#,
        "No implementation found for operator '[]' on type I64",
    );
}

#[test]
fn test_deref_receiver_without_index_support_fails_during_lowering() {
    compile_should_fail(
        r#"
trait Deref
    type Target
    @*: &Self::Target

struct Box T
    < value: T

impl Deref for Box T
    type Target = T
    @* = -> &@value

main = ->
    boxed = Box
        value: 7
    boxed[0]
    0
"#,
        "No implementation found for operator '[]' on type Box I64",
    );
}

// ── Borrow checker / MIR tests ──────────────────────────────────────────────

fn compile_example_should_fail(name: &str, expected_error: &str) {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let examples_dir = PathBuf::from(manifest_dir)
        .parent()
        .unwrap()
        .join("examples");
    let source_path = examples_dir.join(format!("{}.rk", name));
    let source = std::fs::read_to_string(&source_path)
        .unwrap_or_else(|_| panic!("Failed to read example: {}", name));
    compile_should_fail(&source, expected_error);
}

fn compile_example_should_pass(name: &str) {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let examples_dir = PathBuf::from(manifest_dir)
        .parent()
        .unwrap()
        .join("examples");
    let source_path = examples_dir.join(format!("{}.rk", name));
    let source = std::fs::read_to_string(&source_path)
        .unwrap_or_else(|_| panic!("Failed to read example: {}", name));
    compile_and_run(&source);
}

#[test]
fn test_borrow_use_after_move() {
    compile_example_should_fail("mir_tests/use_after_move", "borrow of moved value");
}

#[test]
fn test_borrow_double_move() {
    compile_example_should_fail("mir_tests/double_move", "use of moved value");
}

#[test]
fn test_borrow_uaf() {
    compile_example_should_fail("mir_tests/uaf", "borrow of moved value");
}

#[test]
fn test_borrow_mut_ref_requires_mutable_binding() {
    compile_should_fail(
        r#"
main = ->
    x = 1
    r = &mut x
    0
"#,
        "Cannot take a mutable reference to immutable binding",
    );
}

#[test]
fn returning_reference_to_call_temporary_is_rejected() {
    compile_should_fail(
        r#"
make_value = -> 1

bad: () -> &I64
bad = -> &make_value!

main = ->
    bad!
    0
"#,
        "temporary",
    );
}

#[test]
fn binding_reference_to_call_temporary_is_rejected() {
    compile_should_fail(
        r#"
make_value = -> 1

main = ->
    r = &make_value!
    *r .println!
    0
"#,
        "temporary",
    );
}

#[test]
fn struct_field_reference_to_call_temporary_is_rejected() {
    compile_should_fail(
        r#"
struct Holder
    < ref: &I64

make_value = -> 1

main = ->
    holder = Holder
        ref: &make_value!
    *holder.ref .println!
    0
"#,
        "temporary",
    );
}

#[test]
fn tuple_field_reference_to_call_temporary_is_rejected() {
    compile_should_fail(
        r#"
make_value = -> 1

main = ->
    pair = (&make_value!, 0)
    *pair.0 .println!
    0
"#,
        "temporary",
    );
}

#[test]
fn returning_reference_to_local_is_rejected() {
    compile_should_fail(
        r#"
bad: () -> &I64
bad = ->
    value = 1
    &value

main = ->
    bad!
    0
"#,
        "Cannot return reference",
    );
}

#[test]
fn returning_reference_to_by_value_param_is_rejected() {
    compile_should_fail(
        r#"
bad: I64 -> &I64
bad = value -> &value

main = ->
    x = *(bad 1)
    0
"#,
        "Cannot return reference",
    );
}

#[test]
fn returning_reference_via_known_call_to_local_is_rejected() {
    compile_should_fail(
        r#"
id_ref: &I64 -> &I64
id_ref = value -> value

bad: () -> &I64
bad = ->
    local = 1
    id_ref &local

main = ->
    x = *(bad!)
    0
"#,
        "Cannot return reference",
    );
}

#[test]
fn returning_reference_from_aggregate_field_is_rejected() {
    compile_should_fail(
        r#"
struct Holder
    < ref: &I64

bad: () -> &I64
bad = ->
    local = 1
    holder = Holder
        ref: &local
    holder.ref

main = ->
    x = *(bad!)
    0
"#,
        "Cannot return reference",
    );
}

#[test]
fn returning_option_some_reference_to_local_is_rejected() {
    compile_should_fail(
        r#"
bad: () -> Option &I64
bad = ->
    local = 1
    Option::Some (&local)

main = ->
    value = bad!
    0
"#,
        "Cannot return reference",
    );
}

#[test]
fn returning_struct_containing_reference_to_local_is_rejected() {
    compile_should_fail(
        r#"
struct Holder
    < ref: &I64

bad: () -> Holder
bad = ->
    local = 1
    Holder
        ref: &local

main = ->
    holder = bad!
    0
"#,
        "Cannot return reference",
    );
}

#[test]
fn returning_vec_get_from_local_vec_is_rejected() {
    compile_should_fail(
        r#"
> stdlib::vec::Vec

bad: () -> Option &I64
bad = ->
    mut v = Vec::new!
    v.push 1
    v.get 0

main = ->
    value = bad!
    0
"#,
        "Cannot return reference",
    );
}

#[test]
fn returning_reference_to_local_vec_index_is_rejected() {
    compile_should_fail(
        r#"
> stdlib::vec::Vec

bad: () -> &I64
bad = ->
    mut v = Vec::new!
    v.push 1
    &v[0]

main = ->
    value = bad!
    0
"#,
        "Cannot return reference",
    );
}

#[test]
fn returning_box_as_ref_from_local_box_is_rejected() {
    compile_should_fail(
        r#"
> stdlib::box_type::Box

bad: () -> &I64
bad = ->
    b = Box::new 1
    b.as_ref!

main = ->
    value = bad!
    0
"#,
        "Cannot return reference",
    );
}

#[test]
fn returning_raw_deref_from_local_box_pointer_is_rejected() {
    compile_should_fail(
        r#"
> stdlib::box_type::Box

bad: () -> &I64
bad = ->
    b = Box::new 1
    ptr = b.as_ptr!
    unsafe &*ptr

main = ->
    value = bad!
    0
"#,
        "Cannot return reference",
    );
}

#[test]
fn returning_vec_get_from_input_vec_reference_is_allowed() {
    let output = compile_and_run(
        r#"
> stdlib::vec::Vec

get_first: &Vec I64 -> Option &I64
get_first = v -> (*v).get 0

main = ->
    mut v = Vec::new!
    v.push 7
    match (get_first &v)
        Option::Some value => *value .println!
        Option::None => 0.println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn returning_vec_index_from_input_vec_reference_is_allowed() {
    let output = compile_and_run(
        r#"
> stdlib::vec::Vec

first: &Vec I64 -> &I64
first = v -> &((*v)[0])

main = ->
    mut v = Vec::new!
    v.push 7
    *(first &v) .println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn index_reference_returning_from_reference_key_blocks_key_mutation() {
    compile_should_fail(
        r#"
struct Key
    < value: I64

struct Container
    < value: I64

impl Index &Key for Container
    type Output = I64
    @index = key -> &key.value

main = ->
    mut key = Key
        value: 1
    container = Container
        value: 7
    saved = &container[&key]
    key.value = 2
    *saved .println!
    0
"#,
        "borrow conflict",
    );
}

#[test]
fn index_reference_returning_from_receiver_does_not_borrow_key() {
    let output = compile_and_run(
        r#"
struct Key
    < value: I64

struct Container
    < value: I64

impl Index &Key for Container
    type Output = I64
    @index = _ -> &@value

main = ->
    mut key = Key
        value: 1
    container = Container
        value: 7
    saved = &container[&key]
    key.value = 2
    *saved .println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn returning_reference_from_branch_known_call_to_local_is_rejected() {
    compile_should_fail(
        r#"
id_ref: &I64 -> &I64
id_ref = value -> value

choose: Bool -> &I64 -> &I64
choose = flag, input ->
    if flag
        local = 1
        id_ref &local
    else
        input

main = ->
    value = 2
    x = *(choose true, &value)
    0
"#,
        "does not live long enough",
    );
}

#[test]
fn returning_reference_from_multi_param_summary_is_rejected() {
    compile_should_fail(
        r#"
choose_ref: Bool -> &I64 -> &I64 -> &I64
choose_ref = flag, left, right ->
    if flag
        left
    else
        right

bad: Bool -> &I64 -> &I64
bad = flag, input ->
    local = 1
    choose_ref flag, &local, input

main = ->
    value = 2
    x = *(bad false, &value)
    0
"#,
        "does not live long enough",
    );
}

#[test]
fn returning_reference_from_extern_call_to_local_is_rejected() {
    compile_should_fail(
        r#"
extern id_ref: &I64 -> &I64

bad: () -> &I64
bad = ->
    local = 1
    id_ref &local

main = ->
    x = *(bad!)
    0
"#,
        "does not live long enough",
    );
}

#[test]
fn returning_input_reference_is_allowed() {
    let output = compile_and_run(
        r#"
id_ref: &I64 -> &I64
id_ref = value -> value

main = ->
    value = 7
    *(id_ref &value) .println!
    0
"#,
    );
    assert_eq!(output.trim(), "7");
}

#[test]
fn test_borrow_mut_ref_not_copy() {
    compile_should_fail(
        r#"
main = ->
    mut x = 1
    r = &mut x
    y = r
    z = r
    0
"#,
        "use of moved value",
    );
}

#[test]
fn test_borrow_mut_ref_moves_across_function_call() {
    compile_should_fail(
        r#"
helper = r -> 0

main = ->
    mut x = 1
    r = &mut x
    helper r
    y = r
    0
"#,
        "use of moved value",
    );
}

#[test]
fn test_borrow_mut_ref_blocks_later_use() {
    compile_should_fail(
        r#"
main = ->
    mut x = 1
    r = &mut x
    x.println!
    r.println!
    0
"#,
        "borrow conflict",
    );
}

#[test]
fn test_borrow_mut_ref_blocks_assignment() {
    compile_should_fail(
        r#"
main = ->
    mut x = 1
    r = &mut x
    x = 2
    r.println!
    0
"#,
        "borrow conflict",
    );
}

#[test]
fn test_borrow_reborrow_allows_write_through_reborrow() {
    compile_should_pass(
        r#"
main = ->
    mut x = 1
    r = &mut x
    s = &mut *r
    *s = 2
    0
"#,
    );
}

#[test]
fn test_borrow_reborrow_blocks_owner_assignment_before_reborrow_use() {
    compile_should_fail(
        r#"
main = ->
    mut x = 1
    r = &mut x
    s = &mut *r
    x = 2
    *s = 3
    0
"#,
        "borrow conflict",
    );
}

#[test]
fn test_borrow_mut_ref_blocks_drop_at_scope_end() {
    compile_should_pass(
        r#"
main = ->
    mut x = String::from_str "hello"
    r = &mut x
    0
"#,
    );
}

#[test]
fn test_borrow_shared_ref_blocks_assignment() {
    compile_should_pass(
        r#"
main = ->
    mut x = 1
    r = &x
    x = 2
    0
"#,
    );
}

#[test]
fn test_borrow_shared_ref_blocks_move() {
    compile_should_pass(
        r#"
main = ->
    mut x: Vec I64 = Vec::new!
    r = &x
    y = x
    0
"#,
    );
}

#[test]
fn test_borrow_reference_copy_releases_with_owner() {
    compile_should_pass(
        r#"
main = ->
    mut x = 1
    r = &x
    s = r
    x = 2
    0
"#,
    );
}

#[test]
fn test_borrow_reference_copy_keeps_original_alias_live() {
    compile_should_fail(
        r#"
main = ->
    mut x = 1
    r = &x
    s = r
    x = 2
    t = r
    0
"#,
        "borrow",
    );
}

#[test]
fn test_borrow_shared_ref_blocks_assignment_rust_parity() {
    compile_example_should_fail("mir_tests/shared_borrow_then_assign", "borrow");
}

#[test]
fn test_borrow_shared_ref_blocks_move_rust_parity() {
    compile_example_should_fail("mir_tests/shared_borrow_then_move", "borrow");
}

#[test]
fn test_trait_impl_requires_associated_type_definition() {
    compile_should_fail(
        r#"
trait Deref
    type Target
    @deref: () -> &Self::Target

struct Box T
    value: T

impl Deref for Box T
    @deref = -> @value

main = -> 0
"#,
        "required associated type 'Target'",
    );
}

#[test]
fn test_trait_impl_rejects_unknown_associated_type_definition() {
    compile_should_fail(
        r#"
trait Deref
    type Target
    @deref: () -> &Self::Target

struct Box T
    value: T

impl Deref for Box T
    type Output = T
    type Target = T
    @deref = -> @value

main = -> 0
"#,
        "unknown associated type 'Output'",
    );
}

#[test]
fn test_trait_impl_rejects_unknown_trait_name() {
    compile_should_fail(
        r#"
struct Box
    value: I64

impl MissingTrait for Box
    @value = -> @value

main = -> 0
"#,
        "unknown trait 'MissingTrait'",
    );
}

#[test]
fn test_binary_operator_resolves_by_symbol_not_trait_name() {
    let output = compile_and_run(
        r#"
trait FakePlus
    @+: Self -> Self

struct Wrapper
    < value: I64

impl FakePlus for Wrapper
    @+ = other -> Wrapper
        value: @value + other.value

main = ->
    left = Wrapper
        value: 1
    right = Wrapper
        value: 2
    sum = left + right
    sum.value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "3");
}

#[test]
fn test_binary_operator_allows_local_trait_name_matching_prelude_trait() {
    let output = compile_and_run(
        r#"
trait Num
    @+: Self -> Self

struct Wrapper
    < value: I64

impl Num for Wrapper
    @+ = other -> Wrapper
        value: 99

main = ->
    left = Wrapper
        value: 1
    right = Wrapper
        value: 2
    sum = left + right
    sum.value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "99");
}

#[test]
fn test_same_name_trait_methods_do_not_dispatch_by_method_name_only() {
    let output = compile_and_run(
        r#"
trait First
    @value: I64

trait Second
    @value: I64

struct Box
    < value: I64

impl Second for Box
    @value = -> 2

impl First for Box
    @value = -> 1

force_second: T -> I64 where T: Second
force_second = value -> value.value!

main = ->
    box = Box
        value: 0
    force_second box .println!
    0
"#,
    );

    assert_eq!(output.trim(), "2");
}

#[test]
fn test_generic_trait_bound_dispatch_uses_trait_arguments() {
    let output = compile_and_run(
        r#"
trait Pick T
    @pick: T

struct Both

impl Pick I64 for Both
    @pick = -> 1

impl Pick Bool for Both
    @pick = -> true

force_bool: T -> Bool where T: Pick Bool
force_bool = value -> value.pick!

main = ->
    both = Both
    force_bool both .println!
    0
"#,
    );

    assert_eq!(output.trim(), "true");
}

#[test]
fn test_concrete_trait_bound_solver_checks_trait_arguments() {
    compile_should_fail(
        r#"trait Pick T

impl Pick Bool for I64

requires_i64_pick: T -> () where T: Pick I64
requires_i64_pick = _ -> return

main = ->
    requires_i64_pick 1
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_concrete_trait_bound_solver_checks_receiver_and_trait_arg_substitution() {
    compile_should_fail(
        r#"trait Pick T

struct Box T
    < value: T

impl Pick T for Box T

requires_i64_pick: T -> () where T: Pick I64
requires_i64_pick = _ -> return

main = ->
    boxed = Box
        value: true
    requires_i64_pick boxed
    0
"#,
        "does not implement trait",
    );
}

#[test]
fn test_generic_trait_bound_dispatch_matches_generic_impl_trait_arguments() {
    let output = compile_and_run(
        r#"
trait Pick T
    @pick: T

struct Box T
    < value: T

impl Pick T for Box T
    @pick = -> @value

force_i64: T -> I64 where T: Pick I64
force_i64 = value -> value.pick!

main = ->
    box = Box
        value: 9
    force_i64 box .println!
    0
"#,
    );

    assert_eq!(output.trim(), "9");
}

#[test]
fn test_trait_default_method_substitutes_trait_generic_output() {
    let output = compile_and_run(
        r#"
trait Unwrap T
    @unwrap_or: T -> T
    @unwrap_or = fallback -> fallback

struct Empty

impl Unwrap I64 for Empty

main = ->
    empty = Empty
    empty.unwrap_or 9 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "9");
}

#[test]
fn test_trait_default_method_explicit_body_type_uses_trait_generic_argument() {
    let output = compile_and_run(
        r#"
trait Identity T
    @identity: T -> T
    @identity = value ->
        typed: T = value
        typed

struct Empty

impl Identity I64 for Empty

main = ->
    empty = Empty
    empty.identity 11 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "11");
}

#[test]
fn test_trait_default_method_substitutes_generic_trait_self_output_projection() {
    let output = compile_and_run(
        r#"
trait Project T
    type Output
    @project: T -> Self::Output
    @project = value -> value

struct Factory

impl Project I64 for Factory
    type Output = I64

main = ->
    factory = Factory
    factory.project 12 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "12");
}

#[test]
fn test_trait_default_method_body_must_match_concrete_signature_return() {
    compile_should_fail(
        r#"
trait Flag
    @flag: Bool
    @flag = -> 1

struct Marker

impl Flag for Marker

main = -> 0
"#,
        "return type mismatch",
    );
}

#[test]
fn test_trait_default_method_substitutes_trait_generic_in_nested_struct_return() {
    let output = compile_and_run(
        r#"
struct Box T
    < value: T

trait MakeBox T
    @make: T -> Box T
    @make = value ->
        Box
            value: value

struct Factory

impl MakeBox I64 for Factory

main = ->
    factory = Factory
    boxed = factory.make 7
    boxed.value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_trait_default_method_substitutes_trait_generic_in_nested_enum_return() {
    let output = compile_and_run(
        r#"
enum Maybe T
    Some T

trait MakeMaybe T
    @make: T -> Maybe T
    @make = value -> Maybe::Some value

struct Factory

impl MakeMaybe I64 for Factory

main = ->
    factory = Factory
    result = factory.make 7
    match result
        Maybe::Some value => value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_trait_default_method_lambda_captures_self_output_projection() {
    let output = compile_and_run(
        r#"
trait MakeClosure
    type Output
    @make: Self::Output -> (() -> Self::Output)
    @make = value -> -> value

struct Factory

impl MakeClosure for Factory
    type Output = I64

main = ->
    factory = Factory
    closure = factory.make 13
    closure!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "13");
}

#[test]
fn test_same_name_trait_default_methods_select_bound_trait_body() {
    let output = compile_and_run(
        r#"
trait First
    @value: I64
    @value = -> 1

trait Second
    @value: I64
    @value = -> 2

struct Box

impl First for Box

impl Second for Box

force_second: T -> I64 where T: Second
force_second = value -> value.value!

main = ->
    box = Box
    force_second box .println!
    0
"#,
    );

    assert_eq!(output.trim(), "2");
}

#[test]
fn test_same_name_generic_trait_default_methods_select_bound_trait_body() {
    let output = compile_and_run(
        r#"
trait First T
    @value: I64
    @value = -> 1

trait Second T
    @value: I64
    @value = -> 2

struct Box

impl First Bool for Box

impl Second I64 for Box

force_second: T -> I64 where T: Second I64
force_second = value -> value.value!

main = ->
    box = Box
    force_second box .println!
    0
"#,
    );

    assert_eq!(output.trim(), "2");
}

#[test]
fn test_same_name_trait_default_projection_is_ambiguous_without_bound_authority() {
    compile_should_fail(
        r#"
trait First
    type Output
    @value: Self::Output -> Self::Output
    @value = input -> input

trait Second
    type Output
    @value: Self::Output -> Self::Output
    @value = input -> input

struct Box

impl Second for Box
    type Output = I64

impl First for Box
    type Output = Bool

main = ->
    box = Box
    box.value 10 .println!
    0
"#,
        "Ambiguous selection for 'value'",
    );
}

#[test]
fn test_trait_default_method_substitutes_self_output_projection() {
    let output = compile_and_run(
        r#"
trait DefaultValue
    type Output
    @value: Self::Output -> Self::Output
    @value = fallback -> fallback

struct Box

impl DefaultValue for Box
    type Output = I64

main = ->
    box = Box
    box.value 12 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "12");
}

#[test]
fn test_trait_impl_body_must_match_associated_type_signature() {
    compile_should_fail(
        r#"
trait Deref
    type Target
    @deref: () -> &Self::Target

struct Box T
    < value: T

impl Deref for Box T
    type Target = T
    @deref = -> @value

main = -> 0
"#,
        "return type mismatch",
    );
}

#[test]
fn test_unary_neg_dispatches_through_stdlib_trait_with_associated_output() {
    let output = compile_and_run(
        r#"
struct Wrapper
    < value: I64

impl Neg for Wrapper
    type Output = I64
    @- = -> @value

main = ->
    value = Wrapper
        value: 7
    -value .println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_unary_not_dispatches_through_stdlib_trait_with_associated_output() {
    let output = compile_and_run(
        r#"
struct Wrapper
    < value: Bool

impl Not for Wrapper
    type Output = Bool
    @! = -> @value

main = ->
    value = Wrapper
        value: true
    !value .println!
    0
"#,
    );

    assert_eq!(output.trim(), "true");
}

#[test]
fn test_unary_not_dispatches_through_operator_symbol_with_associated_output() {
    let output = compile_and_run(
        r#"
struct Wrapper
    < value: Bool

impl Not for Wrapper
    type Output = I64
    @! = -> 11

force_not: Wrapper -> I64
force_not = value -> !value

main = ->
    value = Wrapper
        value: true
    force_not value .println!
    0
"#,
    );

    assert_eq!(output.trim(), "11");
}

#[test]
fn test_unary_neg_allows_local_trait_name_matching_prelude_trait() {
    let output = compile_and_run(
        r#"
trait Neg
    type Output
    @-: Self::Output

struct Wrapper
    < value: I64

impl Neg for Wrapper
    type Output = I64
    @- = -> @value

main = ->
    value = Wrapper
        value: 7
    -value .println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_unary_not_allows_local_trait_name_matching_prelude_trait() {
    let output = compile_and_run(
        r#"
trait Not
    type Output
    @!: Self::Output

struct Wrapper
    < value: Bool

impl Not for Wrapper
    type Output = Bool
    @! = -> @value

main = ->
    value = Wrapper
        value: true
    !value .println!
    0
"#,
    );

    assert_eq!(output.trim(), "true");
}

#[test]
fn test_index_dispatches_through_trait_with_associated_output() {
    let output = compile_and_run(
        r#"
struct Boxed
    < value: I64

impl Index I64 for Boxed
    type Output = I64
    @index = i -> &@value

main = ->
    boxed = Boxed
        value: 7
    boxed[0].println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn index_syntax_uses_marked_trait_member_and_output_ids() {
    let exit_code = compile_and_run_without_stdlib(
        r#"lang index
< trait Lookup Key
    lang output
    type Value
    lang method
    @lookup: Key -> &Self::Value

struct Boxed
    < value: I64

impl Lookup I64 for Boxed
    type Value = I64
    @lookup = _ -> &@value

main = ->
    boxed = Boxed value: 42
    boxed[0]
"#,
    );

    assert_eq!(exit_code, 42);
}

#[test]
fn index_syntax_selects_separate_read_and_write_authority() {
    let exit_code = compile_and_run_without_stdlib(
        r#"lang index
< trait ReadAt Key
    lang output
    type ReadValue
    lang method
    @read_at: Key -> &Self::ReadValue

lang index_mut
< trait WriteAt Key
    lang output
    type WriteValue
    lang method
    ^@write_at: Key -> &mut Self::WriteValue

struct Cell
    < read_value: I64
    < write_value: I64

impl ReadAt I64 for Cell
    type ReadValue = I64
    @read_at = _ -> &@read_value

impl WriteAt I64 for Cell
    type WriteValue = I64
    ^@write_at = _ -> &mut @write_value

probe = ->
    mut cell = Cell
        read_value: 2
        write_value: 4
    first = cell[0]
    cell[0] = 9
    match first
        2 => cell.write_value
        _ => 0

main = ->
    probe!
"#,
    );

    assert_eq!(exit_code, 9);
}

#[test]
fn cast_containing_index_assignment_is_rejected() {
    compile_should_fail_with_exact_diagnostic(
        r#"lang index
< trait ReadAt Key
    lang output
    type ReadValue
    lang method
    @read_at: Key -> &Self::ReadValue

lang index_mut
< trait WriteAt Key
    lang output
    type WriteValue
    lang method
    ^@write_at: Key -> &mut Self::WriteValue

struct Cell
    < read_value: I64
    < write_value: I64

impl ReadAt I64 for Cell
    type ReadValue = I64
    @read_at = _ -> &@read_value

impl WriteAt I64 for Cell
    type WriteValue = I64
    ^@write_at = _ ->
        self.write_value = 9
        &mut @write_value

probe = ->
    mut cell = Cell
        read_value: 2
        write_value: 4
    (cell[0] as I64) = 9
    0

main = ->
    probe!
"#,
        "Cast expressions cannot be used as assignment places",
    );
}

#[test]
fn read_only_index_rejects_indexed_assignment() {
    compile_should_fail_with_exact_diagnostic(
        r#"lang index
< trait Index Key
    lang output
    type Output
    lang method
    @index: Key -> &Self::Output

struct ReadOnly
    < value: I64

impl Index I64 for ReadOnly
    type Output = I64
    @index = _ -> &@value

main = ->
    mut value = ReadOnly value: 1
    value[0] = 9
    0
"#,
        "Cannot use mutable indexing because the IndexMut language-item protocol is unavailable",
    );
}

#[test]
fn index_mut_impl_requires_matching_index_impl() {
    compile_should_fail_with_exact_diagnostic(
        r#"lang index
< trait ReadAt Key
    lang output
    type ReadValue
    lang method
    @read_at: Key -> &Self::ReadValue

lang index_mut
< trait WriteAt Key
    lang output
    type WriteValue
    lang method
    ^@write_at: Key -> &mut Self::WriteValue

struct Cell
    < value: I64

impl WriteAt I64 for Cell
    type WriteValue = I64
    ^@write_at = _ -> &mut @value

main = ->
    0
"#,
        "IndexMut implementation for Cell with key I64 requires a matching Index implementation",
    );
}

#[test]
fn index_mut_impl_requires_matching_output() {
    compile_should_fail_with_exact_diagnostic(
        r#"lang index
< trait ReadAt Key
    lang output
    type ReadValue
    lang method
    @read_at: Key -> &Self::ReadValue

lang index_mut
< trait WriteAt Key
    lang output
    type WriteValue
    lang method
    ^@write_at: Key -> &mut Self::WriteValue

struct Cell
    < read_value: I64
    < write_value: Bool

impl ReadAt I64 for Cell
    type ReadValue = I64
    @read_at = _ -> &@read_value

impl WriteAt I64 for Cell
    type WriteValue = Bool
    ^@write_at = _ -> &mut @write_value

main = ->
    0
"#,
        "IndexMut implementation output Bool does not match Index output I64",
    );
}

#[test]
fn index_mut_impl_pairs_reordered_bounds() {
    compile_should_pass_without_stdlib(
        r#"trait First
trait Second

lang index
< trait ReadAt Key
    lang output
    type ReadValue
    lang method
    @read_at: Key -> &Self::ReadValue

lang index_mut
< trait WriteAt Key
    lang output
    type WriteValue
    lang method
    ^@write_at: Key -> &mut Self::WriteValue

struct Cell T
    < read_value: T
    < write_value: T

impl ReadAt I64 for Cell T where T: First, T: Second
    type ReadValue = T
    @read_at = _ -> &@read_value

impl WriteAt I64 for Cell T where T: Second, T: First
    type WriteValue = T
    ^@write_at = _ -> &mut @write_value

main = ->
    0
"#,
    );
}

#[test]
fn index_mut_impl_accepts_shared_mutable_slice_pair() {
    compile_should_pass_without_stdlib(
        r#"lang index
< trait ReadAt Key
    lang output
    type ReadValue
    lang method
    @read_at: Key -> &Self::ReadValue

lang index_mut
< trait WriteAt Key
    lang output
    type WriteValue
    lang method
    ^@write_at: Key -> &mut Self::WriteValue

impl ReadAt I64 for &[I64]
    type ReadValue = I64
    @read_at = key ->
        slice = *self
        ptr = ~ArrPtr slice
        unsafe &*(~PtrOffset ptr, key)

impl WriteAt I64 for &mut [I64]
    type WriteValue = I64
    ^@write_at = key ->
        slice = &*self
        ptr = ~ArrPtr slice
        unsafe &mut *(~PtrOffset ptr, key)

main = ->
    0
"#,
    );
}

#[test]
fn index_mut_impl_rejects_reverse_reference_orientation() {
    compile_should_fail_with_exact_diagnostic(
        r#"lang index
< trait ReadAt Key
    lang output
    type ReadValue
    lang method
    @read_at: Key -> &Self::ReadValue

lang index_mut
< trait WriteAt Key
    lang output
    type WriteValue
    lang method
    ^@write_at: Key -> &mut Self::WriteValue

struct Cell
    < read_value: I64
    < write_value: I64

impl ReadAt I64 for &mut Cell
    type ReadValue = I64
    @read_at = _ -> &@read_value

impl WriteAt I64 for &Cell
    type WriteValue = I64
    ^@write_at = _ -> &mut @write_value

main = ->
    0
"#,
        "IndexMut implementation for &Cell with key I64 requires a matching Index implementation",
    );
}

#[test]
fn index_mut_impl_rejects_same_shared_reference_orientation() {
    compile_should_fail_with_exact_diagnostic(
        r#"lang index
< trait ReadAt Key
    lang output
    type ReadValue
    lang method
    @read_at: Key -> &Self::ReadValue

lang index_mut
< trait WriteAt Key
    lang output
    type WriteValue
    lang method
    ^@write_at: Key -> &mut Self::WriteValue

struct Cell
    < read_value: I64
    < write_value: I64

impl ReadAt I64 for &Cell
    type ReadValue = I64
    @read_at = _ -> &@read_value

impl WriteAt I64 for &Cell
    type WriteValue = I64
    ^@write_at = _ -> &mut @write_value

main = ->
    0
"#,
        "IndexMut implementation for &Cell with key I64 requires a matching Index implementation",
    );
}

#[test]
fn index_syntax_prefers_marked_array_impl_before_builtin_key_coercion() {
    let exit_code = compile_and_run_without_stdlib(
        r#"lang index
< trait Lookup Key
    lang output
    type Value
    lang method
    @lookup: Key -> &Self::Value

impl Lookup U8 for [I64; 3]
    type Value = I64
    @lookup = _ ->
        ptr: *I64 = self as *I64
        unsafe &*ptr

main = ->
    values: [I64; 3] = [7, 8, 9]
    key: U8 = 1
    values[key]
"#,
    );

    assert_eq!(exit_code, 7);
}

#[test]
fn marked_slice_index_mut_updates_fixed_array() {
    let exit_code = compile_and_run_without_stdlib(
        r#"lang index
< trait ReadAt Key
    lang output
    type ReadValue
    lang method
    @read_at: Key -> &Self::ReadValue

lang index_mut
< trait WriteAt Key
    lang output
    type WriteValue
    lang method
    ^@write_at: Key -> &mut Self::WriteValue

impl ReadAt U8 for &[I64]
    type ReadValue = I64
    @read_at = key ->
        slice = *(*self)
        ptr: *I64 = (~ArrPtr slice) as *I64
        unsafe &*(~PtrOffset ptr, (key as I64))

impl WriteAt U8 for &mut [I64]
    type WriteValue = I64
    ~@write_at = key ->
        slice = *self
        ptr: *I64 = (~ArrPtr slice) as *I64
        unsafe &mut *(~PtrOffset ptr, (key as I64))

main = ->
    mut values: [I64; 3] = [1, 2, 3]
    values[(1 as U8)] = 9
    values[(1 as U8)]
"#,
    );

    assert_eq!(exit_code, 9);
}

#[test]
fn index_syntax_rejects_unmarked_same_spelled_trait_without_provider() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    let source_path = dir.join(format!("no_std_{id}.rk"));
    fs::write(
        &source_path,
        r#"trait Index Key
    type Output
    @index: Key -> &Self::Output

struct Boxed
    < value: I64

impl Index I64 for Boxed
    type Output = I64
    @index = _ -> &@value

main = ->
    boxed = Boxed value: 42
    boxed[0]
"#,
    )
    .unwrap();
    let mut config = test_config(source_path, dir);
    config.extern_artifacts.clear();
    config.no_std = true;
    config.no_prelude = true;

    let diagnostics =
        rock_lib::compile(&config).expect_err("unmarked Index must not receive [] dispatch");
    assert!(diagnostics.0.iter().any(|diagnostic| {
        diagnostic.message
            == "Cannot use indexing because the Index language-item protocol is unavailable"
    }));
}

#[test]
fn test_stdlib_index_trait_is_available_from_prelude() {
    let output = compile_and_run(
        r#"
struct Boxed
    < value: I64

impl Index I64 for Boxed
    type Output = I64
    @index = i -> &@value

main = ->
    boxed = Boxed
        value: 20
    boxed[0].println!
    0
"#,
    );

    assert_eq!(output.trim(), "20");
}

#[test]
fn test_stdlib_deref_trait_is_available_from_prelude() {
    let output = compile_and_run(
        r#"
struct Box T
    < value: T

impl Deref for Box T
    type Target = T
    @* = -> &@value

main = ->
    boxed = Box
        value: 12
    value: I64 = *boxed
    value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "12");
}

#[test]
fn test_index_rejects_wrong_concrete_argument_type_during_lowering() {
    compile_should_fail(
        r#"
trait Index Idx
    type Output
    @index: Idx -> &Self::Output

struct Boxed
    < value: I64

impl Index I64 for Boxed
    type Output = I64
    @index = i -> &@value

main = ->
    boxed = Boxed
        value: 7
    boxed[true]
    0
"#,
        "No implementation found for operator '[]'",
    );
}

#[test]
fn test_index_operator_rejects_local_shadow_index_trait() {
    compile_should_fail(
        r#"
trait Index Idx
    type Output
    @index: Idx -> &Self::Output

struct Boxed
    < value: I64

impl Index I64 for Boxed
    type Output = I64
    @index = i -> &@value

main = ->
    boxed = Boxed
        value: 7
    boxed[0]
"#,
        "No implementation found for operator '[]' on type Boxed",
    );
}

#[test]
fn test_local_index_trait_does_not_shadow_builtin_array_indexing() {
    let output = compile_and_run(
        r#"
trait Index Idx
    type Output
    @index: Idx -> &Self::Output

impl Index I64 for [I64; 3]
    type Output = Bool
    @index = i -> &(i == 0)

main = ->
    arr: [I64; 3] = [7, 8, 9]
    arr[0].println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_index_dispatches_to_matching_impl_when_receiver_has_multiple_index_impls() {
    let output = compile_and_run(
        r#"
struct Boxed
    < number: I64
    < flag: Bool

impl Index I64 for Boxed
    type Output = I64
    @index = i -> &@number

impl Index Bool for Boxed
    type Output = Bool
    @index = b -> &@flag

main = ->
    boxed = Boxed
        number: 7
        flag: true
    boxed[0].println!
    boxed[true].println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "7");
    assert_eq!(lines[1], "true");
}

#[test]
fn test_index_dispatches_to_matching_impl_when_receiver_instantiations_differ() {
    let output = compile_and_run(
        r#"
struct Box T
    < value: T

impl Index I64 for Box I64
    type Output = I64
    @index = i -> &@value

impl Index I64 for Box Bool
    type Output = Bool
    @index = i -> &@value

main = ->
    i = Box
        value: 7
    b = Box
        value: true
    i[0].println!
    b[0].println!
    0
"#,
    );

    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "7");
    assert_eq!(lines[1], "true");
}

#[test]
fn test_index_operator_rejects_impl_with_unsatisfied_concrete_bound() {
    compile_should_fail(
        r#"
trait Marker

struct Box T
    < value: T

impl Index I64 for Box T where T: Marker
    type Output = T
    @index = i -> &@value

main = ->
    boxed = Box
        value: 7
    boxed[0]
    0
"#,
        "No implementation found for operator '[]' on type Box I64",
    );
}

#[test]
fn test_unsafe_index_operator_method_requires_unsafe() {
    compile_should_fail(
        r#"struct Bag
    < value: I64

impl Index I64 for Bag
    type Output = I64

    unsafe @index = _ -> &@value

main = ->
    bag = Bag
        value: 7
    value = bag[0]
    0
"#,
        "requires an unsafe block",
    );
}

#[test]
fn test_deref_target_type_drives_unary_deref() {
    let output = compile_and_run(
        r#"
trait PointerLike
    type Target
    @*: &Self::Target

struct Box T
    < value: T

impl PointerLike for Box T
    type Target = T
    @* = -> &@value

main = ->
    boxed = Box
        value: 9
    value: I64 = *boxed
    value.println!
    0
"#,
    );

    assert_eq!(output.trim(), "9");
}

#[test]
fn test_static_method_value_preserves_selected_authority() {
    let output = compile_and_run(
        r#"
struct Math

impl Math
    double = value -> value + value

main = ->
    operation = Math::double
    operation 6 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "12");
}

#[test]
fn test_vec_as_slice_index_dispatches_to_slice() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 4
    v.push 7
    slice = v.as_slice!
    slice[1].println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_vec_indexing() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 4
    v.push 7
    v[1].println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_vec_index_assignment() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 4
    v.push 7
    v[1] = 42
    v[1].println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_vec_index_out_of_bounds_traps() {
    let (stdout, success) = compile_and_run_with_status(
        r#"
main = ->
    mut v = Vec::new!
    v.push 10
    slice = v.as_slice!
    slice[1].println!
    0
"#,
    );

    assert!(!success);
    assert!(
        stdout.contains("index out of bounds"),
        "stdout was {stdout:?}"
    );
}

#[test]
fn test_borrowed_vec_can_index_explicit_slice() {
    let output = compile_and_run(
        r#"
main = ->
    mut v = Vec::new!
    v.push 4
    v.push 7
    shared = &v
    slice = (*shared).as_slice!
    slice[1].println!
    0
"#,
    );

    assert_eq!(output.trim(), "7");
}

#[test]
fn test_slice_index_of_slice_returns_inner_slice_ref() {
    let output = compile_and_run(
        r#"
main = ->
    rows = ["ab", "cd"]
    rows[1].println!
    0
"#,
    );

    assert_eq!(output.trim(), "cd");
}

#[test]
fn test_generic_move_receiver_direct_call_path_uses_move_abi() {
    let output = compile_and_run(
        r#"
struct Box T
    < value: T

impl Box T
    ~@take = -> @value

main = ->
    box = Box
        value: 9
    box.take!.println!
    0
"#,
    );

    assert_eq!(output.trim(), "9");
}

#[test]
fn test_selfless_trait_default_stays_associated_function_when_injected() {
    let output = compile_and_run(
        r#"
trait Factory
    make = -> 5

struct Thing

impl Factory for Thing

main = ->
    Thing::make! .println!
    0
"#,
    );

    assert_eq!(output.trim(), "5");
}

#[test]
fn test_borrow_closure_shared_capture_blocks_mutation() {
    compile_example_should_fail("mir_tests/closure_shared_capture_blocks_mutation", "borrow");
}

#[test]
fn test_borrow_closure_copy_keeps_capture_loan_live() {
    compile_should_fail(
        r#"
main = ->
    mut x = 1
    f = -> x.println!
    g = f
    x = 2
    g!
    0
"#,
        "borrow",
    );
}

#[test]
fn test_borrow_closure_move_capture_moves_value() {
    compile_example_should_fail("mir_tests/closure_move_capture_moves_value", "moved");
}

#[test]
fn test_borrow_raw_pointer_from_borrow_preserves_reference_rules() {
    compile_example_should_fail("mir_tests/raw_pointer_from_borrow", "borrow");
}

#[test]
fn test_borrow_raw_pointer_copy_keeps_reference_rules() {
    compile_should_fail(
        r#"
main = ->
    mut x = 42
    r = &mut x
    ptr = r as *I64
    ptr2 = ptr
    y = x
    unsafe *ptr2 = 7
    0
"#,
        "borrow",
    );
}

#[test]
fn test_borrow_deref_mut_borrow_conflicts() {
    compile_should_fail(
        r#"
main = ->
    mut x = 1
    shared = &x
    mutable = &mut *shared
    0
"#,
        "borrow conflict",
    );
}

#[test]
fn test_borrow_mut_ref_branch_merge_blocks_later_use() {
    compile_should_fail(
        r#"
main = ->
    mut x = 1
    r = &mut x
    if x == 1
        r = &mut x
    else
        r = &mut x
    x = 2
    r.println!
    0
"#,
        "borrow conflict",
    );
}

#[test]
fn test_borrow_move_valid() {
    compile_example_should_pass("mir_tests/move_valid");
}

#[test]
fn test_borrow_loop() {
    compile_example_should_pass("mir_tests/loop_test");
}

#[test]
fn test_borrow_shared_borrow() {
    compile_example_should_pass("mir_tests/shared_borrow");
}

#[test]
fn test_borrow_conditional_move() {
    compile_example_should_pass("mir_tests/conditional_move");
}

#[test]
fn test_borrow_mut_and_shared() {
    compile_example_should_pass("mir_tests/mut_and_shared");
}

#[test]
fn test_reference_lifetime_cleanup() {
    compile_example_should_pass("mir_tests/reference_lifetime_cleanup");
}

#[test]
fn test_borrow_disjoint_fields() {
    compile_example_should_pass("mir_tests/field_borrowing");
}

fn proc_macro_host_with_response(
    response: rock_lib::macro_expansion::proc_macro::ProcMacroResponse,
) -> (
    rock_lib::macro_expansion::proc_macro::ProcMacroArtifact,
    PathBuf,
) {
    let response = rock_lib::macro_expansion::proc_macro::encode_response(&response).unwrap();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!(
        "rock-integration-proc-macro-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let host = temp_dir.join("proc_macro_host");
    fs::write(host.with_extension("response"), response).unwrap();
    fs::write(&host, "#!/bin/sh\ncat >/dev/null\ncat \"$0.response\"\n").unwrap();
    let mut permissions = fs::metadata(&host).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&host, permissions).unwrap();

    let artifact = rock_lib::macro_expansion::proc_macro::ProcMacroArtifact {
        artifact_format_version: rock_lib::products::PRODUCT_ARTIFACT_FORMAT_VERSION,
        crate_identity: "integration_macros".to_string(),
        protocol_version: rock_lib::macro_expansion::proc_macro::PROC_MACRO_PROTOCOL_VERSION,
        host_triple: "x86_64-unknown-linux-gnu".to_string(),
        executable: host,
        capabilities: vec![rock_lib::macro_expansion::proc_macro::ProcMacroCapability::Stdio],
        exports: vec![rock_lib::macro_expansion::proc_macro::ProcMacroExport {
            name: "make_main".to_string(),
            identity: "integration_macros::make_main".to_string(),
            kind: rock_lib::macro_expansion::proc_macro::ProcMacroKind::FunctionLike,
            input_shape: rock_lib::macro_expansion::proc_macro::ProcMacroInputShape::TokenStream,
        }],
    };

    (artifact, temp_dir)
}

#[derive(Serialize)]
struct ProcMacroToken {
    token_type: ProcMacroTokenType,
    span: ProcMacroSpan,
}

#[derive(Serialize)]
struct ProcMacroSpan {
    file_path: PathBuf,
    start: usize,
    end: usize,
}

#[derive(Serialize)]
#[allow(dead_code)]
enum ProcMacroTokenType {
    Ident(String),
    Type(String),
    Number(String),
    Float(String),
    Operator(String),
    Comment(String),
    StuckOperator(String),
    NativeOperator(String),
    Keyword(String),
    MacroVar(String),
    MacroInvoc(String),
    MacroRepeatOpen,
    MacroRepeatClose,
    Equal,
    OpenParen,
    CloseParen,
    OpenBracket,
    CloseBracket,
    Char(String),
    String(String),
    Arrow,
    CurriedArrow,
    TypeLambda,
    FatArrow,
    Coma,
    Colon,
    DoubleColon,
    Dot,
    DoubleDot,
    SpacedDot,
    Caret,
    Tilde,
    Arobase,
    Interogation,
    Ampersand,
    Indent(u8),
    Underscore,
    Eol,
    Eof,
}

fn proc_macro_token(token_type: ProcMacroTokenType) -> ProcMacroToken {
    ProcMacroToken {
        token_type,
        span: ProcMacroSpan {
            file_path: PathBuf::from("<proc-macro-output>"),
            start: 0,
            end: 0,
        },
    }
}

fn encode_generated_main_tokens() -> Vec<u8> {
    let generated_tokens = vec![
        proc_macro_token(ProcMacroTokenType::Indent(0)),
        proc_macro_token(ProcMacroTokenType::Ident("main".to_string())),
        proc_macro_token(ProcMacroTokenType::Equal),
        proc_macro_token(ProcMacroTokenType::Arrow),
        proc_macro_token(ProcMacroTokenType::Number("0".to_string())),
        proc_macro_token(ProcMacroTokenType::Eol),
        proc_macro_token(ProcMacroTokenType::Indent(0)),
        proc_macro_token(ProcMacroTokenType::Eol),
        proc_macro_token(ProcMacroTokenType::Eof),
    ];

    bincode::DefaultOptions::new()
        .with_limit(rock_lib::macro_expansion::proc_macro::MAX_PROC_MACRO_MESSAGE_BYTES)
        .serialize(&generated_tokens)
        .unwrap()
}

#[test]
fn test_function_like_proc_macro_expands_through_public_macro_api() {
    let config = rock_lib::Config {
        entry_file: PathBuf::new(),
        output_dir: PathBuf::new(),
        debug_print: Vec::new(),
        meta_files: Vec::new(),
        extern_artifacts: Vec::new(),
        source_providers: Vec::new(),
        current_crate_name: None,
        opt_level: 0,
        emit_llvm: false,
        no_link: false,
        emit_object: None,
        no_prelude: false,
        no_std: false,
        sysroot: None,
    };
    let response = rock_lib::macro_expansion::proc_macro::ProcMacroResponse::Expand {
        output: encode_generated_main_tokens(),
    };
    let (artifact, temp_dir) = proc_macro_host_with_response(response);
    let expanded = (|| {
        let input_program = rock_lib::parser::parse_string("%make_main", &config).unwrap();
        let context = rock_lib::macro_expansion::MacroExpansionContext::new(&config)
            .with_proc_macro_artifact(artifact);

        rock_lib::macro_expansion::expand_macros_with_context(input_program, &context)
    })();

    let _ = fs::remove_dir_all(temp_dir);
    let expanded = expanded.unwrap();

    assert!(expanded.module.top_level_from_ident("main").is_some());
}

#[test]
fn test_type_alias_survives_collection_and_lowering() {
    let output = compile_and_run(
        r#"
type Number = Identity I64
type Identity T = T

identity: Number -> Number
identity = value -> value

main = ->
    identity 42 .println!
    0
"#,
    );

    assert_eq!(output.trim(), "42");
}

#[test]
fn test_type_alias_cycle_is_rejected() {
    compile_should_fail(
        r#"
type Left = Right
type Right = Left

main = -> 0
"#,
        "type alias cycle",
    );
}

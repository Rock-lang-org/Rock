//! Deterministic POSIX failures, linked only into individual test executables.
//! A definition in the executable wins over the C library's on Linux and Darwin.

use std::process::{Command, Output};
use std::sync::atomic::Ordering;

use crate::support::*;

fn run_with_shim(source: &str, feature: &str, opt_level: u8) -> Output {
    let dir = test_temp_dir(TEST_COUNTER.fetch_add(1, Ordering::SeqCst));
    std::fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    let entry = dir.join("main.rk");
    let object = dir.join("main.o");
    let executable = dir.join("main");
    let shim = dir.join("shim.c");
    std::fs::write(&entry, source).unwrap();
    std::fs::write(&shim, include_str!("std_discard_errors/shim.c")).unwrap();
    let mut config = test_config(entry, dir.clone());
    config.no_link = true;
    config.emit_object = Some(object.clone());
    config.opt_level = opt_level;
    config
        .lint_levels
        .set("unused", rock_lib::lint::LintLevel::Deny)
        .unwrap();
    rock_lib::compile_with_warnings(&config).unwrap();

    let artifact = std_artifact_path();
    let products =
        rock_lib::products::CompilerProducts::read_artifact_from_path(&artifact).unwrap();
    let std_object = products.link.object_path.as_ref().unwrap();
    let std_object = if std_object.is_absolute() {
        std_object.clone()
    } else {
        artifact.parent().unwrap().join(std_object)
    };
    let mut link = Command::new("cc");
    link.arg(&object)
        .arg(std_object)
        .arg(&shim)
        .arg(format!("-D{feature}"));
    // Matches the compiler's own Linux link line; Darwin is always PIE.
    if cfg!(target_os = "linux") {
        link.arg("-no-pie");
    }
    let linked = run_test_command(
        link.arg("-lm")
            .arg("-pthread")
            .arg("-o")
            .arg(&executable),
    );
    assert!(
        linked.status.success(),
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    run_test_command(Command::new(executable).current_dir(&dir))
}

#[test]
fn test_explicit_close_reports_errors_once_for_files_and_sockets() {
    let source = r#"
> std::io::fs::File
> std::io::net::TcpListener
> std::io::net::TcpStream
> std::io::net::SocketAddrV4
> std::io::traits::IoError
extern mock_set_mode: I64 -> ()
extern mock_calls: () -> I64

report = result ->
    match result
        Result::Ok _ => "ok".println!
        Result::Err (IoError::Os code) => code.println!
        Result::Err _ => "unexpected error".println!

main = !->
    mock_set_mode 1
    match File::create "fixture"
        Result::Ok file => report (file.close!)
        Result::Err _ => "open failed".println!
    mock_calls!.println!
    match TcpListener::bind (SocketAddrV4::any 0)
        Result::Ok listener => report (listener.close!)
        Result::Err _ => "bind failed".println!
    mock_calls!.println!
    match TcpStream::connect (SocketAddrV4::localhost 1)
        Result::Ok stream => report (stream.close!)
        Result::Err _ => "connect failed".println!
    mock_calls!.println!

    mock_set_mode 0
    match File::create "fixture"
        Result::Ok file => report (file.close!)
        Result::Err _ => "open failed".println!
    mock_calls!.println!
    match TcpListener::bind (SocketAddrV4::any 0)
        Result::Ok listener => report (listener.close!)
        Result::Err _ => "bind failed".println!
    mock_calls!.println!
    match TcpStream::connect (SocketAddrV4::localhost 1)
        Result::Ok stream => report (stream.close!)
        Result::Err _ => "connect failed".println!
    mock_calls!.println!
    mock_set_mode 2
    match File::create "fixture"
        Result::Ok file => report (file.close!)
        Result::Err _ => "open failed".println!
    mock_calls!.println!
"#;
    for opt in [0, 3] {
        let output = run_with_shim(source, "TEST_CLOSE", opt);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "4\n1\n4\n2\n4\n3\nok\n1\nok\n2\nok\n3\n5\n1\n"
        );
    }
}

#[test]
fn test_drop_closes_once_and_constructor_errors_survive_failed_cleanup() {
    let source = r#"
> std::io::fs::File
> std::io::net::TcpListener
> std::io::net::TcpStream
> std::io::net::SocketAddrV4
> std::io::traits::IoError
extern mock_set_mode: I64 -> ()
extern mock_calls: () -> I64

report = result ->
    match result
        Result::Ok _ => "unexpected success".println!
        Result::Err (IoError::Os code) => code.println!
        Result::Err _ => "unexpected error".println!

main = !->
    mock_set_mode 1
    match File::create "fixture"
        Result::Ok _ => ()
        Result::Err _ => ()
    mock_calls!.println!
    match TcpListener::bind (SocketAddrV4::any 0)
        Result::Ok _ => ()
        Result::Err _ => ()
    mock_calls!.println!
    match TcpStream::connect (SocketAddrV4::localhost 1)
        Result::Ok _ => ()
        Result::Err _ => ()
    mock_calls!.println!
    mock_set_mode 3
    report (TcpListener::bind (SocketAddrV4::any 0))
    mock_calls!.println!
    mock_set_mode 4
    report (TcpListener::bind (SocketAddrV4::any 0))
    mock_calls!.println!
    mock_set_mode 5
    report (TcpStream::connect (SocketAddrV4::localhost 1))
    mock_calls!.println!
"#;
    let output = run_with_shim(source, "TEST_CLOSE", 0);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "1\n2\n3\n98\n1\n105\n1\n111\n1\n"
    );
}

#[test]
fn test_sleep_retries_remaining_time_and_does_not_truncate_long_intervals() {
    let source = r#"
> std::concurrency::thread::sleep_ms
extern mock_set_mode: I64 -> ()
extern mock_calls: () -> I64
main = !->
    mock_set_mode 0
    sleep_ms 0
    sleep_ms (-1)
    mock_calls!.println!
    sleep_ms 2500
    mock_calls!.println!
    mock_set_mode 2
    sleep_ms 9223372036854775807
    mock_calls!.println!
"#;
    for opt in [0, 3] {
        let output = run_with_shim(source, "TEST_SLEEP", opt);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(String::from_utf8_lossy(&output.stdout), "0\n3\n1\n");
    }
}

#[test]
fn test_unexpected_sleep_yield_and_detach_failures_do_not_continue() {
    for (feature, source) in [
        (
            "TEST_SLEEP",
            r#"
> std::concurrency::thread::sleep_ms
extern mock_set_mode: I64 -> ()
main = !->
    mock_set_mode 1
    sleep_ms 1
    "unreachable".println!
"#,
        ),
        (
            "TEST_YIELD",
            r#"
> std::concurrency::thread::yield_now
main = !->
    yield_now!
    "unreachable".println!
"#,
        ),
        (
            "TEST_YIELD",
            r#"
> std::concurrency::mutex::Mutex
main = !->
    mutex = Mutex::new 7
    held = mutex.lock!
    waiting = mutex.lock!
    waiting.get!.println!
    held.get!.println!
"#,
        ),
        (
            "TEST_DETACH",
            r#"
> std::concurrency::thread::spawn_detached
main = !->
    match spawn_detached (-> ())
        Result::Ok _ => ()
        Result::Err _ => "spawn failed".println!
    "unreachable".println!
"#,
        ),
    ] {
        let output = run_with_shim(source, feature, 0);
        assert_eq!(output.status.code(), Some(1), "{feature}: {output:?}");
        assert!(output.stdout.is_empty(), "{feature}: {output:?}");
    }
}

#[test]
fn test_wildcard_parameters_and_patterns_still_drop_owned_values() {
    let source = r#"
struct Tracked
impl Drop for Tracked
    ~@drop = !-> "dropped".println!
enum Packet
    Item Tracked
discard = _ -> ()
main = !->
    discard Tracked
    match Packet::Item Tracked
        Packet::Item _ => ()
    "after".println!
"#;
    for opt in [0, 3] {
        assert_eq!(
            compile_and_run_at_opt(source, opt),
            "dropped\ndropped\nafter\n"
        );
    }
}

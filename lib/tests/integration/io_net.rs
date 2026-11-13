use crate::support::*;
use std::sync::atomic::Ordering;

#[test]
fn test_socket_posix_ffi_surface_compiles_and_returns_errno() {
    let output = compile_and_run(
        r#"
> std::ffi::libc::socket
> std::ffi::libc::close
> std::ffi::libc::__errno_location

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
> std::io::net::Ipv4Addr
> std::io::net::SocketAddrV4

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
> std::io::net::TcpListener
> std::io::net::SocketAddrV4
> std::control::Result

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
> std::control::Result
> std::io::net::TcpListener
> std::io::net::SocketAddrV4

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
> std::control::Result
> std::io::net::TcpStream
> std::io::net::SocketAddrV4

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
> std::control::Result
> std::io::net::TcpStream
> std::io::net::SocketAddrV4
> std::io::IoError

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
> std::concurrency::Arc
> std::io::IoError
> std::io::Write
> std::control::Result
> std::io::net::TcpListener
> std::io::net::TcpStream
> std::io::net::SocketAddrV4

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
> std::io::stdout

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
fn test_std_arc_forwards_shared_write_bound() {
    let output = compile_and_run(
        r#"
> std::concurrency::Arc
> std::io::IoError
> std::io::Write
> std::control::Result

struct Sink

impl Write for Sink
    @write = _ -> Result::Ok 1
    @write_str = value -> std::io::write_str self, value
    @write_all = bytes -> std::io::write_all self, bytes

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
> std::io::IoError
> std::io::Write
> std::control::Result

struct PartialSink
struct ZeroSink

impl Write for PartialSink
    @write = bytes ->
        len = ~ArrayLen bytes
        Result::Ok (if len < 2
            len
        else
            2)
    @write_str = value -> std::io::write_str self, value
    @write_all = bytes -> std::io::write_all self, bytes

impl Write for ZeroSink
    @write = _ -> Result::Ok 0
    @write_str = value -> std::io::write_str self, value
    @write_all = bytes -> std::io::write_all self, bytes

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
> std::io::IoError
> std::io::Read
> std::io::Write
> std::control::Result
> std::io::net::TcpListener
> std::io::net::TcpStream
> std::io::net::SocketAddrV4

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
> std::io::IoError
> std::io::Read
> std::io::Write
> std::control::Result
> std::io::net::TcpListener
> std::io::net::TcpStream
> std::io::net::SocketAddrV4

accept_with_method_section: TcpListener -> Result TcpStream, IoError
accept_with_method_section = listener ->
    // `>>=` callbacks are FnMut: a lambda moving `listener` out is once-only, so
    // the address check uses `?` and the chain keeps only the method section.
    listener.local_addr!?
    (Result::Ok listener) >>= (.accept!)

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
fn test_tcp_listener_raw_fd_constructor_not_public() {
    compile_should_fail(
        r#"
> std::io::net::TcpListener

main = ->
    listener = TcpListener
        fd: 0
    0
"#,
        "Cannot construct struct 'TcpListener' because it has private fields",
    );
}

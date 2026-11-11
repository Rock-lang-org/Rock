use crate::support::*;
use std::fs;
use std::sync::atomic::Ordering;

#[test]
fn test_read_to_string_preserves_whole_file_bytes() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    // Put a multibyte character across the read-buffer boundary.
    let large = format!("{}é\n{}", "x".repeat(8191), "tail\0🙂\n".repeat(3000));
    let partial = format!("{}end", "b".repeat(8192));
    let cases = ["", "ordinary text\nsecond line\n", "héllo 世界🙂\n", "before\0after\n", &large, &partial];
    let mut source = String::from(
        r#"
> std::io::fs::read_to_string
> std::io::stdout
> std::control::Result

main = ->
    output = stdout!
"#,
    );
    let mut expected = Vec::new();
    for (index, contents) in cases.iter().enumerate() {
        fs::write(dir.join(format!("input{index}.txt")), contents.as_bytes()).unwrap();
        source.push_str(&format!(
            r#"    match read_to_string "input{index}.txt"
        Result::Ok text =>
            match output.write_all text.as_bytes!
                Result::Ok _ => 0
                Result::Err _ => return 2
        Result::Err _ => return 1
"#,
        ));
        expected.extend_from_slice(contents.as_bytes());
    }
    source.push_str("    0\n");
    let output = compile_and_run_in_working_dir(&source, &dir);
    assert!(output.status.success(), "binary failed: {output:?}");
    assert_eq!(output.stdout, expected);
}

#[test]
fn test_read_to_string_reports_errors() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    fs::write(dir.join("invalid.txt"), b"valid prefix\xff").unwrap();
    fs::write(dir.join("truncated.txt"), b"\xe2\x82").unwrap();
    fs::write(dir.join("nul_path.txt"), b"safe").unwrap();
    let output = compile_and_run_in_working_dir(
        r#"
> std::io::fs::read_to_string
> std::io::IoError
> std::control::Result
> std::ffi::libc::open
> std::ffi::libc::close

main = ->
    before = open "nul_path.txt".as_ptr!, (0 as I32), (0 as I32)
    if before < (0 as I32)
        return 6
    closed = close before
    match read_to_string "nul_path.txt"
        Result::Ok _ => 0
        _ => return 7
    match read_to_string "missing.txt"
        Result::Err (IoError::Os _) => 0
        _ => return 1
    match read_to_string "invalid.txt"
        Result::Err IoError::InvalidData => 0
        _ => return 2
    match read_to_string "truncated.txt"
        Result::Err IoError::InvalidData => 0
        _ => return 3
    match read_to_string "nul_path.txt\0suffix"
        Result::Err IoError::InvalidPath => 0
        _ => return 4
    // Opening a directory succeeds on Linux, but reading it fails.
    match read_to_string "."
        Result::Err (IoError::Os _) => 0
        _ => return 5
    // A leaked descriptor on any path would change the next available fd.
    after = open "nul_path.txt".as_ptr!, (0 as I32), (0 as I32)
    closed_after = close after
    if before != after
        return 8
    0
"#,
        &dir,
    );
    assert!(output.status.success(), "binary failed: {output:?}");
}

#[cfg(unix)]
#[test]
fn test_read_to_string_continues_after_short_reads() {
    use std::io::Write;

    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    let fifo = dir.join("input.pipe");
    assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
    let writer = std::thread::spawn(move || {
        let mut pipe = fs::OpenOptions::new().write(true).open(fifo).unwrap();
        // Separate short writes, including a UTF-8 character split across reads.
        for chunk in [b"first\n\xc3".as_slice(), b"\xa9\0middle".as_slice(), b"\nlast\n".as_slice()] {
            pipe.write_all(chunk).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    });
    let output = compile_and_run_in_working_dir(
        r#"
> std::io::fs::read_to_string
> std::io::stdout
> std::control::Result

main = ->
    match read_to_string "input.pipe"
        Result::Ok text =>
            output = stdout!
            match output.write_all text.as_bytes!
                Result::Ok _ => 0
                Result::Err _ => 2
        Result::Err _ => 1
"#,
        &dir,
    );
    writer.join().unwrap();
    assert!(output.status.success(), "binary failed: {output:?}");
    assert_eq!(output.stdout, "first\né\0middle\nlast\n".as_bytes());
}

#[test]
fn test_file_create_write_str_uses_generic_write_trait() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    let _cleanup = TestDirCleanup(dir.clone());

    let output = compile_and_run_in_working_dir(
        r#"
> std::io::fs::File
> std::io::IoError
> std::io::Write
> std::control::Result

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
> std::io::fs::File
> std::io::IoError
> std::io::Read
> std::control::Result

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
> std::io::fs::File
> std::io::IoError
> std::io::Write
> std::control::Result

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
> std::io::fs::File
> std::io::IoError
> std::control::Result

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
> std::io::fs::File
> std::io::IoError
> std::io::Write
> std::control::Result

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
> std::io::fs::File
> std::io::IoError
> std::control::Result

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
> std::io::fs::File
> std::io::IoError
> std::control::Result

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
> std::io::fs::File

main = ->
    file = File
        fd: 0
    0
"#,
        "Cannot construct struct 'std::io::fs::File' because it has private fields",
    );
}

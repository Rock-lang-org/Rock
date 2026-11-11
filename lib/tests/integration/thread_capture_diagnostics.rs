use crate::support::*;

#[test]
fn test_inline_owned_capture_reads_file_and_joins_signatureless() {
    let id = TEST_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    std::fs::write(dir.join("input.txt"), "inline thread contents").unwrap();
    let output = compile_and_run_in_working_dir(
        r#"
> std::concurrency::thread::spawn
> std::io::fs::read_to_string

entry = file ->
    filename = String::from file
    spawn (-> read_to_string filename.as_str!)
        >>= (.join!)
        <&> (.println!)
        <!> e !-> %println "thread failed: ", e

main = !-> entry "input.txt"
"#,
        &dir,
    );
    assert!(output.status.success(), "binary failed: {output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("inline thread contents"), "{stdout}");
    assert!(!stdout.contains("thread failed"), "{stdout}");
}

#[test]
fn test_inline_owned_capture_drops_once_after_borrowing_body() {
    let output = compile_and_run(
        r#"
> std::concurrency::thread::spawn

struct Owned
    < value: String
impl Owned
    @read = -> self.value.len!
impl Drop for Owned
    ~@drop = !-> "dropped".println!

main = !->
    owned = Owned
        value: String::from "hello"
    match spawn (-> owned.read!)
        Result::Ok handle =>
            match handle.join!
                Result::Ok length => length.println!
                Result::Err _ => "join failed".println!
        Result::Err _ => "spawn failed".println!
"#,
    );
    assert_eq!(output.lines().collect::<Vec<_>>(), ["dropped", "5"]);
}

#[test]
fn test_spawn_rejects_capture_borrowed_for_later_source_use() {
    for later in ["filename.len!.println!", "spawn (-> filename.len!)"] {
        compile_should_fail(
            &format!(r#"
> std::concurrency::thread::spawn

main = !->
    filename = String::from "owned"
    spawn (-> filename.len!)
    {later}
"#),
            "shared borrow",
        );
    }
}

#[test]
fn test_named_closures_follow_same_last_use_policy() {
    let output = compile_and_run(
        r#"
main = !->
    filename = String::from "local"
    first = -> filename.len!
    second = -> filename.len!
    first!.println!
    second!.println!
    filename.len!.println!
"#,
    );
    assert_eq!(output.lines().collect::<Vec<_>>(), ["5", "5", "5"]);
    let output = compile_and_run(
        r#"
> std::concurrency::thread::spawn
main = !->
    filename = String::from "local"
    task = -> filename.len!
    match spawn task
        Result::Ok handle =>
            match handle.join!
                Result::Ok length => length.println!
                Result::Err _ => "join failed".println!
        Result::Err _ => "spawn failed".println!
"#,
    );
    assert_eq!(output.trim(), "5");
    compile_should_fail(
        r#"
> std::concurrency::thread::spawn
main = !->
    filename = String::from "local"
    task = -> filename.len!
    spawn (-> filename.len!)
    task!.println!
"#,
        "borrow",
    );
}

#[test]
fn test_inline_owned_capture_generic_and_method_consuming_callables() {
    let output = compile_and_run(
        r#"
apply: F -> I64 where F: FnOnce (), I64
apply = task -> task!

borrow: &F -> I64 where F: Fn (), I64
borrow = task -> task!

struct Runner
impl Runner
    @apply: F -> I64 where F: FnOnce (), I64
    @apply = task -> task!

main = !->
    first = String::from "first"
    (apply (-> first.len!)).println!
    runner = Runner
    second = String::from "second"
    (runner.apply (-> second.len!)).println!
    third = String::from "borrow"
    (borrow &(-> third.len!)).println!
    third.len!.println!
"#,
    );
    assert_eq!(output.lines().collect::<Vec<_>>(), ["5", "6", "6", "6"]);
    for call in ["apply (-> owned.len!)", "runner.apply (-> owned.len!)"] {
        compile_should_pass(
            &format!(r#"
apply: F -> I64 where F: FnOnce (), I64
apply = task -> task!
struct Runner
impl Runner
    @apply: F -> I64 where F: FnOnce (), I64
    @apply = task -> task!
main = !->
    runner = Runner
    owned = String::from "owned"
    {call}
    owned.len!.println!
"#),
        );
    }
}

#[test]
fn test_later_use_keeps_capture_borrowed_with_shared_and_consuming_bounds() {
    let output = compile_and_run(
        r#"
apply: F -> I64 where F: Fn (), I64, F: FnOnce (), I64
apply = task -> task!
main = !->
    owned = String::from "borrowed"
    (apply (-> owned.len!)).println!
    owned.len!.println!
"#,
    );
    assert_eq!(output.lines().collect::<Vec<_>>(), ["8", "8"]);
}

#[test]
fn test_unused_spawn_owned_return_types_need_no_extra_marker_impl() {
    let output = compile_and_run(
        r#"
> std::concurrency::thread::spawn
> std::io::traits::IoError

string_task = !->
    spawn (-> String::from "owned")
error_task = !->
    spawn (-> IoError::InvalidInput)
string_value: () -> String
string_value = -> String::from "owned"
error_value: () -> IoError
error_value = -> IoError::InvalidInput
annotated_tasks = !->
    spawn string_value
    spawn error_value
main = !-> "ok".println!
"#,
    );
    assert_eq!(output.trim(), "ok");
}

#[test]
fn test_spawn_read_to_string_without_captures() {
    let output = compile_and_run(
        r#"
> std::concurrency::thread::spawn
> std::io::fs::read_to_string

main = !->
    match spawn (-> read_to_string "missing-thread-capture-test.txt")
        Result::Ok handle =>
            match handle.join!
                Result::Ok _ => "joined".println!
                Result::Err _ => "join failed".println!
        Result::Err _ => "spawn failed".println!
"#,
    );
    assert_eq!(output.trim(), "joined");
}

#[test]
fn test_unused_spawn_borrowed_filename_reports_closure_capture() {
    for signature in ["", "entry: &Str -> ()\n"] {
        compile_should_fail(
            &format!(r#"
> std::concurrency::thread::spawn
> std::io::fs::read_to_string

{signature}entry = filename !->
    spawn (-> read_to_string filename)
main = !-> ()
"#),
            "of `&Str`",
        );
    }
}

#[test]
fn test_spawn_borrowed_capture_is_independent_of_return_type() {
    for result in ["IoError::InvalidInput", "String::from filename"] {
        compile_should_fail(
            &format!(r#"
> std::concurrency::thread::spawn
> std::io::traits::IoError
> std::io::fs::read_to_string

entry = filename !->
    spawn (->
        read = read_to_string filename
        {result})
main = !-> ()
"#),
            "of `&Str`",
        );
    }
    compile_should_fail(
        r#"
> std::concurrency::thread::spawn
> std::io::fs::read_to_string

main = !->
    filename = String::from "caller-owned.txt"
    borrowed = filename.as_str!
    spawn (-> read_to_string borrowed)
"#,
        "of `&Str`",
    );
}

use crate::support::*;

#[test]
fn test_result_send_sync_owned_payloads() {
    let output = compile_and_run(
        r#"
> std::concurrency::thread::spawn
> std::io::traits::IoError

require_sync: &T -> () where T: Sync
require_sync = value -> ()

produce: Bool -> Result String, IoError
produce = success ->
    if success
        Result::Ok (String::from "hello")
    else
        Result::Err IoError::InvalidInput

run = value ->
    require_sync (&value)
    match spawn (-> value)
        Result::Ok handle =>
            match handle.join!
                Result::Ok result =>
                    match result
                        Result::Ok text => text.println!
                        Result::Err _ => "error".println!
                Result::Err _ => "join failed".println!
        Result::Err _ => "spawn failed".println!

main = !->
    run (produce true)
    run (produce false)
"#,
    );
    assert_eq!(output.trim(), "hello\nerror");
}

#[test]
fn test_result_send_sync_rejects_pointer_in_either_payload() {
    for marker in ["Send", "Sync"] {
        for (ty, value) in [
            ("Result I64, *I64", "Result::Ok 1"),
            ("Result *I64, I64", "Result::Err 1"),
        ] {
            compile_should_fail(
                &format!(
                    r#"
require_marker: &T -> () where T: {marker}
require_marker = value -> ()

main = !->
    value: {ty} = {value}
    require_marker (&value)
"#,
                ),
                "does not implement trait",
            );
        }
    }
}

#[test]
fn test_std_thread_spawn_join_returns_value() {
    let output = compile_and_run(
        r#"
> std::concurrency::thread::spawn

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
fn test_std_thread_handle_drop_detaches_without_waiting() {
    let output = compile_and_run(
        r#"
> std::concurrency::thread::sleep_ms
> std::concurrency::thread::spawn

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
fn test_std_detached_thread_drops_unobserved_result() {
    let output = compile_and_run(
        r#"
> std::concurrency::thread::sleep_ms
> std::concurrency::thread::spawn

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
fn test_std_thread_spawn_accepts_owned_send_capture() {
    let output = compile_and_run(
        r#"
> std::concurrency::thread::spawn

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
fn test_std_thread_spawn_owns_inline_copy_capture() {
    let output = compile_and_run(
        r#"
> std::concurrency::thread::spawn

main = ->
    value = 41
    match spawn (-> value + 1)
        Result::Ok handle =>
            match handle.join!
                Result::Ok result => result.println!
                Result::Err _ => "join failed".println!
        Result::Err _ => "spawn failed".println!
    value.println!
    0
"#,
    );
    assert_eq!(output.lines().collect::<Vec<_>>(), ["42", "41"]);
}

#[test]
fn test_std_thread_spawn_rejects_borrowed_capture_after_branch_merge() {
    compile_should_fail(
        r#"
> std::concurrency::thread::spawn

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
fn test_std_thread_spawn_rejects_borrowed_method_value() {
    compile_should_fail(
        r#"
> std::concurrency::thread::spawn

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
fn test_std_thread_spawn_rejects_downstream_send_impl() {
    compile_should_fail(
        r#"
> std::concurrency::thread::spawn

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
fn test_std_thread_spawn_accepts_struct_callable_impl() {
    let output = compile_and_run(
        r#"
> std::concurrency::thread::spawn

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
fn test_std_arc_mutex_synchronizes_threaded_updates() {
    let output = compile_and_run(
        r#"
> std::concurrency::Arc
> std::concurrency::Mutex
> std::concurrency::thread::spawn

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
fn test_std_mutex_guard_drop_unlocks_mutex() {
    let output = compile_and_run(
        r#"
> std::concurrency::Mutex

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
fn test_std_mutex_guard_is_not_send() {
    compile_should_fail(
        r#"
> std::concurrency::Mutex
> std::concurrency::thread::spawn

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
fn test_std_thread_spawn_rejects_reference_hidden_in_moved_struct() {
    compile_should_fail(
        r#"
> std::concurrency::thread::spawn

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
fn test_std_mutex_try_lock_observes_guard_lifetime() {
    let output = compile_and_run(
        r#"
> std::concurrency::Mutex

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
fn test_std_arc_drops_owned_value_once() {
    let (output, success) = compile_and_run_with_status(
        r#"
> std::concurrency::Arc

main = ->
    value = Arc::new (String::from "shared")
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

use crate::support::*;
use std::sync::atomic::Ordering;

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

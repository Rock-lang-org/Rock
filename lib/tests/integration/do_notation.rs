use crate::support::*;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::Ordering;

#[test]
fn test_multiline_chain_targets_completed_call() {
    let output = compile_and_run(
        r#"
make = value -> Option::Some value
add = left, right -> Option::Some (left + right)

main = !->
    make 42
        .unwrap_or 0
        .println!
    add 20, 22
        .map (+ 1)
        .unwrap_or 0
        .println!
    make (make 44
        .unwrap_or 0)
        .unwrap_or 0
        .println!
"#,
    );
    assert_eq!(output, "42\n43\n44\n");
}

#[test]
fn test_call_result_propagation_operator_continuations() {
    let output = compile_and_run(
        r#"
infix 1 %%
infix 12 **
%% = left, right -> left + right
** = left, right -> left + right

get: I64 -> Result I64, I64
get = value -> if value < 0 then Result::Err 99 else Result::Ok value

compute: () -> Result I64, I64
compute = ->
    first = get 20? + 1
    second = get 20? %% 1
    third = get 20? ** 1
    fourth = get 20? |> (+ 1)
    same = get 21? == 21
    same.println!
    converted = get 21? as I32
    converted.println!
    Result::Ok (first + second + third + fourth)

never = value ->
    999.println!
    value

fail: () -> Result I64, I64
fail = -> Result::Ok (get -1? |> never)

main = !->
    compute!.println!
    fail!.println!
"#,
    );
    assert_eq!(output, "true\n21\nOk(84)\nErr(99)\n");
}

#[test]
fn test_call_result_propagation_suffixes_and_grouped_arguments() {
    let output = compile_and_run(
        r#"
get: I64 -> Result I64, I64
get = value -> Result::Ok value
pair: I64 -> Result (Vec I64), I64
pair = value ->
    mut values = Vec::new!
    values.push value
    values.push value + 1
    Result::Ok values
nested: I64 -> Result (Result I64, I64), I64
nested = value -> Result::Ok (Result::Ok value)
adder: I64 -> Result (I64 -> I64), I64
adder = ignored -> Result::Ok (extra -> extra + 1)

compute: () -> Result I64, I64
compute = ->
    get 42?.println!
    pair 41?[1].println!
    nested 42??.println!
    applied = adder 0? 41
    applied.println!
    explicit = get (get 42?)?
    explicit.println!
    borrowed = [get 42?]
    borrowed[0].println!
    Result::Ok 0

main = !-> compute!.println!
"#,
    );
    assert_eq!(output, "42\n42\n42\n42\n42\n42\nOk(0)\n");
}

#[test]
fn test_nested_bind_transitive_capture() {
    let output = compile_and_run(
        r#"
compute: () -> Result I64, I64
compute = ->
    match true
        _ =>
            bind (Result::Ok 16), (count ->
                bind (Result::Ok 0), (_ ->
                    bind (Result::Ok 6), (appended ->
                        total = count + appended
                        pure total)))

main = !-> compute! .unwrap_or -1 .println!
"#,
    );
    assert_eq!(output, "22\n");
}

#[test]
fn test_match_local_shadow_restores_outer_binding() {
    let output = compile_and_run(
        r#"
main = !->
    outside = 5
    inner = match true
        _ =>
            outside: I64 = 9
            outside
    inner.println!
    outside.println!
"#,
    );
    assert_eq!(output, "9\n5\n");
}

#[test]
fn test_do_notation_result_option_and_scope() {
    let output = compile_and_run(
        r#"
success: I64 -> Result I64, I64
success = value -> Result::Ok value

compute: () -> Result I64, I64
compute = -> do
    count <- success 16
    appended <- success 6
    total = count + appended
    pure total

main = !->
    compute! .unwrap_or -1 .println!
    outside = 5
    nested = do
        outside: I64 = 9
        x <- Option::Some outside
        y <- do
            z <- Option::Some 2
            pure (z + 1)
        Option::Some 0
        _ <- Option::Some 1
        pure (x + y)
    nested.unwrap_or -1 .println!
    outside.println!
    singleton: Option I64 = do
        local = 42
        pure local
    singleton.unwrap_or -1 .println!
"#,
    );
    assert_eq!(output, "22\n12\n5\n42\n");
}

#[test]
fn test_do_notation_short_circuits_and_preserves_order() {
    let output = compile_and_run(
        r#"
step: I64 -> Result I64, I64
step = value ->
    value.println!
    if value == 2 then Result::Err 99 else Result::Ok value

run: () -> Result I64, I64
run = -> do
    first <- step 1
    step 2
    later <- step 3
    pure (first + later)

main = !->
    run! .unwrap_or -1 .println!
    absent: Option I64 = do
        x <- Option::None
        printed = 100.println!
        pure x
    absent.unwrap_or -2 .println!
"#,
    );
    assert_eq!(output, "1\n2\n-1\n-2\n");
}

#[test]
fn test_do_notation_rebinds_bind_and_supports_generic_helpers() {
    let output = compile_and_run(
        r#"
twice: F I64 -> F I64 where F _: Monad
twice = action -> do
    value <- action
    pure (value * 2)

main = !->
    twice (Option::Some 21) .unwrap_or 0 .println!
    bind: I64 -> (I64 -> I64) -> I64 = value, callback -> callback (value + 10)
    result = do
        x <- 2
        y <- 3
        x + y
    result.println!
"#,
    );
    assert_eq!(output, "42\n25\n");
}

#[test]
fn test_do_notation_without_std() {
    let status = compile_and_run_without_std(
        r#"
bind: I64 -> (I64 -> I64) -> I64
bind = value, callback -> callback value

main = -> do
    value <- 7
    value
"#,
    );
    assert_eq!(status, 7);
}

#[test]
fn test_do_notation_repeated_continuations() {
    let output = compile_and_run(
        r#"
repeat: I64 -> M -> I64 where M: FnMut I64, I64
repeat = value, mut callback ->
    first = callback value
    second = callback (value + 1)
    first + second

main = !->
    bind: I64 -> (I64 -> I64) -> I64 = repeat
    result = do
        x <- 1
        printed = x.println!
        x
    result.println!
"#,
    );
    assert_eq!(output, "1\n2\n3\n");
}

#[test]
fn test_do_notation_owned_payload_drops_on_success_and_failure() {
    let output = compile_and_run(
        r#"
struct Tracked
    < id: I64

impl Drop for Tracked
    ~@drop = !-> self.id.println!

run: I64 -> Option I64 -> Option I64
run = id, next -> do
    owned = Tracked
        id: id
    count <- next
    pure (owned.id + count)

main = !->
    run 10, (Option::Some 1) .unwrap_or -1 .println!
    run 20, Option::None .unwrap_or -1 .println!
    length = do
        text <- Option::Some (String::from "hello")
        pure text.len!
    length.unwrap_or 0 .println!
"#,
    );
    assert_eq!(output, "10\n11\n20\n-1\n5\n");
}

#[test]
fn test_do_notation_preserves_move_errors() {
    compile_should_fail(
        r#"
main = !->
    text = Option::Some (String::from "hello")
    length = do
        value <- text
        pure value.len!
    text.println!
"#,
        "borrow of moved value",
    );
}

#[test]
fn test_do_notation_exported_generic_artifact() {
    let dir = test_temp_dir(TEST_COUNTER.fetch_add(1, Ordering::SeqCst));
    fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    let library = dir.join("helpers.rk");
    fs::write(
        &library,
        r#"
increment: F I64 -> F I64 where F _: Monad
< increment = action -> do
    value <- action
    pure (value + 1)
"#,
    )
    .unwrap();
    let object = dir.join("helpers.o");
    let artifact = dir.join("helpers.rkca");
    let mut config = test_config(library, dir.clone());
    config.current_crate_name = Some("helpers".into());
    config.no_link = true;
    config.emit_object = Some(object);
    let mut products = rock_lib::compile_with_products(&config)
        .unwrap()
        .products
        .unwrap();
    products.link.object_path = Some(PathBuf::from("helpers.o"));
    products.write_artifact_to_path(&artifact).unwrap();

    let entry = dir.join("main.rk");
    fs::write(
        &entry,
        "> helpers::increment\nmain = -> increment (Option::Some 41) .unwrap_or 0\n",
    )
    .unwrap();
    let mut config = test_config(entry, dir.clone());
    config.extern_artifacts.push(("helpers".into(), artifact));
    rock_lib::compile(&config).unwrap();
    let output = run_test_command(&mut Command::new(dir.join("main")));
    assert_eq!(output.status.code(), Some(42));
}

#[test]
fn test_do_notation_does_not_reserve_bind_operator_globally() {
    let status = compile_and_run_without_std(
        r#"
infix 8 <-
<-: I64 -> I64 -> I64
<- = left, right -> ~I64Add left, right
main = -> 2 <- 3
"#,
    );
    assert_eq!(status, 5);
}

#[test]
fn test_do_notation_rejects_mixed_carriers() {
    compile_should_fail(
        r#"
main = !->
    result = do
        x <- Option::Some 1
        Result::Ok x
    result.println!
"#,
        "ambiguous constraint at quiescence: call to generic function 'bind'",
    );
}

#[test]
fn test_do_notation_macro_expansion() {
    let output = compile_and_run(
        r#"
macro make_main
    =>
        main = !->
            result = do
                x <- Option::Some 40
                pure (x + 2)
            result.unwrap_or 0 .println!

%make_main
"#,
    );
    assert_eq!(output, "42\n");
}

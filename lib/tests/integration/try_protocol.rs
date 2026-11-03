use crate::support::*;
use std::fs;
use std::sync::atomic::Ordering;

#[test]
fn test_std_try_protocol_methods() {
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
    let exit_code = compile_and_run_without_std(
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

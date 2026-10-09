//! Cross-phase acceptance for the remaining dynamic-dispatch contracts.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use rock_lib::diagnostic::DiagnosticCode;
use rock_lib::Config;

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join("opencode").join(format!(
            "rock_dynamic_completion_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn config(&self, entry: &Path, opt_level: u8) -> Config {
        Config {
            entry_file: entry.into(),
            output_dir: self.0.clone(),
            current_crate_name: Some("dynamic_completion".into()),
            no_std: true,
            no_prelude: true,
            opt_level,
            ..Default::default()
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// Renamed language-item providers ensure invocation is never name-special-cased.
const CALLBACK_PROVIDER: &str = r#"
lang fn_once
< trait Consume Args, Ret
    lang output
    type Output
    lang method
    ~@consume: Args -> Ret
lang fn_mut
< trait Change Args, Ret where Self: Consume Args, Ret
    lang output
    type Output
    lang method
    ^@change: Args -> Ret
lang fn
< trait Observe Args, Ret where Self: Change Args, Ret
    lang output
    type Output
    lang method
    @observe: Args -> Ret
< trait Callbacks
    @shared: &F -> T -> R where F: Observe T, R
    @once: F -> T -> R where F: Consume T, R
    @pair: &F -> T -> U -> R where F: Observe (T, U), R
relay_callback: &F -> T -> R where F: Observe T, R
relay_callback = callback, value -> callback.observe value
< struct Service
    < marker: I64
impl Callbacks for Service
    @shared = callback, value -> relay_callback callback, value
    @once = callback, value -> callback value
    @pair = callback, first, second -> callback.observe (first, second)
as_callbacks: &Service -> &Callbacks
< as_callbacks = service ->
    object: &Callbacks = service
    object
"#;

const SHARED_RESULT: &str = r#"
main = ->
    service = Service
        marker: 0
    object = as_callbacks &service
    value = 42
    callback = reference -> reference
    result = object.shared (&callback), &value
    *result
"#;

const NESTED_RESULT: &str = r#"
main = ->
    service = Service
        marker: 0
    object = as_callbacks &service
    first = 40
    second = 2
    callback = left, right -> (left, right)
    result = object.pair (&callback), &first, &second
    ~I64Add (*result.0), (*result.1)
"#;

const UNIQUE_RESULT: &str = r#"
main = ->
    service = Service
        marker: 0
    object = as_callbacks &service
    mut value = 0
    callback = reference -> reference
    result = object.once callback, &mut value
    *result = 42
    value
"#;

const CAPTURED_RESULT: &str = r#"
main = ->
    service = Service
        marker: 0
    object = as_callbacks &service
    value = 42
    reference = &value
    callback = _ -> reference
    result = object.once callback, ()
    *result
"#;

fn execute(config: &Config) {
    rock_lib::compile(config).unwrap();
    let output = Command::new(config.output_dir.join("main"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(42), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
}

fn execute_source(application: &str) {
    execute_program(&format!("{CALLBACK_PROVIDER}\n{application}"));
}

fn execute_program(source: &str) {
    let scratch = Scratch::new();
    let entry = scratch.0.join("main.rk");
    fs::write(&entry, source).unwrap();
    for opt_level in [0, 3] {
        execute(&scratch.config(&entry, opt_level));
    }
}

#[test]
fn nested_package_payload_positions_keep_the_same_hidden_self() {
    execute_program(
        r#"
trait Sum
    @combine: &Self -> I64
struct Number
    < value: I64
impl Sum for Number
    @combine = other -> ~I64Add self.value, other.value
type Nested = exists T where T: Sum in ((&T, &T), (&T, &T))
add = left, right -> ~I64Add left, right
sum_nested: Nested -> I64
sum_nested = package ->
    open package as Hidden, values
        left = values.0
        right = values.1
        first = left.0.combine right.0
        second = left.1.combine right.1
        add first, second
main = ->
    first = Number
        value: 20
    second = Number
        value: 1
    left = (&first, &second)
    right = (&first, &second)
    package = pack (Nested) ((left, right))
    sum_nested package
"#,
    );
}

#[test]
fn runtime_callback_returns_a_shared_input_borrow_through_a_generic_helper() {
    execute_source(SHARED_RESULT);
}

#[test]
fn runtime_callback_returns_nested_input_borrows() {
    execute_source(NESTED_RESULT);
}

#[test]
fn runtime_consuming_callback_returns_a_unique_input_borrow() {
    execute_source(UNIQUE_RESULT);
}

#[test]
fn runtime_consuming_callback_transports_an_existing_captured_borrow() {
    execute_source(CAPTURED_RESULT);
}

#[test]
fn runtime_callback_result_keeps_the_input_loan_live() {
    let scratch = Scratch::new();
    let entry = scratch.0.join("main.rk");
    fs::write(
        &entry,
        format!(
            r#"
{CALLBACK_PROVIDER}
main = ->
    service = Service
        marker: 0
    object = as_callbacks &service
    mut value = 42
    callback = reference -> reference
    result = object.shared (&callback), &value
    value = 0
    *result
"#
        ),
    )
    .unwrap();
    let diagnostics = rock_lib::compile(&scratch.config(&entry, 0)).unwrap_err();
    assert!(diagnostics.0.iter().any(|diagnostic| {
        diagnostic.code == Some(DiagnosticCode::Borrow)
            && diagnostic.primary.as_ref().is_some_and(|label| {
                label.span.file_path == entry && label.span.start < label.span.end
            })
    }), "{diagnostics:?}");
    assert!(diagnostics
        .0
        .iter()
        .all(|diagnostic| diagnostic.code != Some(DiagnosticCode::Internal)));
}

#[test]
fn source_free_callback_provider_preserves_borrow_origins_for_later_callbacks() {
    let scratch = Scratch::new();
    let producer = scratch.0.join("callbacks.rk");
    fs::write(&producer, CALLBACK_PROVIDER).unwrap();
    let object = scratch.0.join("callbacks.o");
    let producer_config = Config {
        current_crate_name: Some("callbacks".into()),
        no_link: true,
        emit_object: Some(object.clone()),
        ..scratch.config(&producer, 0)
    };
    let mut products = rock_lib::compile_with_products(&producer_config)
        .unwrap()
        .products
        .unwrap();
    products.link.object_path = Some(PathBuf::from("callbacks.o"));
    let artifact = scratch.0.join("callbacks.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let object_before = fs::read(&object).unwrap();
    let artifact_before = fs::read(&artifact).unwrap();
    fs::remove_file(producer).unwrap();

    let entry = scratch.0.join("main.rk");
    for application in [SHARED_RESULT, NESTED_RESULT, UNIQUE_RESULT, CAPTURED_RESULT] {
        fs::write(
            &entry,
            format!(
                "> callbacks::Service\n> callbacks::Callbacks\n> callbacks::as_callbacks\n{application}",
            ),
        )
        .unwrap();
        for opt_level in [0, 3] {
            let consumer = Config {
                extern_artifacts: vec![("callbacks".into(), artifact.clone())],
                ..scratch.config(&entry, opt_level)
            };
            execute(&consumer);
            assert_eq!(fs::read(&object).unwrap(), object_before);
            assert_eq!(fs::read(&artifact).unwrap(), artifact_before);
        }
    }
}

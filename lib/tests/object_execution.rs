use std::sync::atomic::{AtomicUsize, Ordering};

use rock_lib::{Config, SourceProvider};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn configuration(source: &str) -> Config {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!("rock_object_execution_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&directory).unwrap();
    let entry = directory.join("main.rk");
    Config {
        entry_file: entry.clone(), output_dir: directory,
        current_crate_name: Some("object_execution".to_string()),
        no_std: true, no_prelude: true,
        source_providers: vec![SourceProvider::Virtual { path: entry, text: source.to_string() }],
        ..Config::default()
    }
}

fn run(source: &str) -> i32 {
    let config = configuration(source);
    rock_lib::compile(&config).unwrap();
    let result = std::process::Command::new(config.output_dir.join("main")).output().unwrap();
    assert!(result.stderr.is_empty(), "{}", String::from_utf8_lossy(&result.stderr));
    let code = result.status.code().expect("normal process exit");
    std::fs::remove_dir_all(&config.output_dir).unwrap();
    code
}

#[test]
fn source_objects_dispatch_two_implementations_and_a_default() {
    assert_eq!(run(r#"
trait Value
    @value: I64
    @next: I64
    @next = -> ~I64Add (@value!), 1
struct First
    < inner: I64
struct Second
    < inner: I64
impl Value for First
    @value = -> self.inner
impl Value for Second
    @value = -> self.inner
read: &Value -> I64
read = object -> object.next!
main = ->
    first = First
        inner: 19
    second = Second
        inner: 21
    ~I64Add (read &first), (read &second)
"#), 42);
}

#[test]
fn source_mutable_object_updates_the_original_payload() {
    assert_eq!(run(r#"
trait Counter
    ^@bump: ()
    @read: I64
struct Count
    < inner: I64
impl Counter for Count
    ^@bump = !-> self.inner = ~I64Add self.inner, 1
    @read = -> self.inner
main = ->
    mut count = Count
        inner: 40
    mut object: &mut Counter = &mut count
    object.bump!
    object.bump!
    object.read!
"#), 42);
}

#[test]
fn source_borrowed_self_result_preserves_an_upcastable_view() {
    assert_eq!(run(r#"
trait Base
    @read: I64
trait Derived where Self: Base
    @view: &Self
struct Number
    < inner: I64
impl Base for Number
    @read = -> self.inner
impl Derived for Number
    @view = -> self
main = ->
    number = Number
        inner: 42
    derived: &Derived = &number
    another = derived.view!
    base: &Base = another
    base.read!
"#), 42);
}

#[test]
fn source_object_cannot_hide_an_escaping_stack_borrow() {
    let config = configuration(r#"
trait Value
    @value: I64
struct Number
    < inner: I64
impl Value for Number
    @value = -> self.inner
escape: () -> &Value
escape = ->
    number = Number
        inner: 42
    object: &Value = &number
    object
main = ->
    object = escape!
    object.value!
"#);
    let diagnostics = rock_lib::compile(&config).expect_err("an object must preserve its payload borrow");
    assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.message.contains("reference") || diagnostic.message.contains("borrow")), "{diagnostics:?}");
    std::fs::remove_dir_all(config.output_dir).unwrap();
}

#[test]
fn source_forward_inferred_coercion_reaches_virtual_execution() {
    assert_eq!(run(r#"
trait Value
    @value: I64
struct Number
    < inner: I64
impl Value for Number
    @value = -> self.inner
main = ->
    number = make!
    object: &Value = borrow &number
    object.value!
make = -> Number
    inner: 42
borrow = value -> value
"#), 42);
}

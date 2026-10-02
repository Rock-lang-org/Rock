use std::sync::atomic::{AtomicUsize, Ordering};

use rock_lib::{Config, SourceProvider};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn config(source: &str) -> Config {
    let directory = std::env::temp_dir().join(format!(
        "rock_owner_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let entry = directory.join("main.rk");
    Config {
        entry_file: entry.clone(),
        output_dir: directory,
        no_std: true,
        no_prelude: true,
        current_crate_name: Some("owner_test".into()),
        source_providers: vec![SourceProvider::Virtual {
            path: entry,
            text: source.into(),
        }],
        ..Config::default()
    }
}

const PROTOCOL: &str = r#"
lang sized
< trait LayoutKnown
lang object_owner
< trait Transport State for F _
    lang owner_into_parts
    unsafe split: F T -> (*T, State) where T: ?LayoutKnown
    lang owner_from_parts
    unsafe rebuild: *T -> State -> F T where T: ?LayoutKnown
"#;

#[test]
fn renamed_sized_relaxation_allows_only_indirected_storage() {
    let source = format!("{PROTOCOL}\nstruct Handle (T: ?LayoutKnown)\n    < pointer: *T\nignore: &T -> () where T: ?LayoutKnown\nignore = value !-> ()\nmain = !-> ()\n");
    rock_lib::analyze(&config(&source)).unwrap();
    let invalid = format!("{PROTOCOL}\nconsume: T -> () where T: ?LayoutKnown\nconsume = value !-> ()\nmain = !-> ()\n");
    let diagnostics = rock_lib::analyze(&config(&invalid)).unwrap_err();
    assert!(diagnostics
        .0
        .iter()
        .any(|diagnostic| diagnostic.message.contains("by-value storage")));
}

#[test]
fn relaxed_bound_requires_the_canonical_language_item() {
    let source =
        "trait Sized\nignore: &T -> () where T: ?Sized\nignore = value !-> ()\nmain = !-> ()\n";
    assert!(rock_lib::analyze(&config(source))
        .unwrap_err()
        .0
        .iter()
        .any(|diagnostic| diagnostic.message.contains("canonical Sized")));
}

#[test]
fn owner_protocol_requires_unsafe_operations() {
    let source = PROTOCOL.replace("unsafe split:", "split:") + "\nmain = !-> ()\n";
    assert!(rock_lib::analyze(&config(&source))
        .unwrap_err()
        .0
        .iter()
        .any(|diagnostic| diagnostic.message.contains("must be unsafe")));
}

#[test]
fn custom_owner_coercion_uses_constructor_protocol_and_preserves_state() {
    let source = format!(
        "{PROTOCOL}{}",
        r#"
trait Value
    @value: I64
struct Number
    < inner: I64
impl Value for Number
    @value = -> self.inner
struct Shell (T: ?LayoutKnown)
    < pointer: *T
    < tag: I64
impl Transport I64 for Shell
    unsafe split = owner ->
        parts = (owner.pointer, owner.tag)
        ~Forget owner
        parts
    unsafe rebuild = pointer, tag -> Shell
        pointer: pointer
        tag: tag
erase: Shell Number -> Shell Value
erase = owner -> owner
main = !-> ()
"#
    );
    let analysis = rock_lib::analyze(&config(&source)).unwrap();
    let erase = analysis
        .hir
        .program
        .functions
        .values()
        .find(|function| function.name == "erase")
        .unwrap();
    assert!(matches!(
        erase.body.stmts.last(),
        Some(rock_lib::hir::HirStmtFor::Expr(rock_lib::hir::HirExprFor {
            kind: rock_lib::hir::HirExprKindFor::Block(_),
            ..
        }))
    ));
}

#[test]
fn default_nominal_parameter_rejects_an_unsized_argument() {
    let source = "trait Value\nstruct Handle T\n    < pointer: *T\nignore: &(Handle Value) -> ()\nignore = value !-> ()\nmain = !-> ()\n";
    assert!(rock_lib::analyze(&config(source)).is_err());
}

fn stdlib_artifact() -> std::path::PathBuf {
    static ARTIFACT: std::sync::OnceLock<
        Result<std::path::PathBuf, rock_lib::diagnostic::Diagnostics>,
    > = std::sync::OnceLock::new();
    ARTIFACT
        .get_or_init(|| {
            let directory =
                std::env::temp_dir().join(format!("rock_owner_stdlib_{}", std::process::id()));
            std::fs::create_dir_all(&directory).unwrap();
            let output = rock_lib::compile_with_products(&Config {
                entry_file: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .unwrap()
                    .join("stdlib/lib.rk"),
                output_dir: directory.clone(),
                current_crate_name: Some("stdlib".into()),
                no_std: true,
                no_link: true,
                emit_object: Some(directory.join("stdlib.o")),
                ..Config::default()
            })?;
            let mut products = output.products.unwrap();
            products.link.object_path = Some("stdlib.o".into());
            let path = directory.join("stdlib.rkca");
            products.write_artifact_to_path(&path).unwrap();
            Ok(path)
        })
        .as_ref()
        .expect("stdlib must compile before owner verification")
        .clone()
}

fn run_with_stdlib(source: &str) -> i32 {
    let mut config = config(source);
    config
        .extern_artifacts
        .push(("stdlib".into(), stdlib_artifact()));
    rock_lib::compile(&config).unwrap();
    let result = std::process::Command::new(config.output_dir.join("main"))
        .output()
        .unwrap();
    assert!(
        result.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let code = result.status.code().unwrap();
    std::fs::remove_dir_all(config.output_dir).unwrap();
    code
}

#[test]
fn box_object_moves_payload_ownership_and_drops_once() {
    assert_eq!(
        run_with_stdlib(
            r#"
> stdlib::box_type::Box
> stdlib::drop::Drop
trait Value
    @value: I64
struct Payload
    < drops: *I64
impl Value for Payload
    @value = -> 42
impl Drop for Payload
    ~@drop = !->
        unsafe
            *self.drops = ~I64Add (*self.drops), 1
exercise: *I64 -> ()
exercise = drops !->
    payload = Payload
        drops: drops
    concrete = Box::new payload
    object: Box Value = concrete
    object.as_ref!.value!
main = ->
    mut drops: I64 = 0
    exercise ((&mut drops) as *I64)
    if ~I64Eq drops, 1 then 42 else 1
"#
        ),
        42
    );
}

#[test]
fn arc_object_clone_and_upcast_keep_one_payload_destruction() {
    assert_eq!(
        run_with_stdlib(
            r#"
> stdlib::arc::Arc
> stdlib::clone::Clone
> stdlib::drop::Drop
trait Base
    @value: I64
trait Derived where Self: Base
struct Payload
    < drops: *I64
impl Base for Payload
    @value = -> 42
impl Derived for Payload
impl Drop for Payload
    ~@drop = !->
        unsafe
            *self.drops = ~I64Add (*self.drops), 1
exercise: *I64 -> ()
exercise = drops !->
    payload = Payload
        drops: drops
    concrete = Arc::new payload
    derived: Arc Derived = concrete
    duplicate = derived.clone!
    base: Arc Base = derived
    base.value!
    duplicate.value!
main = ->
    mut drops: I64 = 0
    exercise ((&mut drops) as *I64)
    if ~I64Eq drops, 1 then 42 else 1
"#
        ),
        42
    );
}

#[test]
fn aligned_zero_sized_allocation_is_nonnull_and_releasable() {
    assert_eq!(
        run_with_stdlib(
            r#"
> stdlib::alloc::Layout
> stdlib::alloc::Global
> stdlib::alloc::checked_alloc
main = ->
    layout = Layout
        size: 0
        align: 64
    pointer = checked_alloc layout
    aligned = ~I64Eq (~I64And (pointer as I64), 63), 0
    unsafe Global::dealloc pointer, layout
    if aligned then 42 else 1
"#
        ),
        42
    );
}

#[test]
fn box_consuming_virtual_method_drops_payload_once() {
    assert_eq!(
        run_with_stdlib(
            r#"
> stdlib::box_type::Box
> stdlib::drop::Drop
trait Take
    ~@take: I64
struct Payload
    < drops: *I64
impl Take for Payload
    ~@take = -> 42
impl Drop for Payload
    ~@drop = !->
        unsafe
            *self.drops = ~I64Add (*self.drops), 1
main = ->
    mut drops: I64 = 0
    payload = Payload
        drops: (&mut drops) as *I64
    owner: Box Take = Box::new payload
    answer = owner.take!
    if ~I64Eq drops, 1 then answer else 1
"#
        ),
        42
    );
}

#[test]
fn shared_arc_does_not_gain_consuming_payload_access() {
    let mut config = config(
        r#"
> stdlib::arc::Arc
trait Take
    ~@take: I64
bad: Arc Take -> I64
bad = owner -> owner.take!
main = !-> ()
"#,
    );
    config
        .extern_artifacts
        .push(("stdlib".into(), stdlib_artifact()));
    let diagnostics = rock_lib::analyze(&config).unwrap_err();
    assert!(
        diagnostics
            .0
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unique release")
                || diagnostic.message.contains("unique owner")),
        "{diagnostics:?}"
    );
}

#[test]
fn owned_object_with_explicit_send_guarantee_can_be_consumed_on_a_thread() {
    assert_eq!(
        run_with_stdlib(
            r#"
> stdlib::box_type::Box
> stdlib::result::Result
> stdlib::thread::spawn
> stdlib::thread_safety::Send
trait Take
    ~@take: I64
struct Number
    < value: I64
impl Take for Number
    ~@take = -> self.value
main = ->
    number = Number
        value: 42
    owner: Box (Take { Send }) = Box::new number
    match spawn (-> owner.take!)
        Result::Ok handle =>
            match handle.join!
                Result::Ok answer => answer
                Result::Err _ => 2
        Result::Err _ => 1
"#,
        ),
        42
    );
}

#[test]
fn owned_object_does_not_recover_forgotten_send_guarantees_at_spawn() {
    let mut config = config(
        r#"
> stdlib::box_type::Box
> stdlib::thread::spawn
trait Take
    ~@take: I64
struct Number
    < value: I64
impl Take for Number
    ~@take = -> self.value
main = !->
    number = Number
        value: 42
    owner: Box Take = Box::new number
    spawn (-> owner.take!)
"#,
    );
    config
        .extern_artifacts
        .push(("stdlib".into(), stdlib_artifact()));
    let diagnostics = rock_lib::analyze(&config)
        .expect_err("erasure must retain only the declared marker guarantees");
    assert!(
        diagnostics.0.iter().any(|diagnostic| {
            diagnostic.message.contains("does not implement trait")
                && diagnostic.message.contains("Send")
                && diagnostic.primary.is_some()
        }),
        "{diagnostics:?}"
    );
}

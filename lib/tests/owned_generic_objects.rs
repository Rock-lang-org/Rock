use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use rock_lib::Config;

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch() -> Scratch {
    let root = std::env::temp_dir().join(format!(
        "rock_owned_generic_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    Scratch(root)
}

const OWNER: &str = r#"
lang sized
< trait KnownSize
lang drop
< trait Destroy
    lang method
    ~@destroy: ()
lang object_owner
< trait Transport State for F _
    lang owner_into_parts
    unsafe split: F T -> (*T, State) where T: ?KnownSize
    lang owner_from_parts
    unsafe rebuild: *T -> State -> F T where T: ?KnownSize
lang object_owner_unique
< trait Unique State for F _
    lang owner_release
    unsafe release: *U8 -> State -> ()
extern malloc: I64 -> *U8
extern free: *U8 -> ()
< struct State
    < base: *U8
    < releases: *I64
< struct Own (T: ?KnownSize)
    raw: *T
    base: *U8
    releases: *I64
impl Own T
    new: T -> *I64 -> Own T
    new = value, releases ->
        base = unsafe malloc (~SizeOf value)
        pointer: *T = base as *T
        unsafe *pointer = value
        Own T
            raw: pointer
            base: base
            releases: releases
impl Own T where T: ?KnownSize
    @borrow: &T
    @borrow = -> unsafe &*self.raw
impl Transport State for Own
    unsafe split = owner ->
        state = State
            base: owner.base
            releases: owner.releases
        parts = (owner.raw, state)
        ~Forget owner
        parts
    unsafe rebuild = pointer, state -> Own
        raw: pointer
        base: state.base
        releases: state.releases
impl Unique State for Own
    unsafe release = _, state !->
        *state.releases = ~I64Add (*state.releases), 1
        free state.base
impl Destroy for Own T where T: ?KnownSize
    ~@destroy = !->
        unsafe
            ~DropInPlace self.raw
            *self.releases = ~I64Add (*self.releases), 1
            free self.base
"#;

const SERVICE: &str = r#"
< trait Read
    @read: I64
< trait Take
    ~@identity: T -> T
    ~@read: T -> I64 where T: Read
    @peek: I64
struct Payload
    < answer: I64
    < drops: *I64
impl Take for Payload
    ~@identity = value -> value
    ~@read = value -> value.read!
    @peek = -> self.answer
impl Destroy for Payload
    ~@destroy = !->
        unsafe *self.drops = ~I64Add (*self.drops), 1
make: *I64 -> *I64 -> Own Take
< make = drops, releases ->
    payload = Payload
        answer: 42
        drops: drops
    owner: Own Take = Own::new payload, releases
    owner
"#;

const LATER: &str = r#"
struct Later
    < value: I64
    < drops: *I64
impl Read for Later
    @read = -> self.value
impl Destroy for Later
    ~@destroy = !->
        unsafe *self.drops = ~I64Add (*self.drops), 1
"#;

fn application(method: &str) -> String {
    let result = if method == "identity" {
        "output.value"
    } else {
        "output"
    };
    format!(
        r#"
{LATER}
run: Own Take -> Later -> I64
run = owner, input ->
    output = owner.{method} input
    {result}
main = ->
    mut payload_drops = 0
    mut releases = 0
    mut value_drops = 0
    owner = make ((&mut payload_drops) as *I64), ((&mut releases) as *I64)
    input = Later
        value: 42
        drops: (&mut value_drops) as *I64
    result = run owner, input
    if ~I64Ne payload_drops, 1 then return 1
    if ~I64Ne releases, 1 then return 2
    if ~I64Ne value_drops, 1 then return 3
    result
"#
    )
}

fn config(root: &Path, entry: PathBuf, dependencies: Vec<(String, PathBuf)>) -> Config {
    Config {
        entry_file: entry,
        output_dir: root.to_path_buf(),
        current_crate_name: Some("owned_generic".into()),
        no_std: true,
        no_prelude: true,
        extern_artifacts: dependencies,
        ..Default::default()
    }
}

fn execute(config: &Config) {
    rock_lib::compile(config).unwrap();
    let output = std::process::Command::new(config.output_dir.join("main"))
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(42),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn consuming_generic_identity_returns_noncopy_storage_and_drops_payload_once() {
    let root = scratch();
    let entry = root.0.join("main.rk");
    std::fs::write(
        &entry,
        format!("{OWNER}{SERVICE}{}", application("identity")),
    )
    .unwrap();
    execute(&config(&root.0, entry, vec![]));
}

#[test]
fn consuming_bounded_generic_method_uses_exact_dictionary_and_release() {
    let root = scratch();
    let entry = root.0.join("main.rk");
    std::fs::write(&entry, format!("{OWNER}{SERVICE}{}", application("read"))).unwrap();
    execute(&config(&root.0, entry, vec![]));
}

#[test]
fn consuming_generic_entries_accept_later_types_without_rebuilding_producer() {
    let root = scratch();
    let producer = root.0.join("provider.rk");
    std::fs::write(&producer, format!("{OWNER}{SERVICE}")).unwrap();
    let object = root.0.join("provider.o");
    let producer_config = Config {
        no_link: true,
        emit_object: Some(object.clone()),
        current_crate_name: Some("provider".into()),
        ..config(&root.0, producer.clone(), vec![])
    };
    let mut products = rock_lib::compile_with_products(&producer_config)
        .unwrap()
        .products
        .unwrap();
    products.link.object_path = Some(PathBuf::from("provider.o"));
    let artifact = root.0.join("provider.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let object_before = std::fs::read(&object).unwrap();
    let artifact_before = std::fs::read(&artifact).unwrap();
    std::fs::remove_file(producer).unwrap();
    let entry = root.0.join("main.rk");
    for method in ["identity", "read"] {
        std::fs::write(&entry, format!("> provider::Own\n> provider::Take\n> provider::Read\n> provider::Destroy\n> provider::make\n{}", application(method))).unwrap();
        execute(&config(
            &root.0,
            entry.clone(),
            vec![("provider".into(), artifact.clone())],
        ));
        assert_eq!(std::fs::read(&object).unwrap(), object_before);
        assert_eq!(std::fs::read(&artifact).unwrap(), artifact_before);
    }
}

#[test]
fn consuming_generic_call_rejects_a_live_view_or_reused_owner() {
    for body in [
        "view = owner.borrow!\n    owner.identity 1\n    view.peek!",
        "first = owner.identity 1\n    owner.identity first",
    ] {
        let root = scratch();
        let entry = root.0.join("main.rk");
        std::fs::write(
            &entry,
            format!(
                r#"
{OWNER}{SERVICE}
main = ->
    mut drops = 0
    mut releases = 0
    owner = make ((&mut drops) as *I64), ((&mut releases) as *I64)
    {body}
"#
            ),
        )
        .unwrap();
        let diagnostics = rock_lib::compile(&config(&root.0, entry, vec![])).unwrap_err();
        assert!(
            diagnostics
                .0
                .iter()
                .any(|diagnostic| diagnostic.message.contains("borrow")
                    || diagnostic.message.contains("move")),
            "{diagnostics:?}"
        );
    }
}

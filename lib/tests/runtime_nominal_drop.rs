//! Selected generic destructors must run before descriptor-backed field cleanup.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use rock_lib::Config;

const PROVIDER: &str = r#"
lang drop
< trait Dispose
    lang method
    ~@dispose: ()
extern abort: () -> ()
record: *I64 -> I64 -> ()
record = pointer, tag !->
    unsafe *pointer = ~I64Add (~I64Mul (*pointer), 10), tag
struct Frame T
    < value: T
    < events: *I64
impl Dispose for Frame T
    ~@dispose = !-> record self.events, 1
< trait Service
    @run: T -> *I64 -> ()
< struct Runner
    < tag: I64
impl Service for Runner
    @run = value, events !->
        frame = Frame
            value: value
            events: events
as_service: &Runner -> &Service
< as_service = runner ->
    object: &Service = runner
    object
"#;
const CLIENT: &str = r#"
struct Later
    < events: *I64
impl Dispose for Later
    ~@dispose = !->
        unsafe *self.events = ~I64Add (~I64Mul (*self.events), 10), 2
main = ->
    mut events = 0
    pointer = (&mut events) as *I64
    runner = Runner
        tag: 0
    object = as_service &runner
    object.run (Later
        events: pointer), pointer
    if ~I64Eq events, 12 then 42 else 1
"#;

static NEXT: AtomicUsize = AtomicUsize::new(0);
fn scratch() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
        .join(".agent-work/fixtures")
        .join(format!("nominal_drop_{}_{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    fs::create_dir_all(&root).unwrap();
    root
}
fn config(root: &Path, entry: &Path, level: u8) -> Config {
    Config {
        entry_file: entry.into(), output_dir: root.into(), opt_level: level,
        no_std: true, no_prelude: true, emit_llvm: true,
        ..Default::default()
    }
}
fn run(config: &Config) {
    rock_lib::compile(config).unwrap();
    assert_eq!(Command::new(config.output_dir.join("main")).status().unwrap().code(), Some(42));
}

#[test]
fn generic_nominal_destructor_precedes_field_drop_at_o0_o3() {
    let root = scratch();
    let entry = root.join("main.rk");
    fs::write(&entry, format!("{PROVIDER}{CLIENT}")).unwrap();
    for level in [0, 3] { run(&config(&root, &entry, level)); }
}

#[test]
fn unused_method_dictionary_bound_cannot_be_erased_from_nominal_drop() {
    let root = scratch();
    let entry = root.join("main.rk");
    let provider = PROVIDER.replace(
        "impl Dispose for Frame T\n    ~@dispose = !-> record self.events, 1",
        "trait Gate\n    @check: ()\nimpl Dispose for Frame T\n    ~@dispose: () where T: Gate\n    ~@dispose = !-> record self.events, 1",
    );
    let client = CLIENT.replace("main = ->", "impl Gate for Later\n    @check = !-> ()\nmain = ->");
    fs::write(&entry, format!("{provider}{client}")).unwrap();
    let diagnostics = rock_lib::compile(&config(&root, &entry, 0)).unwrap_err();
    assert!(diagnostics.0.iter().any(|diagnostic| {
        diagnostic.code == Some(rock_lib::diagnostic::DiagnosticCode::Mono)
            && diagnostic.message.contains("nominal Drop typed predicates are not admitted")
    }), "{diagnostics:?}");
}

#[test]
fn native_and_runtime_nominal_drop_authority_coalesce_at_o0_o3() {
    let root = scratch();
    let entry = root.join("main.rk");
    let client = CLIENT.replace("    if ~I64Eq events, 12", r#"    frame = Frame
        value: Later
            events: pointer
        events: pointer
    object.run frame, pointer
    if ~I64Eq events, 12112"#);
    fs::write(&entry, format!("{PROVIDER}{client}")).unwrap();
    for level in [0, 3] { run(&config(&root, &entry, level)); }
}

#[test]
fn generic_nominal_destructor_drops_its_raw_payload_at_o0_o3() {
    let root = scratch();
    let entry = root.join("main.rk");
    let provider = PROVIDER.replace("< value: T", "< value: *T")
        .replace("@run: T ->", "@run: *T ->")
        .replace("~@dispose = !-> record self.events, 1", "~@dispose = !->\n        record self.events, 1\n        unsafe dispose_at self.value");
    let helper = "unsafe dispose_at: *T -> ()\nunsafe dispose_at = pointer !-> unsafe ~DropInPlace pointer\n";
    let client = CLIENT.replace("    object.run (Later\n        events: pointer), pointer", r#"    raw = malloc 8
    payload = raw as *Later
    unsafe *payload = Later
        events: pointer
    object.run payload, pointer
    free raw"#);
    fs::write(&entry, format!("extern malloc: I64 -> *U8\nextern free: *U8 -> ()\n{helper}{provider}{client}")).unwrap();
    for level in [0, 3] { run(&config(&root, &entry, level)); }
}

#[test]
fn selected_owner_destructor_releases_after_payload_drop_at_o0_o3() {
    let root = scratch();
    let entry = root.join("main.rk");
    let provider = PROVIDER.replace("< value: T", "< value: *T\n    < base: *U8")
        .replace("@run: T -> *I64 -> ()", "@run: *T -> *I64 -> *U8 -> ()")
        .replace("@run = value, events !->", "@run = value, events, base !->")
        .replace("            value: value", "            value: value\n            base: base")
        .replace("~@dispose = !-> record self.events, 1", "~@dispose = !->\n        record self.events, 1\n        unsafe dispose_at self.value\n        Releaser::release (self.base, self.events)");
    let helpers = r#"
extern malloc: I64 -> *U8
extern free: *U8 -> ()
unsafe dispose_at: *T -> ()
unsafe dispose_at = pointer !-> unsafe ~DropInPlace pointer
struct Releaser
impl Releaser
    release: (*U8, *I64) -> ()
    release = parts !->
        free parts.0
        record parts.1, 3
"#;
    let client = CLIENT.replace("    object.run (Later\n        events: pointer), pointer", r#"    raw = malloc 8
    payload = raw as *Later
    unsafe *payload = Later
        events: pointer
    object.run payload, pointer, raw"#).replace("events, 12", "events, 123");
    fs::write(&entry, format!("{helpers}{provider}{client}")).unwrap();
    for level in [0, 3] { run(&config(&root, &entry, level)); }
}

#[test]
fn selected_destructor_early_return_keeps_structural_cleanup_at_o0_o3() {
    let root = scratch();
    let entry = root.join("main.rk");
    let provider = PROVIDER.replace("< events: *I64", "< events: *I64\n    < stop: Bool")
        .replace("            events: events", "            events: events\n            stop: true")
        .replace("~@dispose = !-> record self.events, 1", "~@dispose = !->\n        record self.events, 1\n        if self.stop then return ()\n        record self.events, 9");
    fs::write(&entry, format!("{provider}{CLIENT}")).unwrap();
    for level in [0, 3] { run(&config(&root, &entry, level)); }
}

#[test]
fn source_free_generic_nominal_drop_accepts_later_layouts_at_o0_o3() {
    let root = scratch();
    let provider = root.join("provider.rk");
    let object = root.join("provider.o");
    let artifact = root.join("provider.rkca");
    fs::write(&provider, PROVIDER).unwrap();
    let producer = Config {
        current_crate_name: Some("nominal_drop".into()), no_link: true,
        emit_object: Some(object.clone()), ..config(&root, &provider, 0)
    };
    let mut products = rock_lib::compile_with_products(&producer).unwrap().products.unwrap();
    products.link.object_path = Some("provider.o".into());
    products.write_artifact_to_path(&artifact).unwrap();
    let bytes = fs::read(&artifact).unwrap();
    let object_bytes = fs::read(&object).unwrap();
    fs::remove_file(provider).unwrap();
    let entry = root.join("main.rk");
    for wide in [false, true] {
        let client = if wide {
            CLIENT.replace("struct Later\n    < events: *I64", "struct Later\n    < padding: (I64, I64)\n    < events: *I64")
                .replace("object.run (Later\n        events: pointer)", "object.run (Later\n        padding: (3, 4)\n        events: pointer)")
        } else { CLIENT.into() };
        fs::write(&entry, format!("> nominal_drop::Dispose\n> nominal_drop::Service\n> nominal_drop::Runner\n> nominal_drop::as_service\n{client}")).unwrap();
        for level in [0, 3] {
            run(&Config {
                extern_artifacts: vec![("nominal_drop".into(), artifact.clone())],
                ..config(&root, &entry, level)
            });
            assert_eq!(fs::read(&artifact).unwrap(), bytes);
            assert_eq!(fs::read(&object).unwrap(), object_bytes);
        }
    }
}

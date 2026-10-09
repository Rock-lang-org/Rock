//! Read-only public evidence produces a selected blanket RelayRead dictionary.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use rock_lib::products::{ProductRuntimeSupport, RuntimeRootSymbol};
use rock_lib::Config;

const PROVIDER: &str = r#"
lang drop
< trait Dispose
    lang method
    ~@dispose: ()
< trait Read
    @read: I64
trait RelayRead
    @relay_read: I64
impl RelayRead for T where T: Read
    @relay_read = -> self.read!
relay_helper: U -> I64 where U: RelayRead
relay_helper = value -> value.relay_read!
< trait Reader
    @apply: T -> I64 where T: Read
< struct Service
    < marker: I64
impl Reader for Service
    @apply = value -> relay_helper value
as_reader: &Service -> &Reader
< as_reader = service ->
    object: &Reader = service
    object
"#;

const CLIENT: &str = r#"
struct Later
    < value: I64
    < reads: *I64
    < drops: *I64
impl Read for Later
    @read = ->
        unsafe *self.reads = ~I64Add (*self.reads), 1
        self.value
impl Dispose for Later
    ~@dispose = !->
        unsafe *self.drops = ~I64Add (*self.drops), 1
main = ->
    mut drops = 0
    mut reads = 0
    service = Service
        marker: 0
    object = as_reader &service
    drops_pointer = (&mut drops) as *I64
    reads_pointer = (&mut reads) as *I64
    first = Later
        value: 42
        reads: reads_pointer
        drops: drops_pointer
    second = Later
        value: 42
        reads: reads_pointer
        drops: drops_pointer
    first_result = object.apply first
    second_result = object.apply second
    if ~I64Ne first_result, 42 then 1
    else if ~I64Ne second_result, 42 then 2
    else if ~I64Ne reads, 2 then 3
    else if ~I64Ne drops, 2 then 4
    else 42
"#;

static NEXT: AtomicUsize = AtomicUsize::new(0);
fn scratch() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.agent-work/fixtures").join(format!("rock-dictionary-production-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    fs::create_dir_all(&root).unwrap(); root
}
fn config(root: &Path, entry: &Path, opt_level: u8) -> Config {
    Config { entry_file: entry.into(), output_dir: root.into(), current_crate_name: Some("dictionary_factory".into()),
        no_std: true, no_prelude: true, emit_llvm: true, opt_level, ..Default::default() }
}
fn client(wide: bool) -> String {
    if !wide { return CLIENT.into(); }
    CLIENT.replace("struct Later\n    < value: I64", "struct Later\n    < leading: (I64, I64)\n    < value: I64")
        .replace("    < drops: *I64\n", "    < drops: *I64\n    < trailing: I64\n")
        .replace("first = Later\n        value: 42", "first = Later\n        leading: (1, 2)\n        trailing: 3\n        value: 42")
        .replace("second = Later\n        value: 42", "second = Later\n        leading: (4, 5)\n        trailing: 6\n        value: 42")
}
fn run(config: &Config) {
    rock_lib::compile(config).unwrap();
    let output = Command::new(config.output_dir.join("main")).output().unwrap();
    assert_eq!(output.status.code(), Some(42), "actual Read delegation, two calls and exactly two non-Copy drops: {output:?}");
}

#[test]
fn selected_blanket_read_delegates_and_cleans_up_two_later_layouts_at_o0_o3() {
    let root = scratch(); let entry = root.join("main.rk");
    for wide in [false, true] {
        fs::write(&entry, format!("{PROVIDER}{}", client(wide))).unwrap();
        for optimization in [0, 3] { run(&config(&root, &entry, optimization)); }
    }
}

#[test]
fn source_free_selected_dictionary_uses_only_read_evidence_and_unchanged_producer_bytes() {
    let root = scratch(); let provider = root.join("provider.rk"); fs::write(&provider, PROVIDER).unwrap();
    let object = root.join("provider.o");
    let producer = Config { no_link: true, emit_object: Some(object.clone()), ..config(&root, &provider, 0) };
    let mut products = rock_lib::compile_with_products(&producer).unwrap().products.unwrap();
    assert!(products.link.runtime_support.iter().any(|support| matches!(support,
        ProductRuntimeSupport::SelectedDictionariesV1 { roots, .. } if roots == &std::collections::BTreeSet::from([RuntimeRootSymbol::SelectedDictionaryFactoryV1]))));
    let read = products.interface.object_abi.schemas.iter().flat_map(|schema| &schema.erased_slots)
        .flat_map(|slot| &slot.signature.dictionaries).map(|schema| schema.trait_id).collect::<std::collections::BTreeSet<_>>();
    assert_eq!(read.len(), 1, "only the Read trait is supplied by the caller");
    assert!(products.interface.object_abi.schemas.iter().flat_map(|schema| &schema.erased_slots).all(|slot| slot.signature.dictionaries.len() == 1));
    let ir = fs::read_to_string(root.join("provider.ll")).unwrap();
    let bodies = ir.split("define ").skip(1).map(|body| body.split_once("\n}").unwrap().0).collect::<Vec<_>>();
    let producer_body = bodies.iter().find(|body| body.starts_with("internal void @rock.erased.") && body.contains("dictionary.factory.status")).unwrap();
    assert_eq!(producer_body.lines().next().unwrap().split_once('(').unwrap().1.split_once(')').unwrap().0.split(',').count(), 6,
        "fixed out + Self/T descriptors + Read dictionary + receiver/value; no RelayRead public evidence");
    assert!(producer_body.lines().any(|line| line.contains("call void @rock.erased.") && line.contains("dictionary.factory.dictionary")), "factory result reaches the genuine erased helper");
    let adapter = bodies.iter().find(|body| body.starts_with("internal void @rock.dictionary.selected.")).unwrap();
    assert!(adapter.contains("dictionary.environment") && adapter.contains("call void @rock.erased.selected."));
    let member = bodies.iter().find(|body| body.starts_with("internal void @rock.erased.selected.")).unwrap();
    assert!(member.contains("%dictionary.entry = load ptr")
        && member.lines().any(|line| line.trim_start().starts_with("call void %dictionary.entry(") && line.contains("ptr %3,")),
        "selected blanket member invokes captured Read through its actual dictionary: {member}");
    assert!(bodies.iter().filter(|body| body.starts_with("internal void @rock.erased.")).all(|body|
        body.lines().filter(|line| line.contains("call ")).map(|line| line.split_once('(').unwrap().0.split_whitespace().last().unwrap())
            .all(|callee| callee.starts_with('%') || callee.starts_with("@rock.erased.") || callee.starts_with("@llvm.") || callee == "@rock_dictionary_selected")),
        "producer has no native body/helper/dictionary bypass");
    assert!(!ir.contains("Later"), "producer knows no later payload");
    products.link.object_path = Some("provider.o".into()); let artifact = root.join("provider.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let object_before = fs::read(&object).unwrap(); let artifact_before = fs::read(&artifact).unwrap();
    fs::remove_file(&provider).unwrap(); // Only source created by this harness.
    let entry = root.join("main.rk");
    for wide in [false, true] {
        fs::write(&entry, format!("> dictionary_factory::Dispose\n> dictionary_factory::Read\n> dictionary_factory::Reader\n> dictionary_factory::Service\n> dictionary_factory::as_reader\n{}", client(wide))).unwrap();
        for optimization in [0, 3] {
            let consumer = Config { current_crate_name: Some("later_dictionary_client".into()), extern_artifacts: vec![("dictionary_factory".into(), artifact.clone())], ..config(&root, &entry, optimization) };
            run(&consumer);
            assert_eq!(fs::read(&object).unwrap(), object_before); assert_eq!(fs::read(&artifact).unwrap(), artifact_before);
            assert!(!fs::read_to_string(root.join("main.ll")).unwrap().contains("dictionary.factory.status"), "producer-only runtime root links without rebuilding its body");
        }
    }
}

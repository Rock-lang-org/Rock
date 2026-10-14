//! Partial erased values retain exact initialization and surviving-field cleanup.
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
< trait Service
    @run: T -> T -> Bool -> ()
struct Pair T
    < first: T
    < second: T
< struct Runner
    < tag: I64
impl Service for Runner
    @run = first, second, early !->
        pair = Pair
            first: first
            second: second
        moved = pair.first
        if early then return
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
        unsafe *self.events = ~I64Add (*self.events), 1
main = ->
    mut events = 0
    pointer = (&mut events) as *I64
    runner = Runner
        tag: 0
    object = as_service &runner
    object.run (Later
        events: pointer), (Later
        events: pointer), false
    object.run (Later
        events: pointer), (Later
        events: pointer), true
    if ~I64Eq events, 4 then 42 else 1
"#;

fn scratch() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join(".agent-work/fixtures")
        .join(format!("erased_moves_{}_{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    fs::create_dir_all(&path).unwrap();
    path
}

fn config(root: &Path, entry: &Path, level: u8) -> Config {
    Config { entry_file: entry.into(), no_std: true, no_prelude: true, opt_level: level,
        output_dir: root.join(format!("build{level}")), ..Config::default() }
}

fn observe(root: &Path, config: &Config) {
    rock_lib::compile(config).unwrap_or_else(|errors| panic!("{errors:?}"));
    let result = Command::new(config.output_dir.join("main")).current_dir(root).output().unwrap();
    assert_eq!(result.status.code(), Some(42), "{result:?}");
}

#[test]
fn partial_nominal_move_and_early_return_drop_each_noncopy_field_once_at_o0_o3() {
    for level in [0, 3] {
        let root = scratch();
        let entry = root.join("main.rk");
        fs::write(&entry, format!("{PROVIDER}{CLIENT}")).unwrap();
        observe(&root, &config(&root, &entry, level));
    }
}

#[test]
fn nested_tuple_and_branch_partial_cleanup_keep_disjoint_fields_at_o0_o3() {
    let pair = "        pair = Pair\n            first: first\n            second: second\n        moved = pair.first";
    for provider in [
        PROVIDER.replace(pair, "        pair = ((first, second), ())\n        moved = (pair.0).0"),
        PROVIDER.replace("        moved = pair.first", "        if early\n            moved = pair.first"),
    ] {
        for level in [0, 3] {
            let root = scratch();
            let entry = root.join("main.rk");
            fs::write(&entry, format!("{provider}{CLIENT}")).unwrap();
            observe(&root, &config(&root, &entry, level));
        }
    }
}

#[test]
fn partial_move_does_not_authorize_second_use_or_live_loan_overlap() {
    for (replacement, message) in [
        ("        moved = pair.first\n        again = pair.first", "uninitialized or moved"),
        ("        held = &pair.first\n        moved = pair.first\n        kept = held", "borrow conflict"),
    ] {
        let root = scratch();
        let entry = root.join("main.rk");
        let provider = PROVIDER.replace("        moved = pair.first", replacement);
        fs::write(&entry, format!("{provider}{CLIENT}")).unwrap();
        let errors = rock_lib::compile(&config(&root, &entry, 0)).unwrap_err();
        assert!(errors.0.iter().any(|error| error.code == Some(rock_lib::diagnostic::DiagnosticCode::Borrow)
            && error.message.contains(message)), "{errors:?}");
    }
}

#[test]
fn source_free_partial_move_keeps_later_field_drop_at_o0_o3() {
    for level in [0, 3] {
        let root = scratch();
        let source = root.join("provider.rk");
        fs::write(&source, PROVIDER).unwrap();
        let mut producer = config(&root, &source, level);
        producer.output_dir = root.join("product");
        producer.current_crate_name = Some("provider".into());
        producer.no_link = true;
        producer.emit_object = Some(root.join("provider.o"));
        let mut product = rock_lib::compile_with_products(&producer).unwrap_or_else(|errors| panic!("{errors:?}")).products.unwrap();
        product.link.object_path = Some("provider.o".into());
        product.write_artifact_to_path(&root.join("provider.rkca")).unwrap();
        let object_bytes = fs::read(root.join("provider.o")).unwrap();
        let artifact_bytes = fs::read(root.join("provider.rkca")).unwrap();
        fs::remove_file(source).unwrap();
        let entry = root.join("main.rk");
        let mut consumer = config(&root, &entry, level);
        consumer.extern_artifacts = vec![("provider".into(), root.join("provider.rkca"))];
        for wide in [false, true] {
            let client = if wide {
                CLIENT.replace("    < events: *I64", "    < events: *I64\n    < padding: (I64, I64)")
                    .replace("        events: pointer)", "        events: pointer\n        padding: (3, 4))")
            } else { CLIENT.into() };
            fs::write(&entry, format!("> provider::Dispose\n> provider::Service\n> provider::Runner\n> provider::as_service\n{client}")).unwrap();
            observe(&root, &consumer);
            assert_eq!(fs::read(root.join("provider.o")).unwrap(), object_bytes);
            assert_eq!(fs::read(root.join("provider.rkca")).unwrap(), artifact_bytes);
        }
    }
}

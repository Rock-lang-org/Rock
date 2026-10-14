//! Metadata constructed inside Open retains exact witness and destructor evidence.
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
check_tag: I64 -> ()
check_tag = tag !->
    if ~I64Ne tag, 42 then abort!
record: *I64 -> ()
record = drops !->
    unsafe *drops = ~I64Add (*drops), 1
< trait Read
    @read: I64
struct Frame T
    < address: T
    < drops: *I64
    < tag: I64
impl Dispose for Frame T
    ~@dispose = !->
        check_tag self.tag
        record self.drops
consume: T -> ()
consume = value !-> ()
inspect_open: &Read -> *I64 -> Bool -> ()
< inspect_open = object, drops, early !->
    open object as Hidden, value
        frame = Frame
            address: &value
            drops: drops
            tag: 42
        nested = (frame, (42, 17))
        if early then return ()
        consume nested
"#;

const CLIENT: &str = r#"
struct Item
    < tag: I64
impl Read for Item
    @read = -> self.tag
main = ->
    mut drops = 0
    item = Item
        tag: 42
    object: &Read = &item
    inspect_open object, ((&mut drops) as *I64), false
    inspect_open object, ((&mut drops) as *I64), true
    if ~I64Eq drops, 2 then 42 else 1
"#;

static NEXT: AtomicUsize = AtomicUsize::new(0);
fn scratch() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.agent-work/fixtures")
        .join(format!("scoped-factories-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    fs::create_dir_all(&root).unwrap();
    root
}

fn config(root: &Path, entry: &Path, opt_level: u8) -> Config {
    Config {
        entry_file: entry.into(), output_dir: root.into(), no_std: true, no_prelude: true,
        current_crate_name: Some("scoped_factories".into()), emit_llvm: true, opt_level,
        ..Default::default()
    }
}

fn run(config: &Config) {
    rock_lib::compile(config).unwrap_or_else(|errors| panic!("{errors:?}"));
    let output = Command::new(config.output_dir.join("main")).output().unwrap();
    assert_eq!(output.status.code(), Some(42), "scoped destructor count: {output:?}");
}

#[test]
fn direct_scoped_nominal_and_nested_tuple_cleanup_at_o0_o3() {
    let root = scratch();
    let entry = root.join("main.rk");
    fs::write(&entry, format!("{PROVIDER}{CLIENT}")).unwrap();
    for optimization in [0, 3] { run(&config(&root, &entry, optimization)); }
    let ir = fs::read_to_string(root.join("main.ll")).unwrap();
    assert!(ir.contains("nominal.factory"));
    assert!(ir.contains("rock.nominal.userdrop"));
}

#[test]
fn scoped_nominal_borrow_does_not_authorize_moving_its_reference_storage() {
    let root = scratch();
    let entry = root.join("main.rk");
    let provider = PROVIDER.replace("nested = (frame, (42, 17))", "nested = (frame, (value, 17))");
    fs::write(&entry, format!("{provider}{CLIENT}")).unwrap();
    let errors = rock_lib::compile(&config(&root, &entry, 0)).unwrap_err();
    assert!(errors.0.iter().any(|error| {
        error.code == Some(rock_lib::diagnostic::DiagnosticCode::Borrow)
            && error.primary.as_ref().is_some_and(|label| {
                fs::read_to_string(&entry).unwrap().get(label.span.start..label.span.end) == Some("value")
            })
    }), "{errors:?}");
    assert!(errors.0.iter().all(|error| error.code != Some(rock_lib::diagnostic::DiagnosticCode::Internal)), "{errors:?}");
}

#[test]
fn source_free_scoped_factories_keep_provider_bytes_for_two_later_layouts() {
    let root = scratch();
    let provider = root.join("provider.rk");
    fs::write(&provider, PROVIDER).unwrap();
    let object = root.join("provider.o");
    let mut products = rock_lib::compile_with_products(&Config {
        no_link: true, emit_object: Some(object.clone()), ..config(&root, &provider, 0)
    }).unwrap().products.unwrap();
    products.link.object_path = Some("provider.o".into());
    let artifact = root.join("provider.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let object_bytes = fs::read(&object).unwrap();
    let artifact_bytes = fs::read(&artifact).unwrap();
    fs::remove_file(provider).unwrap();
    let entry = root.join("main.rk");
    for wide in [false, true] {
        let client = if wide {
            CLIENT.replace("< tag: I64", "< tag: I64\n    < padding: (I64, I64)")
                .replace("tag: 42", "tag: 42\n        padding: (2, 3)")
        } else { CLIENT.into() };
        fs::write(&entry, format!("\n> scoped_factories::Read\n> scoped_factories::inspect_open\n{client}")).unwrap();
        for optimization in [0, 3] {
            run(&Config {
                extern_artifacts: vec![("scoped_factories".into(), artifact.clone())],
                current_crate_name: Some("scoped_factories_client".into()),
                ..config(&root, &entry, optimization)
            });
            assert_eq!(fs::read(&object).unwrap(), object_bytes);
            assert_eq!(fs::read(&artifact).unwrap(), artifact_bytes);
        }
    }
}

//! Body-local thin-handle metadata must retain exact identity without payload drop.
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
discard_frame: U -> ()
discard_frame = value !-> ()
struct Frame T
    < address: &T
    < tag: I64
< trait Inspector
    @inspect: T -> ()
< struct Service
    < marker: I64
impl Inspector for Service
    @inspect = value !->
        frame = Frame
            address: &value
            tag: 42
        check_tag frame.tag
        discard_frame frame
as_inspector: &Service -> &Inspector
< as_inspector = service ->
    object: &Inspector = service
    object
"#;

const CLIENT: &str = r#"
struct Later
    < drops: *I64
impl Dispose for Later
    ~@dispose = !->
        unsafe *self.drops = ~I64Add (*self.drops), 1
main = ->
    mut drops = 0
    service = Service
        marker: 0
    object = as_inspector &service
    pointer = (&mut drops) as *I64
    value = Later
        drops: pointer
    object.inspect value
    if ~I64Eq drops, 1 then 42 else 1
"#;

const SCOPED_PROVIDER: &str = r#"
< trait Read
    @read: I64
inspect_frame: T -> ()
inspect_frame = value !->
    frame = Frame
        address: &value
        tag: 42
    check_tag frame.tag
    discard_frame frame
inspect_open: &Read -> ()
< inspect_open = object !->
    open object as Hidden, value
        inspect_frame value
"#;

static NEXT: AtomicUsize = AtomicUsize::new(0);
fn scratch() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.agent-work/fixtures")
        .join(format!("thin-handles-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    fs::create_dir_all(&root).unwrap();
    root
}

fn config(root: &Path, entry: &Path, opt_level: u8) -> Config {
    Config {
        entry_file: entry.into(), output_dir: root.into(), no_std: true, no_prelude: true,
        current_crate_name: Some("thin_handles".into()), emit_llvm: true, opt_level,
        ..Default::default()
    }
}

fn run(config: &Config) {
    rock_lib::compile(config).unwrap_or_else(|errors| panic!("{errors:?}"));
    let output = Command::new(config.output_dir.join("main")).output().unwrap();
    assert_eq!(output.status.code(), Some(42), "handle must not destroy its pointee: {output:?}");
}

#[test]
fn body_local_reference_frame_destroys_its_payload_once_at_o0_o3() {
    let root = scratch();
    let entry = root.join("main.rk");
    fs::write(&entry, format!("{PROVIDER}{CLIENT}")).unwrap();
    for optimization in [0, 3] { run(&config(&root, &entry, optimization)); }
    let ir = fs::read_to_string(root.join("main.ll")).unwrap();
    assert!(ir.contains("thin.handle.frozen.descriptor"));
}

#[test]
fn body_local_mutable_reference_frame_keeps_its_access_and_drop_identity() {
    let root = scratch();
    let entry = root.join("main.rk");
    let provider = PROVIDER.replace("< address: &T", "< address: &mut T")
        .replace("@inspect = value", "@inspect = mut value")
        .replace("address: &value", "address: &mut value");
    fs::write(&entry, format!("{provider}{CLIENT}")).unwrap();
    for optimization in [0, 3] { run(&config(&root, &entry, optimization)); }
}

#[test]
fn raw_pointer_field_uses_the_exact_pointee_without_destroying_it() {
    let root = scratch();
    let entry = root.join("main.rk");
    let provider = PROVIDER.replace("< address: &T", "< address: *T")
        .replace("@inspect: T -> ()", "@inspect: *T -> T -> ()")
        .replace("@inspect = value", "@inspect = address, value")
        .replace("address: &value", "address: address");
    let client = CLIENT.replace("object.inspect value", "other = Later\n        drops: pointer\n    address = (&other) as *Later\n    object.inspect address, value");
    fs::write(&entry, format!("{provider}{client}")).unwrap();
    for optimization in [0, 3] { run(&config(&root, &entry, optimization)); }
}

#[test]
fn reference_frame_does_not_authorize_moving_a_live_borrowed_payload() {
    let root = scratch();
    let entry = root.join("main.rk");
    let provider = PROVIDER.replace("discard_frame frame", "discard_frame value\n        discard_frame frame.address");
    fs::write(&entry, format!("{provider}{CLIENT}")).unwrap();
    let errors = rock_lib::compile(&config(&root, &entry, 0)).unwrap_err();
    assert!(errors.0.iter().any(|error| {
        error.code == Some(rock_lib::diagnostic::DiagnosticCode::Borrow)
            && error.primary.as_ref().is_some_and(|label| {
                fs::read_to_string(&entry).unwrap().get(label.span.start..label.span.end) == Some("discard_frame value")
            })
    }), "{errors:?}");
    assert!(errors.0.iter().all(|error| error.code != Some(rock_lib::diagnostic::DiagnosticCode::Internal)), "{errors:?}");
}

#[test]
fn reference_frame_move_binding_retains_the_conflicting_initializers_span() {
    let root = scratch();
    let entry = root.join("main.rk");
    let provider = PROVIDER.replace("discard_frame frame", "moved = value\n        discard_frame moved\n        discard_frame frame.address");
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
fn deleted_source_reference_frame_retains_exact_runtime_metadata_and_drop() {
    let root = scratch();
    let provider = root.join("provider.rk");
    fs::write(&provider, PROVIDER).unwrap();
    let object = root.join("provider.o");
    let products = rock_lib::compile_with_products(&Config {
        no_link: true, emit_object: Some(object.clone()), ..config(&root, &provider, 0)
    }).unwrap().products.unwrap();
    let mut products = products;
    products.link.object_path = Some("provider.o".into());
    let artifact = root.join("provider.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let object_bytes = fs::read(&object).unwrap();
    let artifact_bytes = fs::read(&artifact).unwrap();
    fs::remove_file(provider).unwrap();
    let entry = root.join("main.rk");
    for wide in [false, true] {
        let client = if wide {
            CLIENT.replace("< drops: *I64", "< drops: *I64\n    < padding: (I64, I64)")
                .replace("drops: pointer", "drops: pointer\n        padding: (2, 3)")
        } else { CLIENT.into() };
        fs::write(&entry, format!("> thin_handles::Dispose\n> thin_handles::Service\n> thin_handles::Inspector\n> thin_handles::as_inspector\n{client}")).unwrap();
        for optimization in [0, 3] {
            run(&Config {
                extern_artifacts: vec![("thin_handles".into(), artifact.clone())],
                current_crate_name: Some("thin_handles_client".into()),
                ..config(&root, &entry, optimization)
            });
            assert_eq!(fs::read(&object).unwrap(), object_bytes);
            assert_eq!(fs::read(&artifact).unwrap(), artifact_bytes);
        }
    }
}

#[test]
fn opened_reference_supplies_exact_identity_to_a_generic_frame_factory() {
    let root = scratch();
    let entry = root.join("main.rk");
    let source = format!(r#"{PROVIDER}{SCOPED_PROVIDER}
struct Item
    < tag: I64
impl Read for Item
    @read = -> self.tag
main = ->
    item = Item
        tag: 42
    object: &Read = &item
    inspect_open object
    42
"#);
    fs::write(&entry, source).unwrap();
    for optimization in [0, 3] { run(&config(&root, &entry, optimization)); }
}

#[test]
fn nested_opened_reference_keeps_exact_metadata_through_a_forwarding_helper() {
    let root = scratch();
    let entry = root.join("main.rk");
    let provider = SCOPED_PROVIDER.replace("        inspect_frame value", "        forward &value");
    fs::write(&entry, format!(r#"{PROVIDER}{provider}
forward: T -> ()
forward = value !-> inspect_frame value
struct Item
    < tag: I64
impl Read for Item
    @read = -> self.tag
main = ->
    item = Item
        tag: 42
    object: &Read = &item
    inspect_open object
    42
"#)).unwrap();
    for optimization in [0, 3] { run(&config(&root, &entry, optimization)); }
}

#[test]
fn source_free_opening_reifies_a_later_witness_without_changing_provider_bytes() {
    let root = scratch();
    let provider = root.join("provider.rk");
    fs::write(&provider, format!("{PROVIDER}{SCOPED_PROVIDER}")).unwrap();
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
        let client = CLIENT.replace("    object = as_inspector &service\n", "")
            .replace("object.inspect value", "object: &Read = &value\n    inspect_open object\n    consume value");
        let client = if wide {
            client.replace("< drops: *I64", "< drops: *I64\n    < padding: (I64, I64)")
                .replace("drops: pointer", "drops: pointer\n        padding: (2, 3)")
        } else { client };
        fs::write(&entry, format!(r#"
> thin_handles::Dispose
> thin_handles::Service
> thin_handles::Read
> thin_handles::inspect_open
consume: T -> ()
consume = value !-> ()
{client}
impl Read for Later
    @read = -> 42
"#)).unwrap();
        for optimization in [0, 3] {
            run(&Config {
                extern_artifacts: vec![("thin_handles".into(), artifact.clone())],
                current_crate_name: Some("scoped_handles_client".into()),
                ..config(&root, &entry, optimization)
            });
            assert_eq!(fs::read(&object).unwrap(), object_bytes);
            assert_eq!(fs::read(&artifact).unwrap(), artifact_bytes);
        }
    }
}

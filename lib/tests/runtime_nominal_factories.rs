//! Genuine body-only producer nominal layouts, Copy projections and exact drops.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use rock_lib::mir::MirErasedType;
use rock_lib::products::{ProductRuntimeSupport, RuntimeRootSymbol};
use rock_lib::Config;

const HEADER: &str = r#"
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
< trait FrameConsumer
    @consume: T -> T -> ()
< struct Service
    < marker: I64
"#;
const PLAIN: &str = r#"
struct Frame T
    < first: T
    < second: T
    < tag: I64
impl FrameConsumer for Service
    @consume = first, second !->
        frame = Frame
            first: first
            second: second
            tag: 42
        check_tag frame.tag
        discard_frame frame
"#;
const NESTED: &str = r#"
struct Frame T
    < values: (T, T)
    < tag: I64
struct FrameLabs T
    < frame: Frame T
    < tag: I64
impl FrameConsumer for Service
    @consume = first, second !->
        frame = Frame
            values: (first, second)
            tag: 42
        labs = FrameLabs
            frame: frame
            tag: 42
        check_tag labs.frame.tag
        check_tag labs.tag
        discard_frame labs
"#;
const EXPORT: &str = r#"
as_consumer: &Service -> &FrameConsumer
< as_consumer = service ->
    object: &FrameConsumer = service
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
    object = as_consumer &service
    pointer = (&mut drops) as *I64
    first = Later
        drops: pointer
    second = Later
        drops: pointer
    object.consume first, second
    if ~I64Eq drops, 2 then 42 else 1
"#;

static NEXT: AtomicUsize = AtomicUsize::new(0);
fn scratch() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.agent-work/fixtures").join(format!("rock-nominal-production-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    fs::create_dir_all(&path).unwrap(); path
}
fn config(root: &Path, entry: &Path, opt_level: u8) -> Config {
    Config { entry_file: entry.into(), output_dir: root.into(), current_crate_name: Some("nominal_factory".into()), no_std: true, no_prelude: true, emit_llvm: true, opt_level, ..Default::default() }
}
fn run(config: &Config) {
    rock_lib::compile(config).unwrap();
    let output = Command::new(config.output_dir.join("main")).output().unwrap();
    assert_eq!(output.status.code(), Some(42), "Copy tag read and exact two non-Copy drops: {output:?}");
}
fn client(wide: bool) -> String {
    if !wide { return CLIENT.into(); }
    CLIENT.replace("struct Later\n    < drops: *I64", "struct Later\n    < leading: I64\n    < drops: *I64\n    < trailing: (I64, I64)")
        .replace("first = Later\n        drops: pointer", "first = Later\n        leading: 1\n        drops: pointer\n        trailing: (2, 3)")
        .replace("second = Later\n        drops: pointer", "second = Later\n        leading: 4\n        drops: pointer\n        trailing: (5, 6)")
}
fn source_free(nested: bool) {
    let root = scratch(); let provider = root.join("provider.rk");
    fs::write(&provider, format!("{HEADER}{}{EXPORT}", if nested { NESTED } else { PLAIN })).unwrap();
    let object = root.join("provider.o");
    let producer_config = Config { no_link: true, emit_object: Some(object.clone()), ..config(&root, &provider, 0) };
    let mut products = rock_lib::compile_with_products(&producer_config).unwrap().products.unwrap();
    assert!(products.link.runtime_support.iter().any(|support| matches!(support,
        ProductRuntimeSupport::NominalDescriptorsV1 { roots, .. } if roots.contains(&RuntimeRootSymbol::NominalDescriptorFactoryV1))));
    assert_eq!(products.link.runtime_support.iter().any(|support| matches!(support, ProductRuntimeSupport::TupleDescriptorsV1 { .. })), nested);
    let slots = products.interface.object_abi.schemas.iter().flat_map(|schema| &schema.erased_slots).collect::<Vec<_>>();
    assert!(!slots.is_empty());
    assert!(slots.iter().all(|slot| slot.signature.layouts.iter().chain(&slot.signature.params).chain(std::iter::once(&slot.signature.ret))
        .all(|ty| !matches!(ty, MirErasedType::Nominal { .. } | MirErasedType::Tuple(_)))), "no public Frame T/tuple layout argument");
    let ir = fs::read_to_string(root.join("provider.ll")).unwrap();
    let bodies = ir.split("define ").skip(1).map(|body| body.split_once("\n}").unwrap().0).collect::<Vec<_>>();
    let body = bodies.iter().find(|body| body.starts_with("internal void @rock.erased.") && body.contains("nominal.factory.status")).unwrap();
    // The sole native call inside the erased method checks the already-Copy
    // I64 tag; construction, forwarding and destruction must remain erased.
    let check = bodies.iter().find(|body| body.contains("call void @abort(") && body.contains("icmp ne i64")).unwrap();
    let check_symbol = format!("@{}", check.lines().next().unwrap().split_once('@').unwrap().1.split_once('(').unwrap().0);
    for implementation in bodies.iter().filter(|body| body.starts_with("internal void @rock.erased.")) {
        assert!(implementation.lines().filter(|line| line.contains("call "))
            .map(|line| line.split_once('(').unwrap().0.split_whitespace().last().unwrap())
            .all(|callee| callee.starts_with('%') || callee.starts_with("@llvm.") || callee.starts_with("@rock.erased.")
                || callee == "@rock_descriptor_nominal" || callee == "@rock_descriptor_tuple" || callee == check_symbol),
            "nominal method/helper cannot call a closed-world native construction/drop bridge: {implementation}");
    }
    let parameters = body.lines().next().unwrap().split_once('(').unwrap().1.split_once(')').unwrap().0;
    assert_eq!(parameters.split(',').count(), 6, "fixed out + Self/T layouts + receiver/two values");
    assert!(body.contains("call i32 @rock_descriptor_nominal"));
    assert!(body.lines().any(|line| line.contains("call void @rock.erased.") && line.contains("nominal.factory.descriptor")), "whole Frame T reaches the actual generic erased helper");
    assert!(body.contains("erased.field"), "Copy field read takes the descriptor's checked offset in the actual erased body");
    if nested { assert!(body.find("tuple.factory.status").unwrap() < body.find("nominal.factory.status").unwrap()); }
    assert!(!ir.contains("Later"), "producer has no later caller type enumeration");
    products.link.object_path = Some("provider.o".into());
    let artifact = root.join("provider.rkca"); products.write_artifact_to_path(&artifact).unwrap();
    let object_before = fs::read(&object).unwrap(); let artifact_before = fs::read(&artifact).unwrap();
    fs::remove_file(&provider).unwrap(); // Only this harness's own source.
    let entry = root.join("main.rk");
    for wide in [false, true] {
        fs::write(&entry, format!("> nominal_factory::Dispose\n> nominal_factory::FrameConsumer\n> nominal_factory::Service\n> nominal_factory::as_consumer\n{}", client(wide))).unwrap();
        for optimization in [0, 3] {
            let consumer = Config { extern_artifacts: vec![("nominal_factory".into(), artifact.clone())], current_crate_name: Some("later_consumer".into()), ..config(&root, &entry, optimization) };
            run(&consumer);
            assert_eq!(fs::read(&object).unwrap(), object_before); assert_eq!(fs::read(&artifact).unwrap(), artifact_before);
            let consumer_ir = fs::read_to_string(root.join("main.ll")).unwrap();
            assert!(!consumer_ir.contains("nominal.factory.status") && !consumer_ir.contains("tuple.factory.status"), "imported producer-only support must link without reconstructing its body");
        }
    }
}

#[test]
fn body_local_frame_constructs_projects_copy_tag_and_drops_at_o0_o3() {
    let root = scratch(); let entry = root.join("main.rk");
    fs::write(&entry, format!("{HEADER}{PLAIN}{EXPORT}{CLIENT}")).unwrap();
    for optimization in [0, 3] { run(&config(&root, &entry, optimization)); }
}
#[test]
fn source_free_frame_uses_producer_only_nominal_support_without_changing_provider_bytes() { source_free(false); }
#[test]
fn source_free_nested_tuple_frame_and_framelabs_use_dependency_first_shared_factories() { source_free(true); }

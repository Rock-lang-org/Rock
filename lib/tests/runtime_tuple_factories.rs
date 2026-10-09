//! Genuine producer-only erased tuple composition and whole-value cleanup.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use rock_lib::mir::MirErasedType;
use rock_lib::products::{ProductRuntimeSupport, RuntimeRootSymbol};
use rock_lib::Config;

const PROVIDER: &str = r#"
lang drop
< trait Dispose
    lang method
    ~@dispose: ()
discard_pair: U -> ()
discard_pair = pair !-> ()
< trait TupleConsumer
    @consume: T -> T -> ()
< struct Service
    < marker: I64
impl TupleConsumer for Service
    @consume = first, second !->
        pair = (first, second)
        discard_pair pair
as_consumer: &Service -> &TupleConsumer
< as_consumer = service ->
    object: &TupleConsumer = service
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
    let path = std::env::temp_dir().join("opencode").join(format!("rock_runtime_tuple_{}_{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    fs::create_dir_all(&path).unwrap(); path
}
fn config(root: &Path, entry: &Path, opt_level: u8) -> Config {
    Config { entry_file: entry.into(), output_dir: root.into(), current_crate_name: Some("tuple_factory".into()), no_std: true, no_prelude: true, emit_llvm: true, opt_level, ..Default::default() }
}
fn execute(config: &Config) {
    rock_lib::compile(config).unwrap();
    let output = Command::new(config.output_dir.join("main")).output().unwrap();
    assert_eq!(output.status.code(), Some(42), "exact two-drop result: {output:?}");
}

#[test]
fn body_local_tuple_creation_forwarding_and_noncopy_drop_execute_at_o0_o3() {
    let root = scratch();
    let entry = root.join("main.rk");
    fs::write(&entry, format!("{PROVIDER}{CLIENT}")).unwrap();
    for optimization in [0, 3] { execute(&config(&root, &entry, optimization)); }
}

#[test]
fn source_free_producer_only_factory_support_preserves_full_provider_bytes() {
    let root = scratch();
    let provider = root.join("provider.rk");
    fs::write(&provider, PROVIDER).unwrap();
    let object = root.join("provider.o");
    let producer_config = Config { no_link: true, emit_object: Some(object.clone()), ..config(&root, &provider, 0) };
    let mut products = rock_lib::compile_with_products(&producer_config).unwrap().products.unwrap();
    assert_eq!(products.link.runtime_support.len(), 1);
    for support in &products.link.runtime_support {
        support.validate().unwrap();
        let ProductRuntimeSupport::TupleDescriptorsV1 { roots, .. } = support else { panic!("unexpected tuple runtime root kind"); };
        assert!(roots.contains(&RuntimeRootSymbol::TupleDescriptorFactoryV1));
    }
    let slots = products.interface.object_abi.schemas.iter().flat_map(|schema| &schema.erased_slots).collect::<Vec<_>>();
    assert!(!slots.is_empty());
    assert!(slots.iter().all(|slot| slot.signature.layouts.iter().chain(&slot.signature.params).chain(std::iter::once(&slot.signature.ret)).all(|ty| !matches!(ty, MirErasedType::Tuple(_)))), "no caller-supplied tuple layout in the public virtual interface");
    let ir = fs::read_to_string(root.join("provider.ll")).unwrap();
    let bodies = ir.split("define ").skip(1).collect::<Vec<_>>();
    let factory_body = bodies.iter().find(|body| body.contains("tuple.factory.status")).unwrap();
    assert!(factory_body.starts_with("internal void @rock.erased."));
    let entry_parameters = factory_body.lines().next().unwrap().split_once('(').unwrap().1.split_once(')').unwrap().0;
    assert_eq!(entry_parameters.split(',').count(), 6, "fixed out + Self/T descriptors + receiver/two values; body tuple is not a public layout argument");
    assert!(factory_body.contains("call i32 @rock_descriptor_tuple"));
    assert!(factory_body.lines().any(|line| line.contains("call void @rock.erased.") && line.contains("tuple.factory.descriptor")), "the actual erased helper receives the factory descriptor");
    assert!(bodies.iter().filter(|body| body.starts_with("internal void @rock.erased.")).count() >= 2, "method and helper are both generic erased entries, not caller instances");
    for body in bodies.iter().filter(|body| body.starts_with("internal void @rock.erased.")) {
        let implementation = body.split_once("\n}").unwrap().0;
        assert!(implementation.lines().filter(|line| line.contains("call "))
            .map(|line| line.split_once('(').unwrap().0.split_whitespace().last().unwrap())
            .all(|callee| callee.starts_with('%') || callee.starts_with("@llvm.") || callee.starts_with("@rock.erased.") || callee == "@rock_descriptor_tuple"),
            "tuple method/helper must not bridge to a native closed-world producer: {implementation}");
    }
    assert!(!ir.contains("Later"), "producer knows no later native payload type");

    products.link.object_path = Some(PathBuf::from("provider.o"));
    let artifact = root.join("provider.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let object_before = fs::read(&object).unwrap();
    let artifact_before = fs::read(&artifact).unwrap();
    // Only the source owned by this harness is removed; keep exact producer bytes.
    fs::remove_file(&provider).unwrap();
    let entry = root.join("main.rk");
    fs::write(&entry, format!("> tuple_factory::Dispose\n> tuple_factory::TupleConsumer\n> tuple_factory::Service\n> tuple_factory::as_consumer\n{CLIENT}")).unwrap();
    for optimization in [0, 3] {
        let consumer = Config { extern_artifacts: vec![("tuple_factory".into(), artifact.clone())], current_crate_name: Some("later_consumer".into()), ..config(&root, &entry, optimization) };
        execute(&consumer);
        assert_eq!(fs::read(&object).unwrap(), object_before);
        assert_eq!(fs::read(&artifact).unwrap(), artifact_before);
        let consumer_ir = fs::read_to_string(root.join("main.ll")).unwrap();
        assert!(!consumer_ir.contains("tuple.factory.status"), "consumer must link producer-only support, not rebuild its factory body");
    }
    // Retain harness-only files for coordinator inspection of IR/object/artifact.
}

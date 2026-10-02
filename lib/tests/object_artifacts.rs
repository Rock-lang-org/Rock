use rock_lib::{products::CompilerProducts, Config};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn library(root: &Path, name: &str, source: &str, dependencies: Vec<(String, PathBuf)>) -> PathBuf {
    let entry = root.join(format!("{name}.rk"));
    std::fs::write(&entry, source).unwrap();
    let object = root.join(format!("{name}.o"));
    let config = Config {
        entry_file: entry,
        output_dir: root.to_path_buf(),
        current_crate_name: Some(name.into()),
        no_std: true,
        no_prelude: true,
        no_link: true,
        emit_object: Some(object),
        extern_artifacts: dependencies,
        ..Default::default()
    };
    let mut products = rock_lib::compile_with_products(&config)
        .unwrap()
        .products
        .unwrap();
    products.link.object_path = Some(PathBuf::from(format!("{name}.o")));
    let artifact = root.join(format!("{name}.rkca"));
    products.write_artifact_to_path(&artifact).unwrap();
    artifact
}

#[test]
fn generic_object_entries_accept_a_later_noncopy_type_and_dictionary() {
    let root = std::env::temp_dir().join(format!(
        "rock_erased_artifacts_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let _cleanup = Scratch(root.clone());
    let producer = library(
        &root,
        "identity_provider",
        r#"
< trait Identity
    @identity: T -> T
< trait Read
    @read: I64
< trait Reader
    @apply: T -> I64 where T: Read
< struct Service
    < marker: I64
impl Identity for Service
    @identity = value -> value
impl Reader for Service
    @apply = value -> value.read!
as_identity: &Service -> &Identity
< as_identity = service ->
    object: &Identity = service
    object
as_reader: &Service -> &Reader
< as_reader = service ->
    object: &Reader = service
    object
"#,
        vec![],
    );
    let object_path = root.join("identity_provider.o");
    let object_before = std::fs::read(&object_path).unwrap();
    let artifact_before = std::fs::read(&producer).unwrap();
    std::fs::remove_file(root.join("identity_provider.rk")).unwrap();
    let entry = root.join("main.rk");
    std::fs::write(
        &entry,
        r#"
> identity_provider::Service
> identity_provider::as_identity
> identity_provider::as_reader
> identity_provider::Read
struct Payload
    < first: I64
    < second: I64
    < third: I64
impl Read for Payload
    @read = -> ~I64Add (~I64Add (self.first), (self.second)), (self.third)
main = ->
    service = Service
        marker: 0
    object = as_identity &service
    input = Payload
        first: 40
        second: 1
        third: 1
    output = object.identity input
    reader = as_reader &service
    reader.apply output
"#,
    )
    .unwrap();
    let caller = Config {
        entry_file: entry,
        output_dir: root.clone(),
        current_crate_name: Some("identity_caller".into()),
        no_std: true,
        no_prelude: true,
        extern_artifacts: vec![("identity_provider".into(), producer.clone())],
        ..Default::default()
    };
    rock_lib::compile(&caller).unwrap();
    let output = std::process::Command::new(root.join("main"))
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(42),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read(object_path).unwrap(), object_before);
    assert_eq!(std::fs::read(producer).unwrap(), artifact_before);
}

#[test]
fn generic_dispatch_separates_traits_implementation_argument_and_caller_artifacts() {
    let root = std::env::temp_dir().join(format!(
        "rock_four_object_artifacts_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let _cleanup = Scratch(root.clone());
    let traits = library(
        &root,
        "contracts",
        r#"
< trait Read
    @read: I64
< trait Apply
    @apply: T -> I64 where T: Read
"#,
        vec![],
    );
    let implementation = library(
        &root,
        "implementation",
        r#"
> contracts::Apply
< struct Service
    < marker: I64
impl Apply for Service
    @apply = value -> value.read!
as_apply: &Service -> &Apply
< as_apply = service ->
    object: &Apply = service
    object
"#,
        vec![("contracts".into(), traits.clone())],
    );
    let argument = library(
        &root,
        "argument",
        r#"
> contracts::Read
< struct Payload
    < first: I64
    < second: I64
    < third: I64
impl Read for Payload
    @read = -> ~I64Add (~I64Add (self.first), (self.second)), (self.third)
"#,
        vec![("contracts".into(), traits.clone())],
    );
    let dependencies = vec![
        ("implementation".into(), implementation.clone()),
        ("argument".into(), argument),
        ("contracts".into(), traits),
    ];
    let producer_files = dependencies
        .iter()
        .flat_map(|(name, artifact)| [artifact.clone(), root.join(format!("{name}.o"))])
        .map(|path| {
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect::<Vec<_>>();
    for name in ["contracts", "implementation", "argument"] {
        std::fs::remove_file(root.join(format!("{name}.rk"))).unwrap();
    }
    let entry = root.join("main.rk");
    std::fs::write(
        &entry,
        r#"
> implementation::Service
> implementation::as_apply
> argument::Payload
main = ->
    service = Service
        marker: 0
    object = as_apply &service
    payload = Payload
        first: 40
        second: 1
        third: 1
    object.apply payload
"#,
    )
    .unwrap();
    let mut caller = Config {
        entry_file: entry,
        output_dir: root.clone(),
        current_crate_name: Some("caller".into()),
        no_std: true,
        no_prelude: true,
        extern_artifacts: dependencies,
        ..Default::default()
    };
    for opt_level in [0, 3] {
        caller.opt_level = opt_level;
        rock_lib::compile(&caller).unwrap();
        let output = std::process::Command::new(root.join("main"))
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(42),
            "optimization {opt_level}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        for (path, bytes) in &producer_files {
            assert_eq!(&std::fs::read(path).unwrap(), bytes);
        }
    }

    let mut malformed =
        CompilerProducts::from_artifact_bytes(&std::fs::read(&implementation).unwrap()).unwrap();
    let slot = malformed
        .interface
        .object_abi
        .schemas
        .iter_mut()
        .flat_map(|schema| &mut schema.erased_slots)
        .next()
        .expect("bounded generic producer slot");
    assert!(!slot.signature.dictionaries.is_empty());
    slot.signature.dictionaries.clear();
    let malformed_path = root.join("malformed_implementation.rkca");
    malformed.write_artifact_to_path(&malformed_path).unwrap();
    caller.extern_artifacts[0].1 = malformed_path;
    let diagnostics = rock_lib::compile(&caller)
        .expect_err("a producer cannot omit the declared bound dictionary");
    assert!(
        diagnostics.0.iter().any(|diagnostic| diagnostic
            .message
            .contains("producer erased slot ABI/evidence mismatch")),
        "{diagnostics:?}"
    );
}

#[test]
fn producer_object_return_default_and_self_upcast_cross_three_artifacts() {
    let root = std::env::temp_dir().join(format!(
        "rock_object_artifacts_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let _cleanup = Scratch(root.clone());
    let traits = library(
        &root,
        "object_traits",
        r#"
< trait Base
    @read: I64
    @next: I64
    @next = -> ~I64Add (@read!), 1
< trait Derived where Self: Base
    @view: &Self
"#,
        vec![],
    );
    let values = library(
        &root,
        "object_values",
        r#"
> object_traits::Base
> object_traits::Derived
< struct Number
    < inner: I64
impl Base for Number
    @read = -> self.inner
impl Derived for Number
    @view = -> self
as_derived: &Number -> &Derived
< as_derived = number ->
    object: &Derived = number
    object
relay: T -> T
< relay = value -> value
"#,
        vec![("object_traits".into(), traits.clone())],
    );
    let product_bytes = std::fs::read(&values).unwrap();
    let products = CompilerProducts::from_artifact_bytes(&product_bytes).unwrap();
    assert!(!products.interface.object_abi.schemas.is_empty());
    assert!(products
        .interface
        .object_abi
        .schemas
        .iter()
        .any(|schema| !schema.views.is_empty()));
    assert!(products
        .interface
        .object_abi
        .schemas
        .iter()
        .flat_map(|schema| &schema.slots)
        .any(|slot| matches!(
            slot.result,
            rock_lib::mir::MirObjectResult::BorrowedSelf { .. }
        )));
    assert!(
        products
            .link
            .records
            .values()
            .all(|record| !record.backend_symbol.starts_with("rock.object.adapter.")),
        "private adapters must not require public link records"
    );
    // Imported generic/default bodies and object-backed entrypoints must not
    // discover producer source files or reconstruct symbols from source names.
    std::fs::remove_file(root.join("object_traits.rk")).unwrap();
    std::fs::remove_file(root.join("object_values.rk")).unwrap();
    let entry = root.join("main.rk");
    std::fs::write(
        &entry,
        r#"
> object_traits::Base
> object_values::Number
> object_values::as_derived
> object_values::relay
main = ->
    number = Number
        inner: 41
    derived = relay (as_derived &number)
    another = derived.view!
    base: &Base = another
    base.next!
"#,
    )
    .unwrap();
    let mut caller = Config {
        entry_file: entry,
        output_dir: root.clone(),
        current_crate_name: Some("object_caller".into()),
        no_std: true,
        no_prelude: true,
        extern_artifacts: vec![
            ("object_values".into(), values),
            ("object_traits".into(), traits),
        ],
        ..Default::default()
    };
    // Verify producer-independent authority before mono can consume the call.
    // In particular, imported generic relay must not pin Self to Number merely
    // because Number supplies the sole visible concrete implementation.
    let analysis = rock_lib::analyze(&caller).unwrap();
    let main = analysis
        .hir
        .program
        .functions
        .values()
        .find(|function| function.name == "main")
        .unwrap();
    let view = main
        .body
        .stmts
        .iter()
        .find_map(|statement| match statement {
            rock_lib::hir::HirStmtFor::Let { name, value, .. } if name == "another" => Some(value),
            _ => None,
        })
        .expect("inferred borrowed Self result");
    let rock_lib::hir::HirExprKindFor::MethodCall(receiver, _, _, _, target) = &view.kind else {
        panic!("virtual view call")
    };
    assert!(matches!(
        target.target,
        rock_lib::hir::HirSelectedMethodTarget::TraitMethod {
            dispatch: rock_lib::hir::HirTraitDispatchKind::Object,
            ..
        }
    ));
    assert!(
        matches!(&receiver.ty, rock_lib::types::Type::Reference { inner, .. } if matches!(inner.as_ref(), rock_lib::types::Type::Object(_)))
    );
    assert_eq!(
        view.ty, receiver.ty,
        "borrowed Self must preserve the object view, not expose Number"
    );
    let mut method_value_caller = caller.clone();
    method_value_caller.source_providers = vec![rock_lib::SourceProvider::Virtual {
        path: caller.entry_file.clone(),
        text: std::fs::read_to_string(&caller.entry_file)
            .unwrap()
            .replace(
                "another = derived.view!",
                "view = derived.view\n    another = view!",
            ),
    }];
    rock_lib::analyze(&method_value_caller)
        .expect("method-value authority must also wait for the imported relay");
    rock_lib::compile(&caller).unwrap();
    let output = std::process::Command::new(root.join("main"))
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(42),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut malformed = products;
    let slot = malformed
        .interface
        .object_abi
        .schemas
        .iter_mut()
        .flat_map(|schema| &mut schema.slots)
        .find(|slot| {
            matches!(
                slot.result,
                rock_lib::mir::MirObjectResult::BorrowedSelf { .. }
            )
        })
        .unwrap();
    slot.result = rock_lib::mir::MirObjectResult::Direct;
    let malformed_path = root.join("malformed_values.rkca");
    malformed.write_artifact_to_path(&malformed_path).unwrap();
    caller.extern_artifacts[0].1 = malformed_path;
    let error = rock_lib::compile(&caller)
        .expect_err("producer slot ABI corruption must be rejected before codegen");
    assert!(
        error.0.iter().any(|diagnostic| diagnostic
            .message
            .contains("object slot signature/result adaptation mismatch")),
        "{error:?}"
    );
}

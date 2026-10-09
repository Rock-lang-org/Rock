use std::path::{Path, PathBuf};
use std::process::Command;

use rock_lib::Config;

fn config(root: &Path, entry: &str, level: u8) -> Config {
    Config {
        entry_file: root.join(entry),
        output_dir: root.to_path_buf(),
        no_std: true,
        no_prelude: true,
        no_link: true,
        opt_level: level,
        emit_object: Some(root.join(entry).with_extension("o")),
        emit_llvm: true,
        ..Default::default()
    }
}

fn execute(
    root: &Path,
    name: &str,
    source: &str,
    level: u8,
    artifact: Option<&Path>,
) -> (usize, usize) {
    std::fs::write(root.join(format!("{name}.rk")), source).unwrap();
    let mut caller = config(root, &format!("{name}.rk"), level);
    if let Some(artifact) = artifact {
        caller.extern_artifacts = vec![("provider".into(), artifact.to_path_buf())];
    }
    rock_lib::compile(&caller).unwrap();
    let mut linker = Command::new("cc");
    linker
        .arg("-no-pie")
        .arg(root.join(format!("{name}.o")))
        .arg(root.join("observer.o"));
    if artifact.is_some() {
        linker.arg(root.join("provider.o"));
    }
    let linked = linker
        .arg("-lm")
        .arg("-o")
        .arg(root.join(name))
        .output()
        .unwrap();
    assert!(
        linked.status.success(),
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    let output = Command::new(root.join(name)).output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(84),
        "{name} O{level}: {output:?}"
    );
    let report = String::from_utf8(output.stderr).unwrap();
    let (allocated, released) = report
        .trim()
        .strip_prefix("COPY_ENV allocations=")
        .unwrap()
        .split_once(" releases=")
        .unwrap();
    let counts = (allocated.parse().unwrap(), released.parse().unwrap());
    assert_eq!(counts.0, counts.1, "{name} O{level}: {report}");
    counts
}

#[test]
fn owning_copy_environments_are_reclaimed_locally_and_without_provider_sources() {
    let root: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".agent-work/fixtures")
        .join(format!("copy_environment_{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("observer.c"),
        include_str!("fixtures/copy_environment_observer.c"),
    )
    .unwrap();
    let compiled = Command::new("cc")
        .args(["-Wall", "-Wextra", "-Werror", "-c"])
        .arg(root.join("observer.c"))
        .arg("-o")
        .arg(root.join("observer.o"))
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    const PROVIDER: &str = "< make = value -> -> value\n< relay = callback -> callback\n< nested = value ->\n    inner = make value\n    -> inner!\n";
    std::fs::write(root.join("provider.rk"), PROVIDER).unwrap();
    let mut producer = config(&root, "provider.rk", 0);
    producer.current_crate_name = Some("provider".into());
    let mut products = rock_lib::compile_with_products(&producer)
        .unwrap()
        .products
        .unwrap();
    products.link.object_path = Some("provider.o".into());
    let artifact = root.join("provider.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let bytes = std::fs::read(&artifact).unwrap();
    let object = std::fs::read(root.join("provider.o")).unwrap();
    std::fs::remove_file(root.join("provider.rk")).unwrap();
    const MAIN: &str = r#"
extern observe_copy_environment: &(() -> I64) -> I64
main = ->
    callback = MAKE 42
    first = relay callback
    pair = (callback, first)
    second_pair = pair
    second = second_pair.0
    ~I64Add (observe_copy_environment &first), (observe_copy_environment &second)
"#;
    for level in [0, 3] {
        let baseline = execute(&root, "baseline", "main = -> 84\n", level, None);
        assert_eq!(baseline, (0, 0));
        for maker in ["make", "nested"] {
            let main = MAIN.replace("MAKE", maker);
            assert!(
                execute(
                    &root,
                    &format!("local_{maker}"),
                    &format!("{PROVIDER}\n{main}"),
                    level,
                    None
                )
                .0 > 0
            );
            assert!(
                execute(
                    &root,
                    &format!("imported_{maker}"),
                    &format!("> provider::make\n> provider::nested\n> provider::relay\n{main}"),
                    level,
                    Some(&artifact)
                )
                .0 > 0
            );
        }
        let discarded = r#"
extern observe_copy_environment: &(() -> I64) -> I64
main = ->
    callback = make 42
    (callback, callback)
    ~I64Add (observe_copy_environment &callback), (observe_copy_environment &callback)
"#;
        assert!(
            execute(
                &root,
                "local_discarded_tuple",
                &format!("{PROVIDER}\n{discarded}"),
                level,
                None,
            )
            .0 > 0
        );
        assert!(
            execute(
                &root,
                "imported_discarded_tuple",
                &format!("> provider::make\n{discarded}"),
                level,
                Some(&artifact),
            )
            .0 > 0
        );
        let array = r#"
extern observe_array_environments: &[(() -> I64); 3] -> I64
main = ->
    callback = make 42
    callbacks = [callback; 3]
    observe_array_environments &callbacks
"#;
        assert!(
            execute(
                &root,
                "local_array",
                &format!("{PROVIDER}\n{array}"),
                level,
                None
            )
            .0 > 0
        );
        assert!(
            execute(
                &root,
                "imported_array",
                &format!("> provider::make\n{array}"),
                level,
                Some(&artifact)
            )
            .0 > 0
        );
        let consumed = r#"
lang drop
< trait Dispose
    lang method
    ~@dispose: ()
struct Guard
    < drops: *I64
impl Dispose for Guard
    ~@dispose = !->
        unsafe *self.drops = ~I64Add (*self.drops), 1
extern observe_copy_environment: &(() -> I64) -> I64
main = ->
    mut drops = 0
    inner = make 42
    guard = Guard
        drops: (&mut drops) as *I64
    consume = ->
        moved = guard
        inner!
    answer = consume!
    result = make answer
    if ~I64Eq drops, 1
        ~I64Add (observe_copy_environment &result), (observe_copy_environment &inner)
    else
        1
"#;
        assert!(
            execute(
                &root,
                "local_consumed",
                &format!("{PROVIDER}\n{consumed}"),
                level,
                None
            )
            .0 > 0
        );
        assert!(
            execute(
                &root,
                "imported_consumed",
                &format!("> provider::make\n{consumed}"),
                level,
                Some(&artifact)
            )
            .0 > 0
        );
        assert_eq!(std::fs::read(&artifact).unwrap(), bytes);
        assert_eq!(std::fs::read(root.join("provider.o")).unwrap(), object);
    }
}

#[test]
fn bodyless_native_consuming_producer_reclaims_moved_and_copy_capture_fields() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".agent-work/fixtures")
        .join(format!(
            "native_consumed_environment_{}",
            std::process::id()
        ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("observer.c"),
        include_str!("fixtures/copy_environment_observer.c"),
    )
    .unwrap();
    let compiled = Command::new("cc")
        .args(["-Wall", "-Wextra", "-Werror", "-c"])
        .arg(root.join("observer.c"))
        .arg("-o")
        .arg(root.join("observer.o"))
        .output()
        .unwrap();
    assert!(compiled.status.success(), "{compiled:?}");
    const PROVIDER: &str = r#"
lang drop
< trait Dispose
    lang method
    ~@dispose: ()
struct NativeGuard
    < drops: *I64
impl Dispose for NativeGuard
    ~@dispose = !->
        unsafe *self.drops = ~I64Add (*self.drops), 1
< make = value -> -> value
< consuming = drops ->
    guard = NativeGuard
        drops: drops
    payload = (guard, 42)
    ->
        moved = payload
        moved.1
"#;
    const MAIN: &str = r#"
extern observe_copy_environment: &(() -> I64) -> I64
main = ->
    mut drops = 0
    callback = consuming ((&mut drops) as *I64)
    answer = callback!
    result = make answer
    if ~I64Eq drops, 1
        ~I64Add (observe_copy_environment &result), (observe_copy_environment &result)
    else
        1
"#;
    std::fs::write(root.join("provider.rk"), PROVIDER).unwrap();
    let mut producer = config(&root, "provider.rk", 0);
    producer.current_crate_name = Some("provider".into());
    let mut products = rock_lib::compile_with_products(&producer)
        .unwrap()
        .products
        .unwrap();
    let consuming = products
        .interface
        .functions
        .iter()
        .find(|(_, declaration)| declaration.name.rsplit("::").next() == Some("consuming"))
        .expect("exported native producer");
    assert!(consuming.1.generic_params.is_empty());
    assert!(products.link.records.contains_key(consuming.0));
    assert!(
        !products.bodies.functions.contains_key(consuming.0),
        "producer must remain bodyless in the consumer"
    );
    assert!(matches!(&consuming.1.ret_type,
        rock_lib::types::Type::Function { callable_kind: rock_lib::types::CallableKind::FnOnce, captures, .. }
        if captures.iter().any(|capture| !capture.ty.is_copy())));
    products.link.object_path = Some("provider.o".into());
    let artifact = root.join("provider.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let bytes = std::fs::read(&artifact).unwrap();
    let object = std::fs::read(root.join("provider.o")).unwrap();
    std::fs::remove_file(root.join("provider.rk")).unwrap();
    for level in [0, 3] {
        assert!(execute(&root, "local", &format!("{PROVIDER}\n{MAIN}"), level, None).0 > 0);
        assert!(
            execute(
                &root,
                "imported",
                &format!("> provider::make\n> provider::consuming\n{MAIN}"),
                level,
                Some(&artifact),
            )
            .0 > 0
        );
        let reused = format!(
            "> provider::make\n> provider::consuming\n{}",
            MAIN.replace("answer = callback!", "answer = callback!\n    callback!")
        );
        std::fs::write(root.join("reused.rk"), reused).unwrap();
        let mut negative = config(&root, "reused.rk", level);
        negative.extern_artifacts = vec![("provider".into(), artifact.clone())];
        let errors = rock_lib::compile(&negative).expect_err("consumed carrier cannot be reused");
        assert!(
            errors.0.iter().any(|error| error.code
                == Some(rock_lib::diagnostic::DiagnosticCode::Borrow)
                && error.message.to_ascii_lowercase().contains("moved")),
            "{errors:?}"
        );
        assert!(
            errors
                .0
                .iter()
                .all(|error| error.code != Some(rock_lib::diagnostic::DiagnosticCode::Internal)),
            "{errors:?}"
        );
        assert_eq!(std::fs::read(&artifact).unwrap(), bytes);
        assert_eq!(std::fs::read(root.join("provider.o")).unwrap(), object);
    }
}

fn abstract_stored_arrow_reclamation(level: u8) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".agent-work/fixtures")
        .join(format!(
            "stored_arrow_environment_{}_o{level}",
            std::process::id()
        ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("observer.c"),
        include_str!("fixtures/copy_environment_observer.c"),
    )
    .unwrap();
    let compiled = Command::new("cc")
        .args(["-Wall", "-Wextra", "-Werror", "-c"])
        .arg(root.join("observer.c"))
        .arg("-o")
        .arg(root.join("observer.o"))
        .output()
        .unwrap();
    assert!(compiled.status.success(), "{compiled:?}");
    let stored = r#"
struct Holder
    < value: () -> I64
holder = seed ->
    Holder
        value: -> seed
extern observe_copy_environment: &(() -> I64) -> I64
main = ->
    stored = holder 42
    ~I64Add (observe_copy_environment &stored.value), (observe_copy_environment &stored.value)
"#;
    assert!(execute(&root, "local_abstract_holder", stored, level, None).0 > 0);
    let copied = stored.replace(
        "~I64Add (observe_copy_environment &stored.value), (observe_copy_environment &stored.value)",
        "first = stored.value\n    second = first\n    ~I64Add (observe_copy_environment &first), (observe_copy_environment &second)",
    );
    assert!(execute(&root, "local_copied_field", &copied, level, None).0 > 1);

    const PROVIDER: &str = r#"
< struct Holder
    < value: () -> I64
< holder = seed ->
    Holder
        value: -> seed
< other = pair ->
    Holder
        value: ->
            values = pair
            ~I64Add values.0, values.1
< relay = held -> held
"#;
    std::fs::write(root.join("provider.rk"), PROVIDER).unwrap();
    let mut producer = config(&root, "provider.rk", level);
    producer.current_crate_name = Some("provider".into());
    let mut products = rock_lib::compile_with_products(&producer)
        .unwrap()
        .products
        .unwrap();
    let holder = products
        .interface
        .functions
        .iter()
        .find(|(_, declaration)| declaration.name.rsplit("::").next() == Some("holder"))
        .unwrap();
    assert!(holder.1.generic_params.is_empty());
    assert!(!products.bodies.functions.contains_key(holder.0));
    assert!(
        matches!(&holder.1.ret_type,
        rock_lib::types::Type::StructRepresentation { fields, .. }
        if matches!(&fields[0], rock_lib::types::Type::Function { captures, .. } if !captures.is_empty())),
        "native nominal producer: {holder:?}; origins: {:?}",
        products.link.native_return_origins
    );
    let holder_id = *holder.0;
    products.link.object_path = Some("provider.o".into());
    let mut malformed = products.clone();
    let rock_lib::types::Type::StructRepresentation { fields, .. } = &mut malformed
        .interface
        .functions
        .get_mut(&holder_id)
        .unwrap()
        .ret_type
    else {
        unreachable!()
    };
    fields[0] = rock_lib::types::Type::Unit;
    let error = malformed
        .write_artifact_to_path(&root.join("malformed.rkca"))
        .unwrap_err();
    assert!(error.contains("nominal field schema"), "{error}");
    let artifact = root.join("provider.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let bytes = std::fs::read(&artifact).unwrap();
    let object = std::fs::read(root.join("provider.o")).unwrap();
    std::fs::remove_file(root.join("provider.rk")).unwrap();
    let imported = r#"
> provider::holder
> provider::other
> provider::relay
extern observe_copy_environment: &(() -> I64) -> I64
main = ->
    stored = relay (holder 42)
    different = other (21, 21)
    first = stored.value
    second = different.value
    ~I64Add (observe_copy_environment &first), (observe_copy_environment &second)
"#;
    assert!(
        execute(
            &root,
            "imported_distinct_fields",
            imported,
            level,
            Some(&artifact)
        )
        .0 > 1
    );
    assert_eq!(std::fs::read(&artifact).unwrap(), bytes);
    assert_eq!(std::fs::read(root.join("provider.o")).unwrap(), object);

    let different_layouts = r#"
> provider::holder
> provider::other
main = !->
    selected = if true
        holder 42
    else
        other (21, 21)
    selected.value!
"#;
    std::fs::write(root.join("incompatible.rk"), different_layouts).unwrap();
    let mut negative = config(&root, "incompatible.rk", level);
    negative.extern_artifacts = vec![("provider".into(), artifact.clone())];
    let errors = rock_lib::compile(&negative)
        .expect_err("a branch cannot guess one of two physical environments");
    assert!(
        errors
            .0
            .iter()
            .any(|error| error.message.contains("agreed field representation")),
        "{errors:?}"
    );
    assert!(
        errors
            .0
            .iter()
            .all(|error| error.code != Some(rock_lib::diagnostic::DiagnosticCode::Internal)),
        "{errors:?}"
    );
}

#[test]
fn abstract_stored_arrow_environment_is_reclaimed_at_o0() {
    abstract_stored_arrow_reclamation(0);
}

#[test]
fn abstract_stored_arrow_environment_is_reclaimed_at_o3() {
    abstract_stored_arrow_reclamation(3);
}

fn nominal_storage_boundary(name: &str, source: &str, minimum_allocations: usize) {
    for level in [0, 3] {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(".agent-work/fixtures")
            .join(format!("{name}_{}_o{level}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("observer.c"),
            include_str!("fixtures/copy_environment_observer.c"),
        )
        .unwrap();
        let compiled = Command::new("cc")
            .args(["-Wall", "-Wextra", "-Werror", "-c"])
            .arg(root.join("observer.c"))
            .arg("-o")
            .arg(root.join("observer.o"))
            .output()
            .unwrap();
        assert!(compiled.status.success(), "{compiled:?}");
        let (allocations, _) = execute(&root, name, source, level, None);
        assert!(
            allocations >= minimum_allocations,
            "{name} O{level}: owning field copies must have independent environments"
        );
    }
}

const NOMINAL_STATIC_FACTORY_PROVIDER: &str = r#"
< trait Factory
    make: I64 -> Self
< trait Dispatch
    @build: I64 -> T where T: Factory
< struct Service
    < marker: I64
impl Dispatch for Service
    @build: I64 -> T where T: Factory
    @build = seed ->
        unused = T::make seed
        T::make seed
as_dispatch: &Service -> &Dispatch
< as_dispatch = service ->
    object: &Dispatch = service
    object
< trait CopyValue
    ~@into_value: I64
impl CopyValue for I64
    ~@into_value = -> self
< struct Holder
    < value: () -> I64
impl Factory for Holder
    make = seed ->
        Holder
            value: -> seed.into_value!
consume: Holder -> ()
< consume = held -> ()
borrowed_copy: &Holder -> ()
< borrowed_copy = held ->
    callback = held.value
    ()
"#;

const NOMINAL_STATIC_FACTORY_MAIN: &str = r#"
extern observe_copy_environment: &(() -> I64) -> I64
extern observe_array_environments: &[(() -> I64); 3] -> I64
main = ->
    service = Service
        marker: 0
    object = as_dispatch &service
    held: Holder = object.build 42
    callbacks = [held.value, held.value, held.value]
    observe_array_environments &callbacks
"#;

#[test]
fn nominal_static_factory_result_reclaims_its_measured_environment() {
    let source = format!("{NOMINAL_STATIC_FACTORY_PROVIDER}{NOMINAL_STATIC_FACTORY_MAIN}");
    nominal_storage_boundary("nominal_static_factory", &source, 4);
}

#[test]
fn bodyless_static_nominal_factory_preserves_environment_cleanup_without_provider_sources() {
    for level in [0, 3] {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(".agent-work/fixtures")
            .join(format!(
                "bodyless_nominal_factory_{}_o{level}",
                std::process::id()
            ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("observer.c"),
            include_str!("fixtures/copy_environment_observer.c"),
        )
        .unwrap();
        let compiled = Command::new("cc")
            .args(["-Wall", "-Wextra", "-Werror", "-c"])
            .arg(root.join("observer.c"))
            .arg("-o")
            .arg(root.join("observer.o"))
            .output()
            .unwrap();
        assert!(compiled.status.success(), "{compiled:?}");
        std::fs::write(root.join("provider.rk"), NOMINAL_STATIC_FACTORY_PROVIDER).unwrap();
        let mut producer = config(&root, "provider.rk", level);
        producer.current_crate_name = Some("provider".into());
        let mut products = rock_lib::compile_with_products(&producer)
            .unwrap()
            .products
            .unwrap();
        products.link.object_path = Some("provider.o".into());
        let artifact = root.join("provider.rkca");
        products.write_artifact_to_path(&artifact).unwrap();
        let artifact_bytes = std::fs::read(&artifact).unwrap();
        let object_bytes = std::fs::read(root.join("provider.o")).unwrap();
        std::fs::remove_file(root.join("provider.rk")).unwrap();
        let source = format!("> provider::Service\n> provider::Holder\n> provider::as_dispatch\n{NOMINAL_STATIC_FACTORY_MAIN}");
        assert!(
            execute(
                &root,
                "bodyless_nominal_factory",
                &source,
                level,
                Some(&artifact)
            )
            .0 >= 4
        );
        assert_eq!(std::fs::read(&artifact).unwrap(), artifact_bytes);
        assert_eq!(
            std::fs::read(root.join("provider.o")).unwrap(),
            object_bytes
        );
        assert!(!root.join("provider.rk").exists());
        for (name, argument) in [("consume", "held"), ("borrowed_copy", "&held")] {
            let source = format!(
                "> provider::Service\n> provider::Holder\n> provider::as_dispatch\n> provider::{name}\nmain = !->\n    service = Service\n        marker: 0\n    object = as_dispatch &service\n    held: Holder = object.build 42\n    {name} {argument}\n"
            );
            std::fs::write(root.join("incompatible.rk"), source).unwrap();
            let mut caller = config(&root, "incompatible.rk", level);
            caller.extern_artifacts = vec![("provider".into(), artifact.clone())];
            let diagnostics = rock_lib::compile(&caller)
                .expect_err("bodyless native code cannot guess a different callable field layout");
            assert!(
                diagnostics.0.iter().any(|diagnostic| diagnostic
                    .message
                    .contains("native parameter requires its declared storage representation")),
                "{diagnostics:?}"
            );
            assert!(
                diagnostics.0.iter().all(|diagnostic| diagnostic.code
                    != Some(rock_lib::diagnostic::DiagnosticCode::Internal)),
                "{diagnostics:?}"
            );
        }
        assert_eq!(std::fs::read(&artifact).unwrap(), artifact_bytes);
        assert_eq!(
            std::fs::read(root.join("provider.o")).unwrap(),
            object_bytes
        );
    }
}

#[test]
fn nominal_owned_parameter_preserves_callable_storage() {
    nominal_storage_boundary(
        "owned_nominal_parameter",
        r#"
struct Holder
    < value: () -> I64
holder = seed ->
    Holder
        value: -> seed
holder_pair = pair ->
    Holder
        value: ->
            values = pair
            ~I64Add values.0, values.1
extern observe_copy_environment: &(() -> I64) -> I64
extern observe_array_environments: &[(() -> I64); 3] -> I64
inspect: Holder -> I64
inspect = held ->
    callbacks = [held.value, held.value, held.value]
    observe_array_environments &callbacks
main = ->
    first = inspect (holder 42)
    if ~I64Eq first, 84
        inspect (holder_pair (20, 22))
    else
        0
"#,
        6,
    );
}

#[test]
fn nominal_struct_pattern_preserves_measured_callable_environment() {
    let main = NOMINAL_STATIC_FACTORY_MAIN.replace(
        "callbacks = [held.value, held.value, held.value]",
        "first = match held\n        Holder value: value => value\n    callbacks = [first, first, first]",
    );
    let source = format!("{NOMINAL_STATIC_FACTORY_PROVIDER}{main}");
    nominal_storage_boundary("nominal_struct_pattern", &source, 4);
}

#[test]
fn nominal_enum_payload_preserves_measured_callable_environment() {
    let main = NOMINAL_STATIC_FACTORY_MAIN.replace(
        "callbacks = [held.value, held.value, held.value]",
        "wrapped = Envelope::Held held\n    first = match wrapped\n        Envelope::Held value => value.value\n    callbacks = [first, first, first]",
    );
    let source =
        format!("{NOMINAL_STATIC_FACTORY_PROVIDER}\nenum Envelope\n    Held Holder\n{main}");
    nominal_storage_boundary("nominal_enum_payload", &source, 4);
}

const ENUM_NATIVE_PROVIDER: &str = r#"
< trait CopyValue
    ~@into_value: I64
impl CopyValue for I64
    ~@into_value = -> self
< struct Holder
    < value: () -> I64
< enum Envelope
    Held Holder
make: I64 -> Envelope
< make = seed ->
    held = Holder
        value: -> seed.into_value!
    Envelope::Held held
consume: Envelope -> ()
< consume = wrapped -> ()
borrowed_copy: &Envelope -> ()
< borrowed_copy = wrapped -> ()
"#;

#[test]
fn nominal_enum_active_variants_reclaim_only_their_measured_payloads() {
    nominal_storage_boundary(
        "enum_active_variants",
        r#"
lang drop
< trait Dispose
    lang method
    ~@dispose: ()
struct Guard
    < drops: *I64
impl Dispose for Guard
    ~@dispose = !->
        unsafe *self.drops = ~I64Add (*self.drops), 1
struct Holder
    < value: () -> I64
enum Choice
    Empty
    One Holder, Guard
    Two Holder, Holder, Guard
holder = seed ->
    Holder
        value: -> seed
extern observe_copy_environment: &(() -> I64) -> I64
extern observe_array_environments: &[(() -> I64); 3] -> I64
main = ->
    mut drops = 0
    answer = if true
        empty = Choice::Empty
        one = Choice::One (holder 42), (Guard drops: ((&mut drops) as *I64))
        two = Choice::Two (holder 42), (holder 42), (Guard drops: ((&mut drops) as *I64))
        match two
            Choice::Empty => 0
            Choice::One held, guard =>
                callbacks = [held.value, held.value, held.value]
                observe_array_environments &callbacks
            Choice::Two left, right, guard =>
                callbacks = [left.value, left.value, left.value]
                if ~I64Eq (observe_copy_environment &left.value), 42
                    observe_array_environments &callbacks
                else
                    0
    else
        0
    if ~I64Eq drops, 2
        answer
    else
        1
"#,
        4,
    );
}

#[test]
fn consuming_enum_capture_transports_result_before_payload_cleanup() {
    let source = format!(
        r#"{ENUM_NATIVE_PROVIDER}
lang drop
< trait Dispose
    lang method
    ~@dispose: ()
struct Guard
    < drops: *I64
impl Dispose for Guard
    ~@dispose = !->
        unsafe *self.drops = ~I64Add (*self.drops), 1
extern observe_copy_environment: &(() -> I64) -> I64
extern observe_array_environments: &[(() -> I64); 3] -> I64
main = ->
    mut drops = 0
    wrapped = make 42
    guard = Guard
        drops: (&mut drops) as *I64
    take_result = ->
        moved = guard
        moved_wrapped = wrapped
        match moved_wrapped
            Envelope::Held held => held.value
    first = take_result!
    callbacks = [first, first, first]
    if ~I64Eq drops, 1
        if ~I64Eq (observe_copy_environment &first), 42
            observe_array_environments &callbacks
        else
            0
    else
        1
"#
    );
    nominal_storage_boundary("enum_consuming_result", &source, 4);
}

#[test]
fn unmatched_callable_branch_storage_cannot_erase_environment_ownership() {
    for (name, selection) in [
        (
            "match",
            "match wrapped\n        Choice::Empty => -> 0\n        Choice::Held held => held.value",
        ),
        (
            "conditional",
            "if true\n        make_callback 42\n    else\n        -> 0",
        ),
    ] {
        let source = format!(
            r#"
struct Holder
    < value: () -> I64
enum Choice
    Empty
    Held Holder
make_callback = seed -> -> seed
main = !->
    held = Holder
        value: make_callback 42
    wrapped = Choice::Held held
    selected = {selection}
    selected!
"#
        );
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(".agent-work/fixtures")
            .join(format!("disagreed_callable_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("main.rk"), &source).unwrap();
        for level in [0, 3] {
            let errors = rock_lib::compile(&config(&root, "main.rk", level))
                .expect_err("a canonical arrow cannot erase differing owning environments");
            assert!(
                errors.0.iter().any(|error| error.code
                    == Some(rock_lib::diagnostic::DiagnosticCode::Mono)
                    && error.message.contains("agreed field representation")
                    && error
                        .primary
                        .as_ref()
                        .is_some_and(|label| label.span.file_path == root.join("main.rk"))),
                "{errors:?}"
            );
            assert!(errors.0.iter().all(|error| error.code != Some(rock_lib::diagnostic::DiagnosticCode::Internal)), "{errors:?}");
        }
    }
}

#[test]
fn early_return_callable_storage_is_checked_in_both_orders() {
    for (name, early, tail) in [
        ("captured_first", "make_callback seed", "-> 0"),
        ("plain_first", "-> 0", "make_callback seed"),
        ("different_captures", "make_callback seed", "make_pair (20, 22)"),
    ] {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(".agent-work/fixtures")
            .join(format!("early_storage_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let source = format!(r#"
make_callback = seed -> -> seed
make_pair = pair -> -> ~I64Add pair.0, pair.1
choose = flag, seed ->
    if flag
        return ({early})
    {tail}
main = !->
    callback = choose true, 42
    callback!
"#);
        std::fs::write(root.join("main.rk"), source).unwrap();
        for level in [0, 3] {
            let errors = rock_lib::compile(&config(&root, "main.rk", level))
                .expect_err("every reachable return must agree, including a plain tail");
            assert!(errors.0.iter().any(|error| {
                error.code == Some(rock_lib::diagnostic::DiagnosticCode::Mono)
                    && error.message.contains("agreed field representation")
                    && error.primary.as_ref().is_some_and(|label| {
                        label.span.file_path == root.join("main.rk")
                    })
            }), "{name} O{level}: {errors:?}");
            assert!(errors.0.iter().all(|error| {
                error.code != Some(rock_lib::diagnostic::DiagnosticCode::Internal)
            }), "{name} O{level}: {errors:?}");
        }
    }
}

#[test]
fn compatible_early_return_callable_storage_is_reclaimed() {
    nominal_storage_boundary("compatible_early_storage", r#"
extern observe_array_environments: &[(() -> I64); 3] -> I64
make_callback = seed -> -> seed
make_other = seed -> -> seed
choose = flag, seed ->
    if flag
        return (make_callback seed)
    make_other seed
main = ->
    callback = choose true, 42
    second = choose false, 42
    third = callback
    callbacks = [callback, second, third]
    observe_array_environments &callbacks
"#, 3);
}

#[test]
fn all_explicit_returns_preserve_callable_storage() {
    nominal_storage_boundary("all_explicit_storage", r#"
extern observe_array_environments: &[(() -> I64); 3] -> I64
make_callback = seed -> -> seed
choose = flag, seed ->
    if flag
        return (make_callback seed)
    else
        return (make_callback seed)
main = ->
    callback = choose true, 42
    second = choose false, 42
    third = callback
    callbacks = [callback, second, third]
    observe_array_environments &callbacks
"#, 3);
}

#[test]
fn nested_callable_returns_and_noncontinuing_branches_preserve_storage() {
    for (name, declaration) in [
        ("loop", r#"choose = flag, seed ->
    loop
        if flag
            return (make_callback seed)
        break
    make_callback seed
"#),
        ("lambda", r#"choose = flag, seed ->
    producer = ->
        if flag
            return (make_callback seed)
        make_callback seed
    producer!
"#),
        ("lambda_all_returns", r#"choose = flag, seed ->
    producer = ->
        if flag
            return (make_callback seed)
        else
            return (make_callback seed)
    producer!
"#),
        ("continuing_conditional", r#"choose = flag, seed ->
    if flag
        return (make_callback seed)
    else
        make_callback seed
"#),
        ("continuing_match", r#"choose = flag, seed ->
    match flag
        true => return (make_callback seed)
        false => make_callback seed
"#),
        ("nominal", r#"struct Holder
    < value: () -> I64
held = seed ->
    Holder
        value: make_callback seed
select = flag, seed ->
    if flag
        return (held seed)
    held seed
choose = flag, seed ->
    holder = select flag, seed
    holder.value
"#),
        ("method", r#"struct Service
    < marker: I64
impl Service
    @choose = flag, seed ->
        if flag
            return (make_callback seed)
        make_callback seed
choose = flag, seed ->
    service = Service
        marker: 0
    service.choose flag, seed
"#),
        ("nominal_method", r#"struct Service
    < marker: I64
struct Holder
    < value: () -> I64
impl Service
    @choose = flag, seed ->
        if flag
            return (Holder
                value: make_callback seed)
        Holder
            value: make_callback seed
choose = flag, seed ->
    service = Service
        marker: 0
    held = service.choose flag, seed
    held.value
"#),
        ("generic_nominal_method", r#"struct Service T
    < marker: T
struct Holder
    < value: () -> I64
impl Service T
    @choose = flag, seed ->
        if flag
            return (Holder
                value: make_callback seed)
        Holder
            value: make_callback seed
choose = flag, seed ->
    service = Service
        marker: 0
    held = service.choose flag, seed
    held.value
"#),
        ("enum", r#"struct Holder
    < value: () -> I64
enum Envelope
    Held Holder
wrap = seed ->
    held = Holder
        value: make_callback seed
    Envelope::Held held
select = flag, seed ->
    if flag
        return (wrap seed)
    wrap seed
choose = flag, seed ->
    wrapped = select flag, seed
    match wrapped
        Envelope::Held held => held.value
"#),
        ("nested_tuple", r#"select = flag, seed ->
    if flag
        return (((make_callback seed), 7), 8)
    (((make_callback seed), 7), 8)
choose = flag, seed ->
    pair = select flag, seed
    (pair.0).0
"#),
    ] {
        let source = format!(r#"
extern observe_array_environments: &[(() -> I64); 3] -> I64
make_callback = seed -> -> seed
{declaration}
main = ->
    first = choose true, 42
    second = choose false, 42
    callbacks = [first, second, first]
    observe_array_environments &callbacks
"#);
        nominal_storage_boundary(&format!("nested_early_{name}"), &source, 3);
    }
}

#[test]
fn early_return_callable_array_repeats_preserve_each_owned_environment() {
    nominal_storage_boundary("early_return_array_repeat", r#"
extern observe_array_environments: &[(() -> I64); 3] -> I64
make_callback = seed -> -> seed
select = flag, seed ->
    if flag
        return [make_callback seed; 3]
    [make_callback seed; 3]
main = ->
    first = select true, 42
    second = select false, 42
    if ~I64Eq (observe_array_environments &first), 84
        observe_array_environments &second
    else
        0
"#, 6);
}

#[test]
fn generalized_array_repeat_still_rejects_noncopy_capture_storage() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
        .join(".agent-work/fixtures")
        .join(format!("noncopy_repeat_{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("main.rk"), r#"
make_callback = seed -> -> seed
repeat = seed -> [make_callback seed; 3]
main = !->
    values = repeat ([42])
"#).unwrap();
    for level in [0, 3] {
        let errors = rock_lib::compile(&config(&root, "main.rk", level))
            .expect_err("an actually non-Copy capture cannot be repeated");
        assert!(errors.0.iter().any(|error| {
            error.code == Some(rock_lib::diagnostic::DiagnosticCode::Type)
                && error.message.contains("array repeat initializer must be Copy")
                && error.primary.as_ref().is_some_and(|label| label.span.file_path == root.join("main.rk"))
        }), "{errors:?}");
        assert!(errors.0.iter().all(|error| error.code != Some(rock_lib::diagnostic::DiagnosticCode::Internal)), "{errors:?}");
    }
}

#[test]
fn bodyless_early_return_nominal_producer_preserves_storage_without_sources() {
    const PROVIDER: &str = r#"
trait CopyValue
    ~@into_value: I64
impl CopyValue for I64
    ~@into_value = -> self
< struct Holder
    < value: () -> I64
make: Bool -> I64 -> Holder
< make = flag, seed ->
    if flag
        return (Holder
            value: -> seed.into_value!)
    Holder
        value: -> seed.into_value!
< relay = producer, flag, seed -> producer flag, seed
< transport = value -> value
"#;
    for level in [0, 3] {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent().unwrap().join(".agent-work/fixtures")
            .join(format!("bodyless_early_{}_o{level}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("observer.c"), include_str!("fixtures/copy_environment_observer.c")).unwrap();
        let compiled = Command::new("cc").args(["-Wall", "-Wextra", "-Werror", "-c"])
            .arg(root.join("observer.c")).arg("-o").arg(root.join("observer.o"))
            .output().unwrap();
        assert!(compiled.status.success(), "{compiled:?}");
        std::fs::write(root.join("provider.rk"), PROVIDER).unwrap();
        let mut producer = config(&root, "provider.rk", level);
        producer.current_crate_name = Some("provider".into());
        let mut products = rock_lib::compile_with_products(&producer).unwrap().products.unwrap();
        let (id, make) = products.interface.functions.iter()
            .find(|(_, function)| function.name.rsplit("::").next() == Some("make")).unwrap();
        assert!(make.generic_params.is_empty());
        assert!(!products.bodies.functions.contains_key(id));
        assert!(products.link.records.contains_key(id));
        assert!(matches!(make.ret_type, rock_lib::types::Type::StructRepresentation { .. }));
        assert!(!products.link.native_return_origins.contains_key(id),
            "untracked non-scalar storage must not publish an invented loan certificate");
        let (transport_id, transport) = products.interface.functions.iter()
            .find(|(_, function)| function.name.rsplit("::").next() == Some("transport")).unwrap();
        assert!(!transport.generic_params.is_empty());
        assert!(products.bodies.functions.contains_key(transport_id));
        products.link.object_path = Some("provider.o".into());
        let artifact = root.join("provider.rkca");
        products.write_artifact_to_path(&artifact).unwrap();
        let bytes = std::fs::read(&artifact).unwrap();
        let object = std::fs::read(root.join("provider.o")).unwrap();
        std::fs::remove_file(root.join("provider.rk")).unwrap();
        for entry in ["make", "transport"] {
            let first = if entry == "transport" { "transport (make true, 42)" } else { "make true, 42" };
            let second = if entry == "transport" { "transport (make false, 42)" } else { "make false, 42" };
            let source = format!(r#"
> provider::make
> provider::transport
extern observe_array_environments: &[(() -> I64); 3] -> I64
main = ->
    first = {first}
    second = {second}
    callbacks = [first.value, second.value, first.value]
    observe_array_environments &callbacks
"#);
            assert!(execute(&root, &format!("bodyless_early_{entry}"), &source, level, Some(&artifact)).0 >= 3);
        }
        std::fs::write(root.join("uncertified.rk"), r#"
> provider::make
> provider::relay
main = !->
    held = relay make, true, 42
    held.value!
"#).unwrap();
        let mut caller = config(&root, "uncertified.rk", level);
        caller.extern_artifacts = vec![("provider".into(), artifact.clone())];
        let errors = rock_lib::compile(&caller)
            .expect_err("nominal storage cannot supply a missing native producer loan record");
        assert!(errors.0.iter().any(|error| {
            error.code == Some(rock_lib::diagnostic::DiagnosticCode::Borrow)
                && error.message.contains("Callable result borrowing contract")
                && error.primary.as_ref().is_some_and(|label| label.span.file_path == root.join("uncertified.rk"))
        }), "{errors:?}");
        assert!(errors.0.iter().all(|error| error.code != Some(rock_lib::diagnostic::DiagnosticCode::Internal)), "{errors:?}");
        assert_eq!(std::fs::read(&artifact).unwrap(), bytes);
        assert_eq!(std::fs::read(root.join("provider.o")).unwrap(), object);
        assert!(!root.join("provider.rk").exists());
    }
}

#[test]
fn nominal_enum_native_return_preserves_storage_without_provider_sources() {
    let provider = ENUM_NATIVE_PROVIDER;
    const MAIN: &str = r#"
extern observe_copy_environment: &(() -> I64) -> I64
extern observe_array_environments: &[(() -> I64); 3] -> I64
main = ->
    wrapped = make 42
    first = match wrapped
        Envelope::Held value => value.value
    callbacks = [first, first, first]
    if ~I64Eq (observe_copy_environment &first), 42
        observe_array_environments &callbacks
    else
        0
"#;
    nominal_storage_boundary("enum_native_local", &format!("{provider}{MAIN}"), 4);
    for level in [0, 3] {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(".agent-work/fixtures")
            .join(format!("enum_native_{}_o{level}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("observer.c"),
            include_str!("fixtures/copy_environment_observer.c"),
        )
        .unwrap();
        let compiled = Command::new("cc")
            .args(["-Wall", "-Wextra", "-Werror", "-c"])
            .arg(root.join("observer.c"))
            .arg("-o")
            .arg(root.join("observer.o"))
            .output()
            .unwrap();
        assert!(compiled.status.success(), "{compiled:?}");
        std::fs::write(root.join("provider.rk"), provider).unwrap();
        let mut producer = config(&root, "provider.rk", level);
        producer.current_crate_name = Some("provider".into());
        let mut products = rock_lib::compile_with_products(&producer)
            .unwrap()
            .products
            .unwrap();
        let (id, make) = products
            .interface
            .functions
            .iter()
            .find(|(_, function)| function.name.rsplit("::").next() == Some("make"))
            .unwrap();
        assert!(make.generic_params.is_empty());
        assert!(!products.bodies.functions.contains_key(id));
        assert!(products.link.records.contains_key(id));
        assert!(
            matches!(
                make.ret_type,
                rock_lib::types::Type::EnumRepresentation { .. }
            ),
            "{make:?}"
        );
        products.link.object_path = Some("provider.o".into());
        let artifact = root.join("provider.rkca");
        products.write_artifact_to_path(&artifact).unwrap();
        let bytes = std::fs::read(&artifact).unwrap();
        let object = std::fs::read(root.join("provider.o")).unwrap();
        std::fs::remove_file(root.join("provider.rk")).unwrap();
        let main = format!("> provider::make\n> provider::Envelope\n{MAIN}");
        assert!(execute(&root, "enum_native_imported", &main, level, Some(&artifact)).0 >= 4);
        for (name, argument) in [("consume", "wrapped"), ("borrowed_copy", "&wrapped")] {
            let source = format!("> provider::make\n> provider::{name}\nmain = !->\n    wrapped = make 42\n    {name} {argument}\n");
            std::fs::write(root.join("incompatible.rk"), source).unwrap();
            let mut caller = config(&root, "incompatible.rk", level);
            caller.extern_artifacts = vec![("provider".into(), artifact.clone())];
            let diagnostics = rock_lib::compile(&caller)
                .expect_err("bodyless enum parameter cannot erase measured payloads");
            assert!(
                diagnostics.0.iter().any(|diagnostic| diagnostic
                    .message
                    .contains("native parameter requires its declared storage representation")),
                "{diagnostics:?}"
            );
            assert!(
                diagnostics.0.iter().all(|diagnostic| diagnostic.code
                    != Some(rock_lib::diagnostic::DiagnosticCode::Internal)),
                "{diagnostics:?}"
            );
        }
        assert_eq!(std::fs::read(&artifact).unwrap(), bytes);
        assert_eq!(std::fs::read(root.join("provider.o")).unwrap(), object);
        assert!(!root.join("provider.rk").exists());
    }
}

#[test]
fn nominal_enum_owned_and_borrowed_parameters_preserve_payload_storage() {
    for (name, parameter, scrutinee, argument) in [
        ("owned_enum_parameter", "Envelope", "wrapped", "wrapped"),
        (
            "borrowed_enum_parameter",
            "&Envelope",
            "*wrapped",
            "&wrapped",
        ),
    ] {
        let source = format!(
            r#"{ENUM_NATIVE_PROVIDER}
extern observe_array_environments: &[(() -> I64); 3] -> I64
inspect: {parameter} -> I64
inspect = wrapped ->
    first = match {scrutinee}
        Envelope::Held value => value.value
    callbacks = [first, first, first]
    observe_array_environments &callbacks
main = ->
    wrapped = make 42
    inspect {argument}
"#
        );
        nominal_storage_boundary(name, &source, 3);
    }
}

#[test]
fn nominal_enum_static_factory_preserves_storage_without_provider_sources() {
    let provider = NOMINAL_STATIC_FACTORY_PROVIDER.replace(
        "impl Factory for Holder\n    make = seed ->\n        Holder\n            value: -> seed.into_value!",
        "< enum Envelope\n    Held Holder\nimpl Factory for Envelope\n    make = seed ->\n        held = Holder\n            value: -> seed.into_value!\n        Envelope::Held held",
    );
    let main = NOMINAL_STATIC_FACTORY_MAIN.replace("held: Holder", "held: Envelope").replace(
        "callbacks = [held.value, held.value, held.value]",
        "first = match held\n        Envelope::Held value => value.value\n    callbacks = [first, first, first]",
    );
    nominal_storage_boundary(
        "enum_virtual_factory_local",
        &format!("{provider}{main}"),
        4,
    );
    for level in [0, 3] {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(".agent-work/fixtures")
            .join(format!(
                "enum_virtual_factory_{}_o{level}",
                std::process::id()
            ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("observer.c"),
            include_str!("fixtures/copy_environment_observer.c"),
        )
        .unwrap();
        let compiled = Command::new("cc")
            .args(["-Wall", "-Wextra", "-Werror", "-c"])
            .arg(root.join("observer.c"))
            .arg("-o")
            .arg(root.join("observer.o"))
            .output()
            .unwrap();
        assert!(compiled.status.success(), "{compiled:?}");
        std::fs::write(root.join("provider.rk"), &provider).unwrap();
        let mut producer = config(&root, "provider.rk", level);
        producer.current_crate_name = Some("provider".into());
        let mut products = rock_lib::compile_with_products(&producer)
            .unwrap()
            .products
            .unwrap();
        let methods = products
            .interface
            .impls
            .values()
            .flat_map(|implementation| implementation.methods.values())
            .filter(|method| {
                matches!(
                    method.ret_type,
                    rock_lib::types::Type::EnumRepresentation { .. }
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(methods.len(), 1, "selected enum producer: {methods:?}");
        let method = methods[0];
        assert!(method.generic_params.is_empty());
        let id = rock_lib::products::ProductDefId::from(method.id);
        assert!(products.link.records.contains_key(&id));
        assert!(!products.bodies.functions.contains_key(&id));
        products.link.object_path = Some("provider.o".into());
        let artifact = root.join("provider.rkca");
        products.write_artifact_to_path(&artifact).unwrap();
        let bytes = std::fs::read(&artifact).unwrap();
        let object = std::fs::read(root.join("provider.o")).unwrap();
        std::fs::remove_file(root.join("provider.rk")).unwrap();
        let source =
            format!("> provider::Service\n> provider::Envelope\n> provider::as_dispatch\n{main}");
        assert!(
            execute(
                &root,
                "enum_virtual_factory_imported",
                &source,
                level,
                Some(&artifact)
            )
            .0 >= 4
        );
        assert_eq!(std::fs::read(&artifact).unwrap(), bytes);
        assert_eq!(std::fs::read(root.join("provider.o")).unwrap(), object);
        assert!(!root.join("provider.rk").exists());
    }
}

#[test]
fn nominal_borrowed_parameter_preserves_owning_field_copy() {
    nominal_storage_boundary(
        "borrowed_nominal_parameter",
        r#"
struct Holder
    < value: () -> I64
holder = seed ->
    Holder
        value: -> seed
extern observe_copy_environment: &(() -> I64) -> I64
extern observe_array_environments: &[(() -> I64); 3] -> I64
inspect: &Holder -> I64
inspect = held ->
    callbacks = [held.value, held.value, held.value]
    observe_array_environments &callbacks
main = ->
    stored = holder 42
    inspect &stored
"#,
        3,
    );
}

#[test]
fn nested_tuple_owning_copies_have_independent_callable_environments() {
    nominal_storage_boundary(
        "nested_tuple_copy",
        r#"
make = seed -> -> seed
extern observe_copy_environment: &(() -> I64) -> I64
extern observe_array_environments: &[(() -> I64); 3] -> I64
main = ->
    callback = make 42
    original = (callback, callback, callback)
    nested = (original, 7)
    copied = nested
    first = [original.0, original.1, original.2]
    second = [(nested.0).0, (nested.0).1, (nested.0).2]
    third = [(copied.0).0, (copied.0).1, (copied.0).2]
    if ~I64Eq (observe_array_environments &first), 84
        if ~I64Eq (observe_array_environments &second), 84
            observe_array_environments &third
        else
            0
    else
        0
"#,
        9,
    );
}

#[test]
fn nominal_user_destructor_and_environment_cleanup_both_run_once() {
    nominal_storage_boundary(
        "nominal_user_destructor",
        r#"
lang drop
< trait Dispose
    lang method
    ~@dispose: ()
struct Holder
    < value: () -> I64
    < drops: *I64
impl Dispose for Holder
    ~@dispose = !->
        unsafe *self.drops = ~I64Add (*self.drops), 1
holder = drops, seed ->
    Holder
        value: -> seed
        drops: drops
holder_pair = drops, pair ->
    Holder
        value: ->
            values = pair
            ~I64Add values.0, values.1
        drops: drops
extern observe_copy_environment: &(() -> I64) -> I64
main = ->
    mut drops = 0
    answer = if true
        stored = holder ((&mut drops) as *I64), 42
        first = observe_copy_environment &stored.value
        other = holder_pair ((&mut drops) as *I64), (20, 22)
        ~I64Add first, (observe_copy_environment &other.value)
    else
        0
    if ~I64Eq drops, 2
        answer
    else
        0
"#,
        2,
    );
}

#[test]
fn nominal_field_replacement_reclaims_old_and_new_environments() {
    nominal_storage_boundary(
        "nominal_field_replacement",
        r#"
struct Holder
    < value: () -> I64
holder = seed ->
    Holder
        value: -> seed
make = seed -> -> seed
extern observe_copy_environment: &(() -> I64) -> I64
main = ->
    mut stored = holder 41
    old = observe_copy_environment &stored.value
    stored.value = make 42
    if ~I64Eq old, 41
        ~I64Add (observe_copy_environment &stored.value), (observe_copy_environment &stored.value)
    else
        0
"#,
        2,
    );
}

#[test]
fn nested_source_returns_preserve_storage_and_real_diagnostic_provenance() {
    for (name, selection) in [
        ("block", "selected = if flag\n        return (make_callback seed)\n    else\n        make_callback seed\n    selected"),
        ("initializer", "selected = if flag then return (make_callback seed) else make_callback seed\n    selected"),
        ("unsafe", "selected = unsafe\n        if flag\n            return (make_callback seed)\n        make_callback seed\n    selected"),
        ("argument", "identity (if flag then return (make_callback seed) else make_callback seed)"),
        ("guard", "match flag\n        true if (if flag then return (make_callback seed) else false) => make_callback seed\n        _ => make_callback seed"),
        ("returned_lambda", "captured = Captured\n        flag: flag\n        seed: seed\n    producer = ->\n        values = captured.take!\n        if values.0\n            return (make_callback values.1)\n        make_callback values.1\n    producer"),
    ] {
        let invocation = if name == "returned_lambda" { "(choose true, 42)!" } else { "choose true, 42" };
        let other = if name == "returned_lambda" { "(choose false, 42)!" } else { "choose false, 42" };
        let source = format!(r#"
extern observe_array_environments: &[(() -> I64); 3] -> I64
make_callback = seed -> -> seed
identity = value -> value
struct Captured
    < flag: Bool
    < seed: I64
trait Take
    ~@take: (Bool, I64)
impl Take for Captured
    ~@take = -> (self.flag, self.seed)
choose = flag, seed ->
    {selection}
main = ->
    first = {invocation}
    second = {other}
    callbacks = [first, second, first]
    observe_array_environments &callbacks
"#);
        if name == "returned_lambda" {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
                .join(".agent-work/fixtures").join(format!("returned_lambda_capture_{}", std::process::id()));
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join("main.rk"), &source).unwrap();
            let analysis = rock_lib::analyze(&config(&root, "main.rk", 0)).unwrap();
            let choose = analysis.hir.program.functions.values().find(|function| function.name == "choose").unwrap();
            let rock_lib::hir::HirStmtFor::Let { value, .. } = &choose.body.stmts[1] else { panic!("producer binding") };
            let rock_lib::hir::HirExprKindFor::Lambda { captures, body, .. } = &value.kind else { panic!("returned lambda") };
            assert!(captures.iter().all(|capture| capture.kind == rock_lib::hir::HirClosureCaptureKind::Move), "{captures:?}\n{body:#?}");
        }
        nominal_storage_boundary(&format!("nested_source_{name}"), &source, 3);
        let negative = if name == "returned_lambda" {
            source.replacen("return (make_callback values.1)", "return (-> 0)", 1)
        } else {
            source.replacen("return (make_callback seed)", "return (-> 0)", 1)
        };
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
            .join(".agent-work/fixtures").join(format!("nested_source_negative_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("main.rk"), &negative).unwrap();
        for level in [0, 3] {
            let errors = rock_lib::compile(&config(&root, "main.rk", level)).unwrap_err();
            let diagnostic = errors.0.iter().find(|error| {
                error.code == Some(rock_lib::diagnostic::DiagnosticCode::Mono)
                    && error.message.contains("agreed field representation")
            }).unwrap_or_else(|| panic!("{name} O{level}: {errors:?}"));
            let span = &diagnostic.primary.as_ref().expect("real returned-value provenance").span;
            assert_eq!(span.file_path, root.join("main.rk"));
            let text = &negative[span.start..span.end];
            assert!(text.contains("make_callback seed") || text.contains("make_callback values.1") || text.contains("-> 0")
                || (text == "->" && span.start == negative.find("return (->").unwrap() + "return (".len()),
                "{name}: {text:?}");
            assert!(!text.contains("main ="), "return diagnostics must not invent caller provenance");
            assert!(errors.0.iter().all(|error| error.code != Some(rock_lib::diagnostic::DiagnosticCode::Internal)), "{errors:?}");
        }
    }
}

#[test]
fn early_exit_user_destructors_keep_loan_reads_order_and_field_responsibility() {
    const TYPES: &str = r#"
lang drop
< trait Dispose
    lang method
    ~@dispose: ()
struct Guard
    < value: &I64
    < log: *I64
impl Dispose for Guard
    ~@dispose = !->
        code = if ~I64Eq (*self.value), 42 then 2 else 9
        unsafe *self.log = ~I64Add (~I64Mul (*self.log), 10), code
struct Owner
    < guard: Guard
    < callback: () -> I64
    < log: *I64
impl Dispose for Owner
    ~@dispose = !->
        code = if ~I64Eq (*self.guard.value), 42 then 1 else 9
        unsafe *self.log = ~I64Add (~I64Mul (*self.log), 10), code
struct Bundle
    < guard: Guard
    < callback: () -> I64
make_callback = seed -> -> seed
extern observe_copy_environment: &(() -> I64) -> I64
"#;
    for (name, body, expected) in [
        ("owner", r#"
run = value, log, flag ->
    owner = Owner
        guard: Guard
            value: value
            log: log
        callback: make_callback *value
        log: log
    result = observe_copy_environment &owner.callback
    if flag
        return result
    result
main = ->
    value = 42
    mut log = 0
    first = run &value, ((&mut log) as *I64), true
    second = run &value, ((&mut log) as *I64), false
    if ~I64Eq log, EXPECTED
        ~I64Add first, second
    else
        0
"#, 1212),
        ("partial", r#"
run = value, log, flag ->
    bundle = Bundle
        guard: Guard
            value: value
            log: log
        callback: make_callback *value
    moved = bundle.callback
    if flag
        return moved
    moved
main = ->
    value = 42
    mut log = 0
    first = run &value, ((&mut log) as *I64), true
    if ~I64Eq log, 2
        second = run &value, ((&mut log) as *I64), false
        if ~I64Eq log, EXPECTED
            ~I64Add (observe_copy_environment &first), (observe_copy_environment &second)
        else
            0
    else
        0
"#, 22),
        ("unused", r#"
run = value, log, flag ->
    owner = Owner
        guard: Guard
            value: value
            log: log
        callback: make_callback *value
        log: log
    observed = observe_copy_environment &owner.callback
    if flag
        return owner
    owner
main = ->
    value = 42
    mut log = 0
    run &value, ((&mut log) as *I64), true
    if ~I64Eq log, 12
        run &value, ((&mut log) as *I64), false
        if ~I64Eq log, EXPECTED
            84
        else
            0
    else
        0
"#, 1212),
    ] {
        let source = format!("{TYPES}\n{}", body.replace("EXPECTED", &expected.to_string()));
        nominal_storage_boundary(&format!("early_drop_{name}"), &source, 2);
    }
    for (name, operation) in [
        ("mutation", "value = 43"),
        ("borrowed_move", "borrowed = &owner\n    moved = (*borrowed).guard"),
    ] {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
            .join(".agent-work/fixtures").join(format!("early_drop_negative_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let source = format!(r#"{TYPES}
main = !->
    mut value = 42
    mut log = 0
    owner = Owner
        guard: Guard
            value: &value
            log: ((&mut log) as *I64)
        callback: make_callback 42
        log: ((&mut log) as *I64)
    {operation}
"#);
        std::fs::write(root.join("main.rk"), source).unwrap();
        for level in [0, 3] {
            let errors = rock_lib::compile(&config(&root, "main.rk", level)).unwrap_err();
            assert!(errors.0.iter().any(|error| error.code == Some(rock_lib::diagnostic::DiagnosticCode::Borrow)
                && error.primary.as_ref().is_some_and(|label| label.span.file_path == root.join("main.rk"))), "{name}: {errors:?}");
            assert!(errors.0.iter().all(|error| error.code != Some(rock_lib::diagnostic::DiagnosticCode::Internal)), "{errors:?}");
        }
    }
}

#[test]
fn callable_array_projection_preserves_owned_environments_and_borrowed_access() {
    nominal_storage_boundary("callable_array_projection", r#"
lang index
< trait Index Idx
    lang output
    type Output
    lang method
    @index: Idx -> &Self::Output
impl Index I64 for [T; 3]
    type Output = T
    @index = i ->
        slice: &[T] = self
        ptr = ~ArrPtr slice
        unsafe &*(~PtrOffset ptr, i)
extern observe_array_environments: &[(() -> I64); 3] -> I64
extern observe_copy_environment: &(() -> I64) -> I64
make = seed -> -> seed
select = flag, seed ->
    callbacks = [make seed; 3]
    observed = observe_array_environments &callbacks
    index = if flag then 0 else 1
    selected = callbacks[index]
    selected
main = ->
    first = select true, 42
    second = select false, 42
    ~I64Add (observe_copy_environment &first), (observe_copy_environment &second)
"#, 8);
}

#[test]
fn callable_registration_and_branch_order_do_not_change_storage_agreement() {
    let mut ids = Vec::new();
    for registration in [false, true] {
        let definitions = if registration {
            "other = seed -> -> seed\nmake = seed -> -> seed"
        } else {
            "make = seed -> -> seed\nother = seed -> -> seed"
        };
        for branches in [false, true] {
            let (early, tail) = if branches { ("other seed", "make seed") } else { ("make seed", "other seed") };
            let source = format!(r#"
extern observe_array_environments: &[(() -> I64); 3] -> I64
{definitions}
choose = flag, seed ->
    if flag
        return ({early})
    {tail}
main = ->
    first = choose true, 42
    second = choose false, 42
    callbacks = [first, second, first]
    observe_array_environments &callbacks
"#);
            let name = format!("registration_{registration}_{branches}");
            nominal_storage_boundary(&name, &source, 3);
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
                .join(".agent-work/fixtures").join(format!("registration_analyze_{}_{}", std::process::id(), name));
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join("main.rk"), &source).unwrap();
            let analysis = rock_lib::analyze(&config(&root, "main.rk", 0)).unwrap();
            ids.push(analysis.hir.program.functions.values().find(|function| function.name == "make").unwrap().id);
            let negative = source.replace("other = seed -> -> seed", "other = seed -> -> ~I64Add seed.0, seed.1");
            // Explicitly different capture storage, without relying on incidental ABI width.
            let negative = negative.replace("other seed", "other (20, 22)");
            std::fs::write(root.join("negative.rk"), negative).unwrap();
            for level in [0, 3] {
                let errors = rock_lib::compile(&config(&root, "negative.rk", level)).unwrap_err();
                assert!(errors.0.iter().any(|error| error.code == Some(rock_lib::diagnostic::DiagnosticCode::Mono)
                    && error.message.contains("agreed field representation")), "{name}: {errors:?}");
                assert!(errors.0.iter().all(|error| error.code != Some(rock_lib::diagnostic::DiagnosticCode::Internal)), "{errors:?}");
            }
        }
    }
    assert_ne!(ids[0], ids[2], "the control must really reorder producer definition identities");
}

#[test]
fn nonreturning_exits_do_not_compete_with_actual_return_storage() {
    nominal_storage_boundary("nonreturning_storage", r#"
extern observe_array_environments: &[(() -> I64); 3] -> I64
make = seed -> -> seed
dead_tail = seed ->
    return (make seed)
    -> 0
never_tail = flag, seed ->
    if flag
        return (make seed)
    loop
        continue
never_branch = flag, seed ->
    if flag
        make seed
    else
        loop
            continue
main = ->
    first = dead_tail 42
    second = never_tail true, 42
    third = never_branch true, 42
    callbacks = [first, second, third]
    observe_array_environments &callbacks
"#, 3);
}

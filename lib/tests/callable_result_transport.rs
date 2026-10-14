use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use rock_lib::diagnostic::DiagnosticCode;
use rock_lib::{products::CompilerProducts, Config};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn scratch(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".agent-work/fixtures")
        .join(format!(
            "callable_result_{name}_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn config(root: &Path, entry: PathBuf, opt_level: u8) -> Config {
    Config {
        entry_file: entry,
        output_dir: root.to_path_buf(),
        no_std: true,
        no_prelude: true,
        opt_level,
        ..Default::default()
    }
}

const CAPTURING: &str = r#"
struct Holder
    < value: &I64
impl Holder
    ~@extract: &I64
    ~@extract = -> self.value
read: &I64 -> I64
read = value -> *value
main = ->
    value = 42
    holder = Holder
        value: &value
    callback = -> holder.extract!
    result = callback!
    read result
"#;

#[test]
fn source_free_raw_result_demands_require_measured_producers_without_scalar_or_escape_authority() {
    let root = scratch("raw_result_input");
    const PROVIDER: &str = r#"
lang fn_mut
< trait Invoke Args, Ret
    lang output
    type Output
    lang method
    ^@invoke: Args -> Ret
< struct Raw
    < pointer: *I64
produce: *I64 -> Raw
< produce = pointer ->
    Raw
        pointer: pointer
visit: F -> A -> (F, R) where F: Invoke A, R
< visit = mut callback, value ->
    result = callback value
    (callback, result)
forward: (A -> R) -> A -> R
< forward = callback, value ->
    visited = visit callback, value
    visited.1
"#;
    const MAIN: &str = r#"
main = ->
    mut value = 42
    pointer = (&mut value) as *I64
    result = forward produce, pointer
    unsafe *result.pointer
"#;
    let entry = root.join("provider.rk");
    std::fs::write(&entry, PROVIDER).unwrap();
    let mut producer = config(&root, entry.clone(), 0);
    producer.current_crate_name = Some("provider".into());
    producer.no_link = true;
    producer.emit_object = Some(root.join("provider.o"));
    let mut products = rock_lib::compile_with_products(&producer)
        .unwrap()
        .products
        .unwrap();
    products.link.object_path = Some("provider.o".into());
    let id = *products
        .interface
        .functions
        .iter()
        .find(|(_, function)| function.name.ends_with("produce"))
        .unwrap()
        .0;
    assert!(products.link.native_return_origins.contains_key(&id));
    let artifact = root.join("provider.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let bytes = std::fs::read(&artifact).unwrap();
    let object = std::fs::read(root.join("provider.o")).unwrap();
    std::fs::remove_file(entry).unwrap();
    let imported = format!("> provider::produce\n> provider::forward\n{MAIN}");
    for level in [0, 3] {
        execute(&root, &format!("{PROVIDER}\n{MAIN}"), level);
        let entry = root.join("imported.rk");
        std::fs::write(&entry, &imported).unwrap();
        let mut caller = config(&root, entry, level);
        caller.extern_artifacts = vec![("provider".into(), artifact.clone())];
        rock_lib::compile(&caller).unwrap();
        assert_eq!(
            std::process::Command::new(root.join("imported"))
                .status()
                .unwrap()
                .code(),
            Some(42)
        );
    }
    assert_eq!(std::fs::read(&artifact).unwrap(), bytes);
    assert_eq!(std::fs::read(root.join("provider.o")).unwrap(), object);
    assert!(products.link.native_return_origins.remove(&id).is_some());
    let missing = root.join("missing.rkca");
    products.write_artifact_to_path(&missing).unwrap();
    for level in [0, 3] {
        let entry = root.join("missing.rk");
        std::fs::write(&entry, &imported).unwrap();
        let mut caller = config(&root, entry, level);
        caller.no_link = true;
        caller.extern_artifacts = vec![("provider".into(), missing.clone())];
        let diagnostics = rock_lib::compile(&caller).unwrap_err();
        assert!(
            diagnostics
                .0
                .iter()
                .any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
                    && diagnostic.message == "Callable result borrowing contract is not satisfied"),
            "{diagnostics:?}"
        );
        assert!(
            diagnostics
                .0
                .iter()
                .all(|diagnostic| diagnostic.code != Some(DiagnosticCode::Internal)),
            "{diagnostics:?}"
        );
    }
    assert!(!root.join("missing").exists());
    assert_eq!(std::fs::read(&artifact).unwrap(), bytes);
    assert_eq!(std::fs::read(root.join("provider.o")).unwrap(), object);
}

#[test]
fn source_free_static_stored_arrow_layouts_keep_distinct_owned_environments() {
    let root = scratch("static_stored_arrow_layout");
    const PROVIDER: &str = r#"
< struct Relay T
    < marker: T
impl Relay T
    relay: (() -> (), T) -> (() -> (), T)
    relay = pair -> pair
"#;
    const SOURCE: &str = r#"
lang drop
< trait Dispose
    lang method
    ~@dispose: ()
struct Guard
    < drops: *I64
impl Dispose for Guard
    ~@dispose = !->
        unsafe *self.drops = ~I64Add (*self.drops), 1
main = ->
    mut drops = 0
    pointer = (&mut drops) as *I64
    if true
        resource = Guard
            drops: pointer
        callback = !->
            moved = resource
        forwarded = Relay::relay (callback, 0)
        ()
    if true
        resource = Guard
            drops: pointer
        other = Guard
            drops: pointer
        callback = !->
            moved = resource
            also_moved = other
        forwarded = Relay::relay (callback, 0)
        ()
    if ~I64Eq drops, 3 then 42 else 1
"#;
    let entry = root.join("provider.rk");
    std::fs::write(&entry, PROVIDER).unwrap();
    let mut producer = config(&root, entry.clone(), 0);
    producer.current_crate_name = Some("provider".into());
    producer.no_link = true;
    producer.emit_object = Some(root.join("provider.o"));
    let mut products = rock_lib::compile_with_products(&producer)
        .unwrap()
        .products
        .unwrap();
    products.link.object_path = Some("provider.o".into());
    let artifact = root.join("provider.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let bytes = std::fs::read(&artifact).unwrap();
    let object = std::fs::read(root.join("provider.o")).unwrap();
    std::fs::remove_file(entry).unwrap();
    for invoked in [false, true] {
        let source = if invoked {
            SOURCE.replace(
                "        ()",
                "        match forwarded\n            (stored, marker) => stored!",
            )
        } else {
            SOURCE.to_owned()
        };
        for level in [0, 3] {
            execute(&root, &format!("{PROVIDER}\n{source}"), level);
            let entry = root.join("imported.rk");
            std::fs::write(&entry, format!("> provider::Relay\n{source}")).unwrap();
            let mut caller = config(&root, entry.clone(), level);
            caller.extern_artifacts = vec![("provider".into(), artifact.clone())];
            rock_lib::compile(&caller).unwrap();
            assert_eq!(
                std::process::Command::new(root.join("imported"))
                    .status()
                    .unwrap()
                    .code(),
                Some(42)
            );
            caller.no_link = true;
            std::fs::write(
                &entry,
                format!(
                    "> provider::Relay\n{}",
                    SOURCE.replace(
                        "        forwarded = Relay::relay (callback, 0)",
                        "        forwarded = Relay::relay (callback, 0)\n        reused = callback",
                    )
                ),
            )
            .unwrap();
            let diagnostics = rock_lib::compile(&caller).unwrap_err();
            assert!(
                diagnostics
                    .0
                    .iter()
                    .any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
                        && diagnostic.message.contains("moved")),
                "{diagnostics:?}"
            );
            assert_eq!(std::fs::read(&artifact).unwrap(), bytes);
            assert_eq!(std::fs::read(root.join("provider.o")).unwrap(), object);
        }
    }
}

#[test]
fn stored_arrow_tuple_layout_transport_preserves_actual_environments_and_live_results() {
    let root = scratch("stored_arrow_layout");
    const SOURCE: &str = r#"
relay: (() -> &I64, T) -> (() -> &I64, T)
relay = pair -> pair
main = ->
    mut value = 42
    reference = &value
    first = -> reference
    one = relay (first, ())
    extra = 7
    second = ->
        observed = extra
        reference
    two = relay (second, ())
    result = two.0!
    *result
"#;
    let provider_entry = root.join("provider.rk");
    std::fs::write(
        &provider_entry,
        "relay: (() -> &I64, T) -> (() -> &I64, T)\n< relay = pair -> pair\n",
    )
    .unwrap();
    let mut producer = config(&root, provider_entry.clone(), 0);
    producer.current_crate_name = Some("provider".into());
    producer.no_link = true;
    producer.emit_object = Some(root.join("provider.o"));
    let mut products = rock_lib::compile_with_products(&producer)
        .unwrap()
        .products
        .unwrap();
    products.link.object_path = Some("provider.o".into());
    assert!(
        products.link.native_return_origins.is_empty(),
        "layout transport is not a measured callable producer"
    );
    let artifact = root.join("provider.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let frozen = std::fs::read(&artifact).unwrap();
    let object = std::fs::read(root.join("provider.o")).unwrap();
    std::fs::remove_file(provider_entry).unwrap();
    for level in [0, 3] {
        execute(&root, SOURCE, level);
        let imported = SOURCE.replace(
            "relay: (() -> &I64, T) -> (() -> &I64, T)\nrelay = pair -> pair",
            "> provider::relay",
        );
        let entry = root.join("imported.rk");
        std::fs::write(&entry, &imported).unwrap();
        let mut caller = config(&root, entry.clone(), level);
        caller.extern_artifacts = vec![("provider".into(), artifact.clone())];
        rock_lib::compile(&caller).unwrap();
        assert_eq!(
            std::process::Command::new(root.join("imported"))
                .status()
                .unwrap()
                .code(),
            Some(42)
        );
        caller.no_link = true;
        std::fs::write(
            &entry,
            imported.replace("    *result", "    value = 43\n    *result"),
        )
        .unwrap();
        let diagnostics = rock_lib::compile(&caller).unwrap_err();
        assert!(
            diagnostics
                .0
                .iter()
                .any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
                    && diagnostic.message == "borrow conflict on this value"),
            "{diagnostics:?}"
        );
        assert_eq!(std::fs::read(&artifact).unwrap(), frozen);
        assert_eq!(std::fs::read(root.join("provider.o")).unwrap(), object);
        // A result demand is not an effect certificate: retaining a borrowed
        // environment across an intervening call stays conservative.
        let exposed = SOURCE.replace("    one = relay (first, ())\n", "").replace(
            "    two = relay (second, ())",
            "    one = relay (first, ())\n    two = relay (second, ())",
        );
        let entry = root.join("exposed.rk");
        std::fs::write(&entry, exposed).unwrap();
        let mut caller = config(&root, entry, level);
        caller.no_link = true;
        let diagnostics = rock_lib::compile(&caller).unwrap_err();
        assert!(
            diagnostics.0.iter().any(|diagnostic| diagnostic.message
                == "Callable result borrowing contract is not satisfied"),
            "{diagnostics:?}"
        );
        let entry = root.join("negative.rk");
        std::fs::write(
            &entry,
            SOURCE.replace("    *result", "    value = 43\n    *result"),
        )
        .unwrap();
        let mut caller = config(&root, entry, level);
        caller.no_link = true;
        let diagnostics = rock_lib::compile(&caller).unwrap_err();
        assert!(
            diagnostics
                .0
                .iter()
                .any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
                    && diagnostic.message == "borrow conflict on this value"),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn source_free_generic_value_transport_keeps_an_immediate_lambda_environment_at_o0_and_o3() {
    let root = scratch("immediate_environment");
    let provider_entry = root.join("provider.rk");
    std::fs::write(&provider_entry, "< forward = value -> value\n").unwrap();
    let mut producer = config(&root, provider_entry.clone(), 0);
    producer.current_crate_name = Some("provider".into());
    producer.no_link = true;
    producer.emit_object = Some(root.join("provider.o"));
    let mut products = rock_lib::compile_with_products(&producer)
        .unwrap()
        .products
        .unwrap();
    products.link.object_path = Some("provider.o".into());
    let id = products
        .interface
        .functions
        .iter()
        .find(|(_, function)| function.name.ends_with("forward"))
        .unwrap()
        .0;
    assert!(products.bodies.functions.contains_key(id));
    assert!(
        products.link.native_return_origins.is_empty(),
        "a generic transport header is not a producer measurement"
    );
    let artifact = root.join("provider.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let bytes = std::fs::read(&artifact).unwrap();
    let object_bytes = std::fs::read(root.join("provider.o")).unwrap();
    std::fs::remove_file(provider_entry).unwrap();
    assert!(CAPTURING.contains("    result = callback!"));
    let source = CAPTURING
        .replace("main = ->", "> provider::forward\nmain = ->")
        .replace(
            "    result = callback!",
            "    forwarded = (input -> forward input) callback\n    result = forwarded!",
        );
    for level in [0, 3] {
        let entry = root.join("main.rk");
        std::fs::write(&entry, &source).unwrap();
        let mut caller = config(&root, entry.clone(), level);
        caller.extern_artifacts = vec![("provider".into(), artifact.clone())];
        rock_lib::compile(&caller).unwrap_or_else(|errors| panic!("{errors:?}"));
        assert_eq!(
            std::process::Command::new(root.join("main"))
                .status()
                .unwrap()
                .code(),
            Some(42)
        );
        let negative = source
            .replace("    value = 42", "    mut value = 42")
            .replace("    read result", "    value = 43\n    read result");
        std::fs::write(&entry, negative).unwrap();
        caller.no_link = true;
        let diagnostics = rock_lib::compile(&caller).unwrap_err();
        assert!(
            diagnostics
                .0
                .iter()
                .any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
                    && diagnostic.message == "borrow conflict on this value"),
            "{diagnostics:?}"
        );
        let repeated = source.replace(
            "    read result",
            "    ignored = forwarded!\n    read result",
        );
        std::fs::write(&entry, repeated).unwrap();
        let diagnostics = rock_lib::compile(&caller).unwrap_err();
        assert!(
            diagnostics
                .0
                .iter()
                .any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
                    && diagnostic.message.contains("moved")),
            "the transported environment is not Copy: {diagnostics:?}"
        );
        assert_eq!(std::fs::read(&artifact).unwrap(), bytes);
        assert_eq!(
            std::fs::read(root.join("provider.o")).unwrap(),
            object_bytes
        );
    }
    const CLEANUP: &str = r#"
> provider::forward
lang drop
< trait Dispose
    lang method
    ~@dispose: ()
struct Guard
    < drops: *I64
impl Dispose for Guard
    ~@dispose = !->
        unsafe *self.drops = ~I64Add (*self.drops), 1
main = ->
    mut drops = 0
    pointer = (&mut drops) as *I64
    if true
        resource = Guard
            drops: pointer
        callback: () -> Guard = -> resource
        forwarded: () -> Guard = (input -> forward input) callback
        ()
    if ~I64Eq drops, 1 then 42 else 1
"#;
    for invoked in [false, true] {
        let source = if invoked {
            CLEANUP.replace("        ()", "        moved = forwarded!\n        ()")
        } else {
            CLEANUP.to_owned()
        };
        for level in [0, 3] {
            let entry = root.join("main.rk");
            std::fs::write(&entry, &source).unwrap();
            let mut caller = config(&root, entry, level);
            caller.extern_artifacts = vec![("provider".into(), artifact.clone())];
            rock_lib::compile(&caller).unwrap_or_else(|errors| panic!("{errors:?}"));
            assert_eq!(std::process::Command::new(root.join("main")).status().unwrap().code(), Some(42), "both an unused environment and an invoked/moved payload drop exactly once at block exit: invoked={invoked} O{level} root={root:?}");
            assert_eq!(std::fs::read(&artifact).unwrap(), bytes);
            assert_eq!(
                std::fs::read(root.join("provider.o")).unwrap(),
                object_bytes
            );
        }
    }
}

fn execute(root: &Path, source: &str, opt_level: u8) {
    let entry = root.join("main.rk");
    std::fs::write(&entry, source).unwrap();
    rock_lib::compile(&config(root, entry, opt_level)).unwrap();
    let output = std::process::Command::new(root.join("main"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(42), "{output:?}");
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
}

#[test]
fn stored_consuming_callable_keeps_payload_loans_and_rejects_second_use() {
    let root = scratch("stored_consuming_once");
    let source = CAPTURING
        .replace("main = ->", "struct Stored F\n    < callback: F\nmain = ->")
        .replace("result = callback!", "stored = Stored\n        callback: callback\n    result = stored.callback!");
    for level in [0, 3] {
        execute(&root, &source, level);
        for (name, negative) in [
            ("twice", source.replace("    read result", "    again = stored.callback!\n    read result")),
            ("mutation", source.replace("value = 42", "mut value = 42")
                .replace("    read result", "    value = 43\n    read result")),
        ] {
            let entry = root.join(format!("{name}_o{level}.rk"));
            std::fs::write(&entry, negative).unwrap();
            let mut caller = config(&root, entry.clone(), level);
            caller.no_link = true;
            let errors = rock_lib::compile(&caller).expect_err("stored ownership and loans remain authoritative");
            assert!(errors.0.iter().any(|error| error.code == Some(DiagnosticCode::Borrow)
                && error.primary.as_ref().is_some_and(|label| label.span.file_path == entry)), "{name}: {errors:?}");
            assert!(errors.0.iter().all(|error| error.code != Some(DiagnosticCode::Internal)), "{errors:?}");
            assert!(!root.join(format!("{name}_o{level}")).exists());
        }
    }
}

#[test]
fn early_return_reference_alternatives_keep_both_input_loans() {
    let root = scratch("early_return_reference_alternatives");
    const SOURCE: &str = r#"
choose = flag, left, right ->
    if flag
        return left
    right
main = ->
    mut left = 42
    mut right = 42
    result = choose false, &left, &right
    *result
"#;
    for level in [0, 3] {
        execute(&root, SOURCE, level);
        for name in ["left", "right"] {
            let entry = root.join(format!("{name}_o{level}.rk"));
            std::fs::write(&entry, SOURCE.replace("    *result", &format!("    {name} = 43\n    *result"))).unwrap();
            let mut caller = config(&root, entry.clone(), level);
            caller.no_link = true;
            let errors = rock_lib::compile(&caller).expect_err("literal arguments do not erase a reachable loan alternative");
            assert!(errors.0.iter().any(|error| error.code == Some(DiagnosticCode::Borrow)
                && error.message == "borrow conflict on this value"
                && error.primary.as_ref().is_some_and(|label| label.span.file_path == entry)), "{errors:?}");
            assert!(errors.0.iter().all(|error| error.code != Some(DiagnosticCode::Internal)), "{errors:?}");
        }
        let entry = root.join(format!("dead_alternative_o{level}.rk"));
        let negative = SOURCE.replace("    right\nmain", "    local = 42\n    &local\nmain");
        std::fs::write(&entry, negative).unwrap();
        let mut caller = config(&root, entry.clone(), level);
        caller.no_link = true;
        let errors = rock_lib::compile(&caller).unwrap_err();
        assert!(errors.0.iter().any(|error| error.code == Some(DiagnosticCode::Borrow)
            && error.message.contains("does not live long enough")
            && error.primary.as_ref().is_some_and(|label| label.span.file_path == entry)), "{errors:?}");
        assert!(errors.0.iter().all(|error| error.code != Some(DiagnosticCode::Internal)), "{errors:?}");
    }
}

#[test]
fn callable_environment_cleanup_keeps_owned_capture_destructor_loans_live() {
    let root = scratch("environment_drop_loans");
    const SOURCE: &str = r#"
lang drop
< trait Dispose
    lang method
    ~@dispose: ()
struct Guard
    < value: &I64
impl Dispose for Guard
    ~@dispose = !->
        observed = *self.value
forward = value -> value
main = !->
    mut value = 42
    resource = Guard
        value: &value
    callback = -> resource
    forwarded = (input -> forward input) callback
    value = 43
"#;
    for level in [0, 3] {
        let entry = root.join("main.rk");
        std::fs::write(&entry, SOURCE).unwrap();
        let mut caller = config(&root, entry, level);
        caller.no_link = true;
        let diagnostics = rock_lib::compile(&caller).unwrap_err();
        assert!(
            diagnostics
                .0
                .iter()
                .any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
                    && diagnostic.message == "borrow conflict on this value"),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn native_capturing_result_keeps_payload_live_after_consuming_the_callable_at_o0_and_o3() {
    let root = scratch("capturing");
    for level in [0, 3] {
        execute(&root, CAPTURING, level);
    }
}

#[test]
fn shared_capturing_result_survives_consumption_of_the_holder_at_o0_and_o3() {
    let root = scratch("shared");
    let source = CAPTURING
        .replace("~@extract", "@extract")
        .replace(
            "read: &I64 -> I64",
            "impl Holder\n    ~@consume: &I64\n    ~@consume = -> self.value\nread: &I64 -> I64",
        )
        .replace("    read result", "    holder.consume!\n    read result");
    for level in [0, 3] {
        execute(&root, &source, level);
    }
}

#[test]
fn concrete_returned_and_nested_callables_compose_at_o0_and_o3() {
    let root = scratch("composition");
    for source in [
        CAPTURING.replace("main = ->", "make: &I64 -> (() -> &I64)\nmake = value ->\n    holder = Holder\n        value: value\n    -> holder.extract!\nmain = ->")
            .replace("callback = -> holder.extract!", "callback = make &value"),
        CAPTURING.replace("main = ->", "relay: Holder -> &I64\nrelay = holder ->\n    inner = -> holder.extract!\n    inner!\nmain = ->")
            .replace("callback = -> holder.extract!", "callback = -> relay holder"),
    ] {
        for level in [0, 3] { execute(&root, &source, level); }
    }
}

#[test]
fn generic_fn_once_local_input_keeps_measured_capture_loans_at_o0_and_o3() {
    let root = scratch("generic_fn_once_input");
    let source = CAPTURING
        .replace(
            "main = ->",
            r#"lang fn_once
< trait Consume Args, Ret
    lang output
    type Output
    lang method
    ~@consume: Args -> Ret
invoke: F -> T where F: Consume (), T
invoke = callback -> callback!
main = ->"#,
        )
        .replace("result = callback!", "result = invoke callback");
    for level in [0, 3] {
        execute(&root, &source, level);
        let negative = source
            .replace("value = 42", "mut value = 42")
            .replace("    read result", "    value = 43\n    read result");
        let entry = root.join(format!("negative{level}.rk"));
        std::fs::write(&entry, negative).unwrap();
        let mut caller = config(&root, entry, level);
        caller.no_link = true;
        let diagnostics = rock_lib::compile(&caller).unwrap_err();
        assert!(
            diagnostics
                .0
                .iter()
                .any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
                    && diagnostic.message == "borrow conflict on this value"
                    && diagnostic.primary.is_some()),
            "{diagnostics:?}"
        );
        assert!(
            diagnostics
                .0
                .iter()
                .all(|diagnostic| diagnostic.code != Some(DiagnosticCode::Internal)),
            "{diagnostics:?}"
        );
        assert!(!root.join(format!("negative{level}")).exists());
    }
}

#[test]
fn composed_definition_inputs_discharge_each_producer_and_keep_payload_loans_at_o0_and_o3() {
    let root = scratch("composed_inputs");
    for wrapper in [
        "compose = outer, inner -> outer (inner!)\n",
        "compose = outer, inner ->\n    first = inner\n    second = outer\n    value = first!\n    result = second value\n    result\n",
        "compose: (T -> U) -> (() -> T) -> U\ncompose = outer, inner ->\n    first = inner\n    second = outer\n    value = first!\n    second value\n",
    ] {
      let source = CAPTURING
        .replace("main = ->", &format!("{wrapper}identity = value -> value\nmain = ->"))
        .replace("result = callback!", "result = compose identity, callback");
      for level in [0, 3] {
        execute(&root, &source, level);
        let negative = source.replace("value = 42", "mut value = 42")
            .replace("    read result", "    value = 43\n    read result");
        let entry = root.join(format!("negative{level}.rk"));
        std::fs::write(&entry, negative).unwrap();
        let mut caller = config(&root, entry, level);
        caller.no_link = true;
        let diagnostics = rock_lib::compile(&caller).unwrap_err();
        assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
            && diagnostic.message == "borrow conflict on this value" && diagnostic.primary.is_some()), "{diagnostics:?}");
        assert!(diagnostics.0.iter().all(|diagnostic| diagnostic.code != Some(DiagnosticCode::Internal)), "{diagnostics:?}");
        assert!(!root.join(format!("negative{level}")).exists());
      }
    }
}

#[test]
fn signatureless_callable_input_discharges_measured_local_capturing_producers_at_o0_and_o3() {
    let root = scratch("local_input");
    let source = CAPTURING
        .replace("main = ->", "invoke = callback -> callback!\nmain = ->")
        .replace("result = callback!", "result = invoke callback");
    for level in [0, 3] {
        execute(&root, &source, level);
        let negative = source
            .replace("value = 42", "mut value = 42")
            .replace("    read result", "    value = 43\n    read result");
        let entry = root.join(format!("negative{level}.rk"));
        std::fs::write(&entry, negative).unwrap();
        let mut caller = config(&root, entry, level);
        caller.no_link = true;
        let diagnostics = rock_lib::compile(&caller).unwrap_err();
        assert!(
            diagnostics
                .0
                .iter()
                .any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
                    && diagnostic.message == "borrow conflict on this value"
                    && diagnostic.primary.is_some()),
            "{diagnostics:?}"
        );
        assert!(!root.join(format!("negative{level}")).exists());
    }
}

#[test]
fn definition_owned_demands_follow_immutable_input_and_result_aliases_at_o0_and_o3() {
    let root = scratch("input_aliases");
    for wrapper in [
        "invoke = callback ->\n    alias = callback\n    alias!\n",
        "invoke = callback ->\n    alias = callback\n    result = alias!\n    result\n",
        "invoke: (() -> T) -> T\ninvoke = callback ->\n    alias = callback\n    result = alias!\n    result\n",
    ] {
        let source = CAPTURING.replace("main = ->", &format!("{wrapper}main = ->"))
            .replace("result = callback!", "result = invoke callback");
        for level in [0, 3] {
            execute(&root, &source, level);
            let negative = source.replace("value = 42", "mut value = 42")
                .replace("    read result", "    value = 43\n    read result");
            let entry = root.join(format!("negative{level}.rk"));
            std::fs::write(&entry, negative).unwrap();
            let mut caller = config(&root, entry, level);
            caller.no_link = true;
            let diagnostics = rock_lib::compile(&caller).unwrap_err();
            assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
                && diagnostic.message == "borrow conflict on this value" && diagnostic.primary.is_some()), "{diagnostics:?}");
            assert!(!root.join(format!("negative{level}")).exists());
        }
    }
}

#[test]
fn definition_use_demands_transport_aggregate_results_after_incidental_calls_at_o0_and_o3() {
    let root = scratch("definition_use_aggregate");
    let source = CAPTURING
        .replace("result = callback!", "result = invoke callback")
        .replace(
            "main = ->",
            r#"
struct Wrapped T
    < result: T
incidental = !-> ()
invoke = callback ->
    incidental!
    result = callback!
    Wrapped
        result: (result, result)
main = ->"#,
        )
        .replace("    read result", "    read result.result.0");
    for level in [0, 3] {
        execute(&root, &source, level);
        let negative = source.replace("value = 42", "mut value = 42").replace(
            "    read result.result.0",
            "    value = 43\n    read result.result.0",
        );
        let entry = root.join(format!("negative{level}.rk"));
        std::fs::write(&entry, negative).unwrap();
        let mut caller = config(&root, entry, level);
        caller.no_link = true;
        let diagnostics = rock_lib::compile(&caller).unwrap_err();
        assert!(
            diagnostics
                .0
                .iter()
                .any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
                    && diagnostic.message == "borrow conflict on this value"
                    && diagnostic.primary.is_some()),
            "{diagnostics:?}"
        );
        assert!(!root.join(format!("negative{level}")).exists());
    }
    let entry = root.join("provider.rk");
    let provider = source
        .split("main = ->")
        .next()
        .unwrap()
        .replace("struct Wrapped T", "< struct Wrapped T")
        .replace("invoke = callback ->", "< invoke = callback ->");
    std::fs::write(&entry, provider).unwrap();
    let mut producer = config(&root, entry.clone(), 0);
    producer.current_crate_name = Some("provider".into());
    producer.no_link = true;
    producer.emit_object = Some(root.join("provider.o"));
    let mut products = rock_lib::compile_with_products(&producer)
        .unwrap()
        .products
        .unwrap();
    let (invoke, declaration) = products
        .interface
        .functions
        .iter()
        .find(|(_, function)| function.name.ends_with("invoke"))
        .unwrap();
    assert!(
        products.bodies.functions.contains_key(invoke),
        "the universal definition must remain available without its source"
    );
    assert!(matches!(
        &declaration.params[0],
        rock_lib::types::Type::Function {
            callable_kind: rock_lib::types::CallableKind::FnOnce,
            result_borrowing: rock_lib::types::CallableResultBorrowing::Sources(_),
            ..
        }
    ));
    products.link.object_path = Some("provider.o".into());
    let artifact = root.join("provider.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let frozen_artifact = std::fs::read(&artifact).unwrap();
    let frozen_object = std::fs::read(root.join("provider.o")).unwrap();
    std::fs::remove_file(entry).unwrap();
    let consumer = format!(
        "> provider::invoke\n{}",
        CAPTURING
            .replace("result = callback!", "result = invoke callback")
            .replace("    read result", "    read result.result.0")
    );
    for level in [0, 3] {
        let entry = root.join("main.rk");
        std::fs::write(&entry, &consumer).unwrap();
        let mut caller = config(&root, entry, level);
        caller.extern_artifacts = vec![("provider".into(), artifact.clone())];
        rock_lib::compile(&caller).unwrap();
        let output = std::process::Command::new(root.join("main"))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(42), "{output:?}");
        assert!(output.stdout.is_empty() && output.stderr.is_empty());
        let negative = consumer.replace("value = 42", "mut value = 42").replace(
            "    read result.result.0",
            "    value = 43\n    read result.result.0",
        );
        caller.entry_file = root.join(format!("import_negative{level}.rk"));
        std::fs::write(&caller.entry_file, negative).unwrap();
        caller.no_link = true;
        let diagnostics = rock_lib::compile(&caller).unwrap_err();
        assert!(
            diagnostics
                .0
                .iter()
                .any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
                    && diagnostic.message == "borrow conflict on this value"
                    && diagnostic.primary.is_some()),
            "{diagnostics:?}"
        );
        assert!(!root.join(format!("import_negative{level}")).exists());
        assert!(!root.join("provider.rk").exists());
        assert_eq!(std::fs::read(&artifact).unwrap(), frozen_artifact);
        assert_eq!(
            std::fs::read(root.join("provider.o")).unwrap(),
            frozen_object
        );
    }
}

#[test]
fn signatureless_argument_bearing_callable_input_discharges_native_identity_at_o0_and_o3() {
    let root = scratch("argument_input");
    let source = "identity = value -> value\ninvoke = callback, value -> callback value\nread: &I64 -> I64\nread = value -> *value\nmain = ->\n    value = 42\n    result = invoke identity, &value\n    read result\n";
    for level in [0, 3] {
        execute(&root, source, level);
        let negative = source
            .replace("value = 42", "mut value = 42")
            .replace("    read result", "    value = 43\n    read result");
        let entry = root.join(format!("negative{level}.rk"));
        std::fs::write(&entry, negative).unwrap();
        let mut caller = config(&root, entry, level);
        caller.no_link = true;
        let diagnostics = rock_lib::compile(&caller).unwrap_err();
        assert!(
            diagnostics
                .0
                .iter()
                .any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
                    && diagnostic.message == "borrow conflict on this value"
                    && diagnostic.primary.is_some()),
            "{diagnostics:?}"
        );
        assert!(!root.join(format!("negative{level}")).exists());
    }
}

#[test]
fn explicit_generic_operator_discharges_native_and_local_argument_producers_at_o0_and_o3() {
    let root = scratch("generic_operator");
    let prefix = "infix 1 |>>\n|>>: T -> (T -> U) -> U\n|>> = value, callback -> callback value\nread: &I64 -> I64\nread = value -> *value\n";
    for callback in ["identity", "value -> value"] {
        let source = format!("{prefix}identity = value -> value\nmain = ->\n    value = 42\n    result = &value |>> ({callback})\n    read result\n");
        for level in [0, 3] {
            execute(&root, &source, level);
            let negative = source
                .replace("value = 42", "mut value = 42")
                .replace("    read result", "    value = 43\n    read result");
            let entry = root.join(format!("negative{level}.rk"));
            std::fs::write(&entry, negative).unwrap();
            let mut caller = config(&root, entry, level);
            caller.no_link = true;
            let diagnostics = rock_lib::compile(&caller).unwrap_err();
            assert!(
                diagnostics
                    .0
                    .iter()
                    .any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
                        && diagnostic.message == "borrow conflict on this value"
                        && diagnostic.primary.is_some()),
                "{diagnostics:?}"
            );
            assert!(!root.join(format!("negative{level}")).exists());
        }
    }
}

#[test]
fn source_free_scalar_opaque_dependencies_require_measured_native_evidence_at_o0_and_o3() {
    use rock_lib::hir::borrowing::NativeReturnSource;
    let root = scratch("scalar_opaque_input");
    let entry = root.join("provider.rk");
    std::fs::write(
        &entry,
        "extern abs: I32 -> I32\nchecked: I32 -> I32\n< checked = value -> abs value\n",
    )
    .unwrap();
    let mut producer = config(&root, entry.clone(), 0);
    producer.current_crate_name = Some("provider".into());
    producer.no_link = true;
    producer.emit_object = Some(root.join("provider.o"));
    let mut products = rock_lib::compile_with_products(&producer)
        .unwrap()
        .products
        .unwrap();
    let id = *products
        .interface
        .functions
        .iter()
        .find(|(_, function)| function.name.ends_with("checked"))
        .unwrap()
        .0;
    let origins = products
        .link
        .native_return_origins
        .get(&id)
        .expect("a measured opaque scalar body must retain its real portable dependencies");
    assert_eq!(
        origins.sources,
        vec![
            NativeReturnSource::ArgumentValue(0),
            NativeReturnSource::UnknownExternal
        ]
    );
    products.link.object_path = Some("provider.o".into());
    let artifact = root.join("provider.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let frozen = std::fs::read(&artifact).unwrap();
    let object = std::fs::read(root.join("provider.o")).unwrap();
    let mut incompatible = frozen.clone();
    incompatible[8..12].copy_from_slice(&58u32.to_le_bytes());
    assert_eq!(
        CompilerProducts::from_artifact_bytes(&incompatible).unwrap_err(),
        "Unsupported product artifact format 58 (expected 60)",
        "schema disagreement must reject before decoding producer evidence"
    );
    assert_eq!(
        CompilerProducts::read_artifact_from_path(&artifact)
            .unwrap()
            .link
            .native_return_origins,
        products.link.native_return_origins
    );
    std::fs::remove_file(entry).unwrap();
    let mut missing = products.clone();
    missing.link.native_return_origins.remove(&id).unwrap();
    let missing_artifact = root.join("missing.rkca");
    missing.write_artifact_to_path(&missing_artifact).unwrap();
    let source = "> provider::checked\ninvoke: (T -> U) -> T -> U\ninvoke = callback, value -> callback value\nmain = -> invoke checked, 42\n";
    for level in [0, 3] {
        let entry = root.join("main.rk");
        std::fs::write(&entry, source).unwrap();
        let mut caller = config(&root, entry, level);
        caller.extern_artifacts = vec![("provider".into(), artifact.clone())];
        rock_lib::compile(&caller).unwrap();
        let output = std::process::Command::new(root.join("main"))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(42), "{output:?}");
        assert!(output.stdout.is_empty() && output.stderr.is_empty());
        for (name, dependency, negative) in [
            ("missing", missing_artifact.clone(), source.to_string()),
            (
                "foreign",
                artifact.clone(),
                source
                    .replace("> provider::checked", "extern abs: I32 -> I32")
                    .replace("invoke checked", "invoke abs"),
            ),
        ] {
            let entry = root.join(format!("{name}{level}.rk"));
            std::fs::write(&entry, negative).unwrap();
            caller.entry_file = entry;
            caller.no_link = true;
            caller.extern_artifacts = vec![("provider".into(), dependency)];
            let diagnostics = rock_lib::compile(&caller).unwrap_err();
            assert!(
                diagnostics.0.iter().any(|diagnostic| diagnostic.code
                    == Some(DiagnosticCode::Borrow)
                    && diagnostic.message == "Callable result borrowing contract is not satisfied"
                    && diagnostic.primary.is_some()),
                "{diagnostics:?}"
            );
            assert!(
                diagnostics
                    .0
                    .iter()
                    .all(|diagnostic| diagnostic.code != Some(DiagnosticCode::Internal)),
                "{diagnostics:?}"
            );
            assert!(!root.join(format!("{name}{level}")).exists());
        }
        assert_eq!(std::fs::read(&artifact).unwrap(), frozen);
        assert_eq!(std::fs::read(root.join("provider.o")).unwrap(), object);
        assert!(!root.join("provider.rk").exists());
    }
}

#[test]
fn source_free_scalar_identity_requires_real_native_producer_evidence_at_o0_and_o3() {
    use rock_lib::hir::borrowing::NativeReturnSource;
    let root = scratch("scalar_input");
    let entry = root.join("provider.rk");
    std::fs::write(
        &entry,
        "identity: I64 -> I64\n< identity = value -> value\n",
    )
    .unwrap();
    let mut producer = config(&root, entry.clone(), 0);
    producer.current_crate_name = Some("provider".into());
    producer.no_link = true;
    producer.emit_object = Some(root.join("provider.o"));
    let mut products = rock_lib::compile_with_products(&producer)
        .unwrap()
        .products
        .unwrap();
    assert_eq!(products.link.native_return_origins.len(), 1);
    assert_eq!(
        products
            .link
            .native_return_origins
            .values()
            .next()
            .unwrap()
            .sources,
        vec![NativeReturnSource::ArgumentValue(0)],
        "the checked copy carries its dependency, not fabricated independence"
    );
    products.link.object_path = Some("provider.o".into());
    let artifact = root.join("provider.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let artifact_bytes = std::fs::read(&artifact).unwrap();
    let object_bytes = std::fs::read(root.join("provider.o")).unwrap();
    std::fs::remove_file(entry).unwrap();
    let mut missing = products.clone();
    missing.link.native_return_origins.clear();
    let missing_artifact = root.join("missing.rkca");
    missing.write_artifact_to_path(&missing_artifact).unwrap();
    let source = "> provider::identity\ninfix 1 |>>\n|>>: T -> (T -> U) -> U\n|>> = value, callback -> callback value\nmain = -> 42 |>> identity\n";
    for level in [0, 3] {
        let entry = root.join("main.rk");
        std::fs::write(&entry, source).unwrap();
        let mut caller = config(&root, entry, level);
        caller.extern_artifacts = vec![("provider".into(), artifact.clone())];
        rock_lib::compile(&caller).unwrap();
        let output = std::process::Command::new(root.join("main"))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(42));
        assert!(output.stdout.is_empty() && output.stderr.is_empty());
        caller.entry_file = root.join(format!("negative{level}.rk"));
        std::fs::write(&caller.entry_file, source).unwrap();
        caller.no_link = true;
        caller.extern_artifacts = vec![("provider".into(), missing_artifact.clone())];
        let diagnostics = rock_lib::compile(&caller).unwrap_err();
        assert!(
            diagnostics
                .0
                .iter()
                .any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
                    && diagnostic.message == "Callable result borrowing contract is not satisfied"
                    && diagnostic.primary.as_ref().is_some_and(|label| &source
                        [label.span.start..label.span.end]
                        == "identity")),
            "{diagnostics:?}"
        );
        assert!(!root.join(format!("negative{level}")).exists());
        assert!(!root.join("provider.rk").exists());
        assert_eq!(std::fs::read(&artifact).unwrap(), artifact_bytes);
        assert_eq!(
            std::fs::read(root.join("provider.o")).unwrap(),
            object_bytes
        );
    }
}

#[test]
fn source_free_unique_adapter_result_keeps_permission_and_producer_checks_at_o0_and_o3() {
    let root = scratch("unique_adapter");
    let entry = root.join("provider.rk");
    std::fs::write(
        &entry,
        "identity: &mut I64 -> &mut I64\n< identity = value -> value\n",
    )
    .unwrap();
    let mut producer = config(&root, entry.clone(), 0);
    producer.current_crate_name = Some("provider".into());
    producer.no_link = true;
    producer.emit_object = Some(root.join("provider.o"));
    let mut products = rock_lib::compile_with_products(&producer)
        .unwrap()
        .products
        .unwrap();
    assert_eq!(products.link.native_return_origins.len(), 1);
    let identity = products
        .interface
        .functions
        .iter()
        .find(|(_, function)| function.name.ends_with("identity"))
        .unwrap()
        .0;
    assert!(
        !products.bodies.functions.contains_key(identity),
        "the consumer must use the bodyless native producer record"
    );
    products.link.object_path = Some("provider.o".into());
    let artifact = root.join("provider.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let frozen_artifact = std::fs::read(&artifact).unwrap();
    let frozen_object = std::fs::read(root.join("provider.o")).unwrap();
    std::fs::remove_file(entry).unwrap();
    let mut missing = products.clone();
    missing.link.native_return_origins.clear();
    let missing_artifact = root.join("missing.rkca");
    missing.write_artifact_to_path(&missing_artifact).unwrap();

    let source = r#"
> provider::identity
lang fn_once
< trait Consume Args, Ret
    lang output
    type Output
    lang method
    ~@consume: Args -> Ret
invoke: F -> &mut I64 -> &mut I64 where F: Consume &mut I64, &mut I64
invoke = callback, value -> callback value
main = ->
    mut value = 0
    adapter = inner, item, choose ->
        if choose then invoke inner, item
        else invoke inner, item
    selected = adapter identity, &mut value, true
    ()
    *selected = 42
    *selected
"#;
    for level in [0, 3] {
        let entry = root.join("main.rk");
        std::fs::write(&entry, source).unwrap();
        let mut caller = config(&root, entry, level);
        caller.extern_artifacts = vec![("provider".into(), artifact.clone())];
        rock_lib::compile(&caller).unwrap();
        let output = std::process::Command::new(root.join("main"))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(42), "{output:?}");
        assert!(output.stdout.is_empty() && output.stderr.is_empty());
        for (name, dependency, source, message) in [
            (
                "live_input",
                artifact.clone(),
                source.replace("    *selected = 42", "    value = 1\n    *selected = 42"),
                "borrow conflict on this value",
            ),
            (
                "missing",
                missing_artifact.clone(),
                source.to_string(),
                "Callable result borrowing contract is not satisfied",
            ),
        ] {
            caller.entry_file = root.join(format!("{name}{level}.rk"));
            std::fs::write(&caller.entry_file, &source).unwrap();
            caller.no_link = true;
            caller.extern_artifacts = vec![("provider".into(), dependency)];
            let diagnostics = rock_lib::compile(&caller).unwrap_err();
            assert!(
                diagnostics
                    .0
                    .iter()
                    .any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
                        && diagnostic.message == message
                        && diagnostic.primary.is_some()),
                "{diagnostics:?}"
            );
            assert!(
                diagnostics
                    .0
                    .iter()
                    .all(|diagnostic| diagnostic.code != Some(DiagnosticCode::Internal)),
                "{diagnostics:?}"
            );
            assert!(!root.join(format!("{name}{level}")).exists());
        }
        assert!(!root.join("provider.rk").exists());
        assert_eq!(std::fs::read(&artifact).unwrap(), frozen_artifact);
        assert_eq!(
            std::fs::read(root.join("provider.o")).unwrap(),
            frozen_object
        );
    }
}

#[test]
fn source_free_native_function_alternatives_use_real_producer_contracts_at_o0_and_o3() {
    let root = scratch("source_free");
    let entry = root.join("provider.rk");
    let provider = r#"
identity: &I64 -> &I64
< identity = value -> value
alternate: &I64 -> &I64
< alternate = value -> value
"#;
    std::fs::write(&entry, provider).unwrap();
    let mut producer_config = config(&root, entry.clone(), 0);
    producer_config.current_crate_name = Some("provider".into());
    producer_config.no_link = true;
    producer_config.emit_object = Some(root.join("provider.o"));
    let mut products = rock_lib::compile_with_products(&producer_config)
        .unwrap()
        .products
        .unwrap();
    products.link.object_path = Some("provider.o".into());
    let artifact = root.join("provider.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let artifact_bytes = std::fs::read(&artifact).unwrap();
    let object_bytes = std::fs::read(root.join("provider.o")).unwrap();
    std::fs::remove_file(entry).unwrap();
    let decoded = CompilerProducts::read_artifact_from_path(&artifact).unwrap();
    assert_eq!(decoded.link.object_path, Some("provider.o".into()));
    let source = r#"
> provider::identity
> provider::alternate
read: &I64 -> I64
read = value -> *value
main = ->
    value = 42
    input = &value
    callback = if true then identity else alternate
    result = callback input
    read result
"#;
    for level in [0, 3] {
        let entry = root.join("main.rk");
        std::fs::write(&entry, source).unwrap();
        let mut caller = config(&root, entry, level);
        caller.extern_artifacts = vec![("provider".into(), artifact.clone())];
        rock_lib::compile(&caller).unwrap();
        let output = std::process::Command::new(root.join("main"))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(42), "{output:?}");
        assert!(output.stdout.is_empty() && output.stderr.is_empty());
        assert!(!root.join("provider.rk").exists());
        assert_eq!(std::fs::read(&artifact).unwrap(), artifact_bytes);
        assert_eq!(
            std::fs::read(root.join("provider.o")).unwrap(),
            object_bytes
        );
        let negative = source
            .replace("value = 42", "mut value = 42")
            .replace("    read result", "    value = 43\n    read result");
        let entry = root.join("negative.rk");
        std::fs::write(&entry, negative).unwrap();
        caller.entry_file = entry;
        caller.no_link = true;
        let diagnostics = rock_lib::compile(&caller).unwrap_err();
        assert!(
            diagnostics
                .0
                .iter()
                .any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
                    && diagnostic.message == "borrow conflict on this value"
                    && diagnostic.primary.is_some()),
            "{diagnostics:?}"
        );
        assert!(
            !root.join("negative").exists(),
            "negative must never be linked or executed"
        );
        assert_eq!(std::fs::read(&artifact).unwrap(), artifact_bytes);
        assert_eq!(
            std::fs::read(root.join("provider.o")).unwrap(),
            object_bytes
        );
    }
}

#[test]
fn capturing_payload_mutation_and_environment_storage_results_still_reject() {
    let root = scratch("negatives");
    let storage = r#"
struct Holder
    < value: I64
impl Holder
    ~@consume: I64
    ~@consume = -> self.value
main = !->
    holder = Holder
        value: 42
    callback = ->
        result = &holder.value
        holder.consume!
        result
    result = callback!
"#;
    for (index, source) in [
        CAPTURING
            .replace("value = 42", "mut value = 42")
            .replace("    read result", "    value = 43\n    read result"),
        CAPTURING
            .replace("~@extract", "@extract")
            .replace("value = 42", "mut value = 42")
            .replace("    read result", "    value = 43\n    read result"),
        storage.into(),
        CAPTURING.replace("main = ->", "relay: Holder -> &I64\nrelay = holder ->\n    inner = -> holder.extract!\n    inner!\nmain = ->")
            .replace("callback = -> holder.extract!", "callback = -> relay holder")
            .replace("value = 42", "mut value = 42")
            .replace("    read result", "    value = 43\n    read result"),
    ]
    .into_iter()
    .enumerate()
    {
        let entry = root.join(format!("negative{index}.rk"));
        std::fs::write(&entry, source).unwrap();
        let mut compiler = config(&root, entry, 0);
        compiler.no_link = true;
        let diagnostics = rock_lib::compile(&compiler).unwrap_err();
        assert!(
            diagnostics
                .0
                .iter()
                .any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
                    && diagnostic.message == "borrow conflict on this value"
                    && diagnostic.primary.is_some()),
            "{diagnostics:?}"
        );
        if index == 2 {
            assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.message
                == "Cannot return reference to local value because it does not live long enough"
                && diagnostic.primary.is_some()));
        }
        assert!(!root.join(format!("negative{index}")).exists());
    }
}

#[test]
fn source_free_capturing_factory_exports_measured_invocation_sources_at_o0_and_o3() {
    source_free_capturing_factory(false, false, false);
}

#[test]
fn source_free_capturing_factory_transports_through_a_universal_callable_input_at_o0_and_o3() {
    source_free_capturing_factory(true, false, false);
}

#[test]
fn source_free_capturing_factory_transports_through_a_separately_emitted_universal_adapter_at_o0_and_o3(
) {
    source_free_capturing_factory(true, true, false);
}

#[test]
fn source_free_capturing_factory_composes_through_an_independently_emitted_adapter_at_o0_and_o3() {
    source_free_capturing_factory(true, true, true);
}

fn source_free_capturing_factory(through_input: bool, external_input: bool, composed: bool) {
    use rock_lib::types::{CallableResultBorrowing, CallableResultSource, Type};

    let root = scratch("capturing_factory");
    let entry = root.join("provider.rk");
    let mut provider = format!(
        "{}\nmake: &I64 -> (() -> &I64)\n< make = value ->\n    holder = Holder\n        value: value\n    -> holder.extract!\n",
        CAPTURING.split("main = ->").next().unwrap(),
    );
    if composed {
        provider.push_str("identity: &I64 -> &I64\n< identity = value -> value\n");
    }
    std::fs::write(&entry, provider).unwrap();
    let mut producer = config(&root, entry.clone(), 0);
    producer.current_crate_name = Some("provider".into());
    producer.no_link = true;
    producer.emit_object = Some(root.join("provider.o"));
    let mut products = rock_lib::compile_with_products(&producer)
        .unwrap()
        .products
        .unwrap();
    products.link.object_path = Some("provider.o".into());
    let (id, factory) = products
        .interface
        .functions
        .iter()
        .find(|(_, function)| function.name.ends_with("make"))
        .unwrap();
    let id = *id;
    let Type::Function {
        result_borrowing, ..
    } = &factory.ret_type
    else {
        panic!("factory result");
    };
    assert_eq!(
        result_borrowing,
        &CallableResultBorrowing::Sources(Box::new(vec![CallableResultSource::EnvironmentValue]))
    );
    assert_eq!(
        products.link.native_return_origins[&id].ret,
        factory.ret_type
    );
    assert!(!products.bodies.functions.contains_key(&id));
    let mut malformed = products.clone();
    malformed
        .link
        .native_return_origins
        .get_mut(&id)
        .unwrap()
        .sources = vec![rock_lib::hir::borrowing::NativeReturnSource::UnknownExternal];
    assert!(malformed.to_artifact_bytes().is_err(),
        "a scalar external record cannot certify a returned callable, even with its checked signature");
    let artifact = root.join("provider.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let artifact_bytes = std::fs::read(&artifact).unwrap();
    let object_bytes = std::fs::read(root.join("provider.o")).unwrap();
    std::fs::remove_file(entry).unwrap();

    let mut dependencies = vec![("provider".into(), artifact.clone())];
    let mut frozen_adapter = Vec::new();
    if external_input {
        let entry = root.join("adapter.rk");
        std::fs::write(&entry, if composed { "< invoke = outer, inner ->\n    alias = inner\n    mapped = alias!\n    outer mapped\n" } else { "< invoke = callback -> callback!\n" }).unwrap();
        let mut producer = config(&root, entry.clone(), 0);
        producer.current_crate_name = Some("adapter".into());
        producer.no_link = true;
        producer.emit_object = Some(root.join("adapter.o"));
        let mut adapter = rock_lib::compile_with_products(&producer)
            .unwrap()
            .products
            .unwrap();
        adapter.link.object_path = Some("adapter.o".into());
        let artifact = root.join("adapter.rkca");
        adapter.write_artifact_to_path(&artifact).unwrap();
        let decoded = CompilerProducts::read_artifact_from_path(&artifact).unwrap();
        let (id, invoke) = decoded
            .interface
            .functions
            .iter()
            .find(|(_, function)| function.name.ends_with("invoke"))
            .unwrap();
        assert_eq!(invoke.generic_params.len(), if composed { 2 } else { 1 });
        assert!(
            decoded.bodies.functions.contains_key(id),
            "independently emitted generic body, not a concrete caller substitute"
        );
        assert!(
            decoded.link.native_return_origins.is_empty(),
            "adapter requirement is not a measured producer certificate"
        );
        for path in [&artifact, &root.join("adapter.o")] {
            frozen_adapter.push((path.to_path_buf(), std::fs::read(path).unwrap()));
        }
        dependencies.push(("adapter".into(), artifact));
        std::fs::remove_file(entry).unwrap();
    }

    let source = "\n> provider::make\nread: &I64 -> I64\nread = value -> *value\nmain = ->\n    value = 42\n    callback = make &value\n    result = callback!\n    read result\n";
    let source = if through_input {
        source
            .replace(
                "main = ->",
                if external_input {
                    "> adapter::invoke\nmain = ->"
                } else {
                    "invoke = callback -> callback!\nmain = ->"
                },
            )
            .replace("result = callback!", "result = invoke callback")
    } else {
        source.to_string()
    };
    let source = if composed {
        source
            .replace("> provider::make", "> provider::make\n> provider::identity")
            .replace(
                "result = invoke callback",
                "result = invoke identity, callback",
            )
    } else {
        source
    };
    for level in [0, 3] {
        let entry = root.join("main.rk");
        std::fs::write(&entry, &source).unwrap();
        let mut caller = config(&root, entry, level);
        caller.extern_artifacts = dependencies.clone();
        if through_input && level == 0 {
            let analysis = rock_lib::analyze(&caller).unwrap();
            let (_, _, function) = analysis
                .hir
                .program
                .functions_by_id()
                .find(|(_, name, _)| name.ends_with("invoke"))
                .unwrap();
            assert!(
                matches!(function.ret_type, Type::Generic(_)),
                "definition must remain result-polymorphic, not specialize to the caller's &I64"
            );
            assert_eq!(function.generic_params.len(), if composed { 2 } else { 1 });
            let Type::Function {
                result_borrowing,
                ret,
                ..
            } = &function.params[0].ty
            else {
                panic!("structural callable input");
            };
            assert_eq!(**ret, function.ret_type);
            let mut sources = vec![
                CallableResultSource::EnvironmentValue,
                CallableResultSource::EnvironmentContents,
                CallableResultSource::Static,
                CallableResultSource::Unknown,
            ];
            if composed {
                sources.insert(0, CallableResultSource::ArgumentValue(0));
            }
            assert_eq!(result_borrowing, &CallableResultBorrowing::Sources(Box::new(sources)), "definition demand must preserve conservative dependencies, not producer independence");
        }
        rock_lib::compile(&caller).unwrap();
        let output = std::process::Command::new(root.join("main"))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(42), "{output:?}");
        assert!(output.stdout.is_empty() && output.stderr.is_empty());

        let negative = source
            .replace("value = 42", "mut value = 42")
            .replace("    read result", "    value = 43\n    read result");
        let entry = root.join(format!("negative{level}.rk"));
        std::fs::write(&entry, negative).unwrap();
        caller.entry_file = entry;
        caller.no_link = true;
        let diagnostics = rock_lib::compile(&caller).unwrap_err();
        assert!(
            diagnostics
                .0
                .iter()
                .any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
                    && diagnostic.message == "borrow conflict on this value"
                    && diagnostic.primary.is_some()),
            "{diagnostics:?}"
        );
        assert!(!root.join(format!("negative{level}")).exists());
        assert!(!root.join("provider.rk").exists());
        assert_eq!(std::fs::read(&artifact).unwrap(), artifact_bytes);
        assert_eq!(
            std::fs::read(root.join("provider.o")).unwrap(),
            object_bytes
        );
        for (path, bytes) in &frozen_adapter {
            assert_eq!(std::fs::read(path).unwrap(), *bytes);
        }
        assert!(!root.join("adapter.rk").exists());
    }

    // Keep the identical callable type promise and object, but remove its
    // producer certificate. A portable header alone must not authorize a call.
    let mut uncertified = products.clone();
    uncertified.link.native_return_origins.remove(&id);
    let missing_artifact = root.join("missing-authority.rkca");
    uncertified
        .write_artifact_to_path(&missing_artifact)
        .unwrap();
    let entry = root.join("missing-authority.rk");
    std::fs::write(&entry, &source).unwrap();
    dependencies[0].1 = missing_artifact;
    for level in [0, 3] {
        let mut caller = config(&root, entry.clone(), level);
        caller.no_link = true;
        caller.extern_artifacts = dependencies.clone();
        let diagnostics = rock_lib::compile(&caller).unwrap_err();
        assert!(
            diagnostics
                .0
                .iter()
                .any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
                    && diagnostic.message == "Callable result borrowing contract is not satisfied"
                    && diagnostic.primary.is_some()),
            "{diagnostics:?}"
        );
        assert!(diagnostics
            .0
            .iter()
            .all(|diagnostic| diagnostic.code != Some(DiagnosticCode::Internal)));
    }
    assert!(!root.join("missing-authority").exists());
    assert_eq!(std::fs::read(&artifact).unwrap(), artifact_bytes);
    assert_eq!(
        std::fs::read(root.join("provider.o")).unwrap(),
        object_bytes
    );
    for (path, bytes) in &frozen_adapter {
        assert_eq!(std::fs::read(path).unwrap(), *bytes);
    }
    if composed {
        let identity = products
            .interface
            .functions
            .iter()
            .find(|(_, function)| function.name.ends_with("identity"))
            .unwrap()
            .0;
        let mut uncertified = products.clone();
        assert!(uncertified
            .link
            .native_return_origins
            .remove(identity)
            .is_some());
        let missing = root.join("missing-outer-authority.rkca");
        uncertified.write_artifact_to_path(&missing).unwrap();
        dependencies[0].1 = missing;
        let entry = root.join("missing-outer-authority.rk");
        std::fs::write(&entry, &source).unwrap();
        for level in [0, 3] {
            let mut caller = config(&root, entry.clone(), level);
            caller.no_link = true;
            caller.extern_artifacts = dependencies.clone();
            let diagnostics = rock_lib::compile(&caller).unwrap_err();
            assert!(
                diagnostics.0.iter().any(|diagnostic| diagnostic.code
                    == Some(DiagnosticCode::Borrow)
                    && diagnostic.message == "Callable result borrowing contract is not satisfied"
                    && diagnostic.primary.as_ref().is_some_and(|label| &source
                        [label.span.start..label.span.end]
                        == "identity")),
                "{diagnostics:?}"
            );
            assert!(diagnostics
                .0
                .iter()
                .all(|diagnostic| diagnostic.code != Some(DiagnosticCode::Internal)));
        }
        assert!(!root.join("missing-outer-authority").exists());
        assert_eq!(std::fs::read(&artifact).unwrap(), artifact_bytes);
        assert_eq!(
            std::fs::read(root.join("provider.o")).unwrap(),
            object_bytes
        );
        for (path, bytes) in &frozen_adapter {
            assert_eq!(std::fs::read(path).unwrap(), *bytes);
        }
    }
}

#[test]
fn capturing_factory_does_not_flatten_distinct_raw_lifetime_and_value_roles() {
    use rock_lib::types::{CallableResultBorrowing, Type};

    let root = scratch("raw_factory");
    let entry = root.join("provider.rk");
    let source = format!(
        "{}\nmake: &I64 -> (() -> *I64)\n< make = value ->\n    holder = Holder\n        value: value\n    -> holder.extract! as *I64\n",
        CAPTURING.split("main = ->").next().unwrap(),
    );
    std::fs::write(&entry, source).unwrap();
    let mut producer = config(&root, entry, 0);
    producer.current_crate_name = Some("provider".into());
    producer.no_link = true;
    producer.emit_object = Some(root.join("provider.o"));
    let products = rock_lib::compile_with_products(&producer)
        .unwrap()
        .products
        .unwrap();
    let (id, factory) = products
        .interface
        .functions
        .iter()
        .find(|(_, function)| function.name.ends_with("make"))
        .unwrap();
    let Type::Function {
        result_borrowing, ..
    } = &factory.ret_type
    else {
        panic!("factory result");
    };
    assert_eq!(result_borrowing, &CallableResultBorrowing::Unknown);
    assert_eq!(
        products.link.native_return_origins[id].ret,
        factory.ret_type
    );
}

#[test]
fn source_free_record_field_writes_preserve_callable_payload_loans_at_o0_and_o3() {
    let root = scratch("record_field_write");
    let provider_entry = root.join("provider.rk");
    std::fs::write(
        &provider_entry,
        r#"
< struct Envelope F
    < callback: F
    < tag: I64
retag: Envelope F -> Bool -> Envelope F
< retag = mut envelope, choose ->
    if choose then envelope.tag = 7
    else envelope.tag = 8
    mut repeat = choose
    while repeat
        envelope.tag = 9
        repeat = false
    envelope
"#,
    )
    .unwrap();
    let mut producer = config(&root, provider_entry.clone(), 0);
    producer.current_crate_name = Some("provider".into());
    producer.no_link = true;
    producer.emit_object = Some(root.join("provider.o"));
    let mut products = rock_lib::compile_with_products(&producer)
        .unwrap()
        .products
        .unwrap();
    products.link.object_path = Some("provider.o".into());
    let artifact = root.join("provider.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let artifact_bytes = std::fs::read(&artifact).unwrap();
    let object_bytes = std::fs::read(root.join("provider.o")).unwrap();
    let decoded = CompilerProducts::read_artifact_from_path(&artifact).unwrap();
    let (id, retag) = decoded
        .interface
        .functions
        .iter()
        .find(|(_, function)| function.name.ends_with("retag"))
        .unwrap();
    assert_eq!(retag.generic_params.len(), 1);
    assert!(
        decoded.bodies.functions.contains_key(id),
        "retain the definition, not a consumer-specific body"
    );
    assert!(
        decoded.link.native_return_origins.is_empty(),
        "generic record transport is not invocation authority"
    );
    std::fs::remove_file(&provider_entry).unwrap();

    let source = CAPTURING
        .replace(
            "main = ->",
            r#"
> provider::Envelope
> provider::retag
lang fn_once
< trait Consume Args, Ret
    lang output
    type Output
    lang method
    ~@consume: Args -> Ret
apply: Envelope F -> &I64 where F: Consume (), &I64
apply = envelope -> envelope.callback!
main = ->"#,
        )
        .replace(
            "result = callback!",
            r#"envelope = Envelope
        callback: callback
        tag: 0
    changed = retag envelope, true
    result = apply changed"#,
        );
    for level in [0, 3] {
        let entry = root.join("main.rk");
        std::fs::write(&entry, &source).unwrap();
        let mut caller = config(&root, entry, level);
        caller.extern_artifacts = vec![("provider".into(), artifact.clone())];
        rock_lib::compile(&caller).unwrap();
        let output = std::process::Command::new(root.join("main"))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(42), "{output:?}");
        assert!(output.stdout.is_empty() && output.stderr.is_empty());

        let negative = source
            .replace("value = 42", "mut value = 42")
            .replace("    read result", "    value = 43\n    read result");
        let entry = root.join(format!("negative{level}.rk"));
        std::fs::write(&entry, negative).unwrap();
        caller.entry_file = entry;
        caller.no_link = true;
        let diagnostics = rock_lib::compile(&caller).unwrap_err();
        assert!(
            diagnostics
                .0
                .iter()
                .any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
                    && diagnostic.message == "borrow conflict on this value"
                    && diagnostic.primary.is_some()),
            "{diagnostics:?}"
        );
        assert!(
            diagnostics
                .0
                .iter()
                .all(|diagnostic| diagnostic.code != Some(DiagnosticCode::Internal)),
            "{diagnostics:?}"
        );
        assert!(!root.join(format!("negative{level}")).exists());
        assert!(!provider_entry.exists());
        assert_eq!(std::fs::read(&artifact).unwrap(), artifact_bytes);
        assert_eq!(
            std::fs::read(root.join("provider.o")).unwrap(),
            object_bytes
        );
    }
}

#[test]
fn source_free_factory_through_record_and_captured_relay_at_o0_and_o3() {
    let root = scratch("factory_record_captured_relay");
    let provider_entry = root.join("provider.rk");
    std::fs::write(
        &provider_entry,
        r#"
< struct Envelope F
    < callback: F
    < tag: I64
lang fn_once
< trait Consume Args, Ret
    lang output
    type Output
    lang method
    ~@consume: Args -> Ret
read: &I64 -> I64
read = value -> *value
relay: Envelope F -> &I64 -> &I64 where F: Consume (), &I64
< relay = envelope, fallback ->
    adapter: Envelope F -> &I64 = current ->
        selected = current.callback!
        read fallback
        selected
    adapter envelope
retag: Envelope F -> Envelope F
< retag = mut envelope ->
    envelope.tag = 7
    envelope
struct Holder
    < value: &I64
impl Holder
    ~@extract: &I64
    ~@extract = -> self.value
make: &I64 -> (() -> &I64)
< make = value ->
    holder = Holder
        value: value
    -> holder.extract!
"#,
    )
    .unwrap();
    let mut producer = config(&root, provider_entry.clone(), 0);
    producer.current_crate_name = Some("provider".into());
    producer.no_link = true;
    producer.emit_object = Some(root.join("provider.o"));
    let mut products = rock_lib::compile_with_products(&producer)
        .unwrap()
        .products
        .unwrap();
    products.link.object_path = Some("provider.o".into());
    let artifact = root.join("provider.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let artifact_bytes = std::fs::read(&artifact).unwrap();
    let object_bytes = std::fs::read(root.join("provider.o")).unwrap();
    let decoded = CompilerProducts::read_artifact_from_path(&artifact).unwrap();
    let make = decoded
        .interface
        .functions
        .iter()
        .find(|(_, function)| function.name.ends_with("make"))
        .unwrap()
        .0;
    let relay = decoded
        .interface
        .functions
        .iter()
        .find(|(_, function)| function.name.ends_with("relay"))
        .unwrap()
        .0;
    assert!(
        !decoded.bodies.functions.contains_key(make),
        "factory authority must be bodyless at the consumer"
    );
    assert!(decoded.link.native_return_origins.contains_key(make));
    assert!(
        decoded.bodies.functions.contains_key(relay),
        "the callable relay retains its canonical generic definition"
    );
    std::fs::remove_file(&provider_entry).unwrap();
    const SOURCE: &str = r#"
> provider::Envelope
> provider::make
> provider::retag
> provider::relay
read: &I64 -> I64
read = value -> *value
main = ->
    value = 42
    fallback = 7
    callback = make &value
    envelope = Envelope
        callback: callback
        tag: 0
    changed = retag envelope
    result = relay changed, &fallback
    read result
"#;
    for level in [0, 3] {
        let entry = root.join("main.rk");
        std::fs::write(&entry, SOURCE).unwrap();
        let mut caller = config(&root, entry, level);
        caller.extern_artifacts = vec![("provider".into(), artifact.clone())];
        rock_lib::compile(&caller).unwrap();
        let output = std::process::Command::new(root.join("main"))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(42), "{output:?}");
        assert!(output.stdout.is_empty() && output.stderr.is_empty());
        let negative = SOURCE
            .replace("value = 42", "mut value = 42")
            .replace("    read result", "    value = 43\n    read result");
        let entry = root.join(format!("negative{level}.rk"));
        std::fs::write(&entry, negative).unwrap();
        caller.entry_file = entry;
        caller.no_link = true;
        let diagnostics = rock_lib::compile(&caller).unwrap_err();
        assert!(
            diagnostics
                .0
                .iter()
                .any(|diagnostic| diagnostic.code == Some(DiagnosticCode::Borrow)
                    && diagnostic.message == "borrow conflict on this value"
                    && diagnostic.primary.is_some()),
            "{diagnostics:?}"
        );
        assert!(
            diagnostics
                .0
                .iter()
                .all(|diagnostic| diagnostic.code != Some(DiagnosticCode::Internal)),
            "{diagnostics:?}"
        );
        assert!(!root.join(format!("negative{level}")).exists());
        assert!(!provider_entry.exists());
        assert_eq!(std::fs::read(&artifact).unwrap(), artifact_bytes);
        assert_eq!(
            std::fs::read(root.join("provider.o")).unwrap(),
            object_bytes
        );
    }
}

#[test]
fn branch_and_loop_callable_replacements_validate_every_carried_producer() {
    let root = scratch("replaced_callable_join");
    for (name, replacement) in [
        ("branches", "if flag\n        envelope.callback = identity\n    else\n        envelope.callback = alternate"),
        ("one_branch", "if flag\n        envelope.callback = alternate"),
        ("loop", "mut repeat = flag\n    while repeat\n        envelope.callback = alternate\n        repeat = false"),
    ] {
        let source = format!(r#"
struct Envelope F
    < callback: F
identity: &I64 -> &I64
identity = value -> value
alternate: &I64 -> &I64
alternate = value -> value
extern opaque: &I64 -> &I64
replace = mut envelope, flag ->
    {replacement}
    envelope
invoke = envelope, value -> envelope.callback value
main = ->
    mut value = 42
    initial = Envelope
        callback: identity
    first = replace initial, true
    second = replace first, false
    result = invoke second, &value
    *result
"#);
        for level in [0, 3] {
            execute(&root, &source, level);
            for (kind, negative) in [
                ("missing", source.replace("envelope.callback = alternate", "envelope.callback = opaque")),
                ("mutation", source.replace("    *result", "    value = 43\n    *result")),
            ] {
                let entry = root.join(format!("{name}_{kind}_{level}.rk"));
                std::fs::write(&entry, negative).unwrap();
                let mut caller = config(&root, entry.clone(), level);
                caller.no_link = true;
                let errors = rock_lib::compile(&caller).expect_err("all applicable writes and returned loans remain authoritative");
                assert!(errors.0.iter().any(|error| error.code == Some(DiagnosticCode::Borrow)
                    && error.primary.as_ref().is_some_and(|label| label.span.file_path == entry)), "{name} {kind}: {errors:?}");
                assert!(errors.0.iter().all(|error| error.code != Some(DiagnosticCode::Internal)), "{errors:?}");
                assert!(!entry.with_extension("").exists());
            }
        }
    }
}

#[test]
fn source_free_callable_joins_survive_actual_import_id_renumbering() {
    let root = scratch("import_id_callable_join");
    let provider = r#"
< struct Envelope F
    < callback: F
identity: &I64 -> &I64
< identity = value -> value
alternate: &I64 -> &I64
< alternate = value -> value
< select = flag, left, right ->
    if flag
        return left
    right
< invoke = callback, value -> callback value
"#;
    for (name, source) in [("provider", provider), ("decoy", "< struct Marker\n    < value: I64\n")] {
        let entry = root.join(format!("{name}.rk"));
        std::fs::write(&entry, source).unwrap();
        let mut producer = config(&root, entry.clone(), 0);
        producer.current_crate_name = Some(name.into());
        producer.no_link = true;
        producer.emit_object = Some(root.join(format!("{name}.o")));
        let mut products = rock_lib::compile_with_products(&producer).unwrap().products.unwrap();
        products.link.object_path = Some(format!("{name}.o").into());
        if name == "provider" {
            for function in ["identity", "alternate"] {
                let id = products.interface.functions.iter().find(|(_, f)| f.name.ends_with(function)).unwrap().0;
                assert!(!products.bodies.functions.contains_key(id));
                assert!(products.link.native_return_origins.contains_key(id));
                let mut missing = products.clone();
                missing.link.native_return_origins.remove(id).unwrap();
                missing.write_artifact_to_path(&root.join(format!("missing_{function}.rkca"))).unwrap();
            }
        }
        products.write_artifact_to_path(&root.join(format!("{name}.rkca"))).unwrap();
        std::fs::remove_file(entry).unwrap();
    }
    let frozen = ["provider.rkca", "provider.o", "decoy.rkca", "decoy.o"]
        .map(|name| (name, std::fs::read(root.join(name)).unwrap()));
    let source = r#"
> provider::Envelope
> provider::identity
> provider::alternate
> provider::select
> provider::invoke
main = ->
    mut value = 42
    first = Envelope
        callback: identity
    second = Envelope
        callback: alternate
    envelope = select false, first, second
    result = invoke envelope.callback, &value
    *result
"#;
    let mut ids = Vec::new();
    for reverse in [false, true] {
        for level in [0, 3] {
            let entry = root.join(format!("caller_{reverse}_{level}.rk"));
            std::fs::write(&entry, source).unwrap();
            let mut caller = config(&root, entry.clone(), level);
            let names = if reverse { ["decoy", "provider"] } else { ["provider", "decoy"] };
            caller.extern_artifacts = names.map(|name| (name.into(), root.join(format!("{name}.rkca")))).into();
            let analysis = rock_lib::analyze(&caller).unwrap();
            ids.push(analysis.hir.program.names.structs_by_name["provider::Envelope"].crate_id);
            rock_lib::compile(&caller).unwrap();
            let output = std::process::Command::new(entry.with_extension("")).output().unwrap();
            assert_eq!(output.status.code(), Some(42), "{output:?}");
            let negative = root.join(format!("negative_{reverse}_{level}.rk"));
            std::fs::write(&negative, source.replace("    *result", "    value = 43\n    *result")).unwrap();
            caller.entry_file = negative.clone();
            caller.no_link = true;
            let errors = rock_lib::compile(&caller).unwrap_err();
            assert!(errors.0.iter().any(|error| error.code == Some(DiagnosticCode::Borrow)
                && error.message == "borrow conflict on this value"
                && error.primary.as_ref().is_some_and(|label| label.span.file_path == negative)), "{errors:?}");
            assert!(errors.0.iter().all(|error| error.code != Some(DiagnosticCode::Internal)), "{errors:?}");
            for function in ["identity", "alternate"] {
                let uncertified = root.join(format!("missing_{function}_{reverse}_{level}.rk"));
                std::fs::write(&uncertified, source).unwrap();
                caller.entry_file = uncertified.clone();
                caller.extern_artifacts.iter_mut().find(|(name, _)| name == "provider").unwrap().1
                    = root.join(format!("missing_{function}.rkca"));
                let errors = rock_lib::compile(&caller).unwrap_err();
                assert!(errors.0.iter().any(|error| error.code == Some(DiagnosticCode::Borrow)
                    && error.message == "Callable result borrowing contract is not satisfied"
                    && error.primary.as_ref().is_some_and(|label| label.span.file_path == uncertified)), "{errors:?}");
                assert!(errors.0.iter().all(|error| error.code != Some(DiagnosticCode::Internal)), "{errors:?}");
                assert!(!uncertified.with_extension("").exists());
            }
            for (name, bytes) in &frozen { assert_eq!(std::fs::read(root.join(name)).unwrap(), *bytes); }
        }
    }
    assert_ne!(ids[0], ids[2], "the control must really renumber imported runtime identities");
    assert!(!root.join("provider.rk").exists() && !root.join("decoy.rk").exists());
}

#[test]
fn callable_array_access_retains_the_selected_result_loan() {
    let root = scratch("array_access_loan");
    let source = r#"
lang index
< trait Index Idx
    lang output
    type Output
    lang method
    @index: Idx -> &Self::Output
impl Index I64 for [T; 2]
    type Output = T
    @index = i ->
        slice: &[T] = self
        ptr = ~ArrPtr slice
        unsafe &*(~PtrOffset ptr, i)
identity: &I64 -> &I64
identity = value -> value
alternate: &I64 -> &I64
alternate = value -> value
main = ->
    mut value = 42
    callbacks = [identity, alternate]
    result = callbacks[1] &value
    *result
"#;
    for level in [0, 3] {
        execute(&root, source, level);
        let entry = root.join(format!("negative{level}.rk"));
        std::fs::write(&entry, source.replace("    *result", "    value = 43\n    *result")).unwrap();
        let mut caller = config(&root, entry.clone(), level);
        caller.no_link = true;
        let errors = rock_lib::compile(&caller).unwrap_err();
        assert!(errors.0.iter().any(|error| error.code == Some(DiagnosticCode::Borrow)
            && error.message == "borrow conflict on this value"
            && error.primary.as_ref().is_some_and(|label| label.span.file_path == entry)), "{errors:?}");
        assert!(errors.0.iter().all(|error| error.code != Some(DiagnosticCode::Internal)), "{errors:?}");
    }
}

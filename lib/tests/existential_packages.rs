use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use rock_lib::diagnostic::DiagnosticCode;
use rock_lib::{Config, SourceProvider};

static NEXT: AtomicUsize = AtomicUsize::new(0);

const REFERENCES: &str = r#"
trait Accept
    @accept: &Self -> I64
struct Number
    < value: I64
impl Accept for Number
    @accept = other -> ~I64Add self.value, other.value
type Pair = exists T where T: Accept in (&T, &T)
"#;

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join("opencode").join(format!(
            "rock_existential_packages_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn config(&self, source: &str) -> Config {
        let entry = self.0.join("main.rk");
        Config {
            entry_file: entry.clone(),
            output_dir: self.0.clone(),
            current_crate_name: Some("existential_packages".into()),
            no_std: true,
            no_prelude: true,
            emit_llvm: true,
            source_providers: vec![SourceProvider::Virtual {
                path: entry,
                text: source.into(),
            }],
            ..Config::default()
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

fn execute(root: &Path, expected: i32) {
    let output = Command::new(root.join("main")).output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(expected),
        "{}: status {:?}, stderr {}",
        root.display(),
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty(), "{output:?}");
}

fn run(source: &str, expected: i32) {
    let scratch = Scratch::new();
    for opt_level in [0, 3] {
        let mut config = scratch.config(source);
        config.opt_level = opt_level;
        rock_lib::compile(&config).unwrap_or_else(|errors| {
            panic!("O{opt_level}, fixture {}: {errors:?}", scratch.0.display())
        });
        execute(&scratch.0, expected);
    }
}

fn reject(source: &str, code: DiagnosticCode, messages: &[&str]) {
    let scratch = Scratch::new();
    let config = scratch.config(source);
    let errors = rock_lib::compile(&config).expect_err("invalid package must be rejected");
    assert!(
        errors.0.iter().any(|error| {
            error.code == Some(code)
                && messages.iter().any(|message| {
                    error
                        .message
                        .to_ascii_lowercase()
                        .contains(&message.to_ascii_lowercase())
                })
                && error
                    .primary
                    .as_ref()
                    .is_some_and(|primary| primary.span.file_path == config.entry_file)
        }),
        "{errors:?}"
    );
    assert!(
        errors
            .0
            .iter()
            .all(|error| error.code != Some(DiagnosticCode::Internal)),
        "{errors:?}"
    );
}

#[test]
fn reference_package_restores_one_self_for_both_payload_positions() {
    run(
        &format!(
            "{REFERENCES}{}",
            r#"
consume: Pair -> I64
consume = package ->
    open package as Hidden, pair
        left = pair.0
        right = pair.1
        left.accept right
main = ->
    left = Number
        value: 20
    right = Number
        value: 22
    package = pack (Pair) ((&left, &right))
    consume package
"#
        ),
        42,
    );
}

#[test]
fn reference_package_can_return_parameter_borrows_and_repack_an_opened_witness() {
    run(
        &format!(
            "{REFERENCES}{}",
            r#"
make: &Number -> &Number -> Pair
make = left, right -> pack (Pair) ((left, right))
repack: Pair -> Pair
repack = package ->
    open package as Hidden, pair
        pack (Pair) (pair)
consume: Pair -> I64
consume = package ->
    open package as Hidden, pair
        pair.0.accept pair.1
main = ->
    left = Number
        value: 20
    right = Number
        value: 22
    consume (repack (make &left, &right))
"#
        ),
        42,
    );
}

#[test]
fn reference_package_alpha_renaming_does_not_change_its_type() {
    run(
        &format!(
            "{REFERENCES}{}",
            r#"
type Renamed = exists Other where Other: Accept in (&Other, &Other)
relay: Pair -> Renamed
relay = package -> package
main = ->
    left = Number
        value: 20
    right = Number
        value: 22
    package = relay (pack (Pair) ((&left, &right)))
    open package as Hidden, pair
        pair.0.accept pair.1
"#
        ),
        42,
    );
}

#[test]
fn reference_package_can_reassemble_a_drop_free_payload_with_the_opened_witness() {
    run(
        &format!(
            "{REFERENCES}{}",
            r#"
reverse: Pair -> Pair
reverse = package ->
    open package as Hidden, pair
        pack (Pair) ((pair.1, pair.0))
consume: Pair -> I64
consume = package ->
    open package as Hidden, pair
        pair.0.accept pair.1
main = ->
    left = Number
        value: 20
    right = Number
        value: 22
    consume (reverse (pack (Pair) ((&left, &right))))
"#
        ),
        42,
    );
}

#[test]
fn package_branch_selection_keeps_payload_and_witness_metadata_together() {
    run(
        &format!(
            "{REFERENCES}{}",
            r#"
struct Other
    < ignored: I64
    < left: I64
    < right: I64
impl Accept for Other
    @accept = other -> ~I64Add self.left, other.right
choose: Bool -> Pair -> Pair -> Pair
choose = condition, first, second ->
    if condition
        first
    else
        second
consume: Pair -> I64
consume = package ->
    open package as Hidden, pair
        pair.0.accept pair.1
main = ->
    number_left = Number
        value: 20
    number_right = Number
        value: 1
    other_left = Other
        ignored: 0
        left: 2
        right: 0
    other_right = Other
        ignored: 0
        left: 0
        right: 19
    first = consume (choose true, (pack (Pair) ((&number_left, &number_right))), (pack (Pair) ((&other_left, &other_right))))
    second = consume (choose false, (pack (Pair) ((&number_left, &number_right))), (pack (Pair) ((&other_left, &other_right))))
    ~I64Add first, second
"#
        ),
        42,
    );
}

#[test]
fn mutable_reference_package_preserves_exclusive_access() {
    run(
        r#"
trait Change
    ^@change: I64 -> ()
    @read: I64
struct Number
    < value: I64
impl Change for Number
    ^@change = value !-> self.value = value
    @read = -> self.value
type Exclusive = exists T where T: Change in &mut T
main = ->
    mut number = Number
        value: 0
    package = pack (Exclusive) (&mut number)
    open package as Hidden, value
        value.change 42
        value.read!
"#,
        42,
    );
}

#[test]
fn shared_reference_package_does_not_allocate_payload_storage() {
    let scratch = Scratch::new();
    rock_lib::compile(&scratch.config(&format!(
        "{REFERENCES}{}",
        r#"
main = ->
    left = Number
        value: 20
    right = Number
        value: 22
    package = pack (Pair) ((&left, &right))
    open package as Hidden, pair
        pair.0.accept pair.1
"#
    )))
    .unwrap();
    execute(&scratch.0, 42);
    let llvm = fs::read_to_string(scratch.0.join("main.ll")).unwrap();
    for allocator in ["@malloc(", "@calloc(", "@realloc("] {
        assert!(
            !llvm.contains(allocator),
            "packing a reference pair must not introduce an allocator: {allocator}"
        );
    }
}

#[test]
fn whole_owned_payload_repacking_transfers_exactly_once_drop_responsibility() {
    let source = r#"
extern putchar: I32 -> I32
lang drop
< trait Dispose
    lang method
    ~@dispose: ()
trait Accept
    @accept: &Self -> I64
struct Number
impl Accept for Number
    @accept = other -> 0
struct Indirect T
    < pointer: *T
    < marker: I32
impl Dispose for Indirect T
    ~@dispose = !-> putchar self.marker
type Owned = exists T where T: Accept in Indirect T
repack: Owned -> Owned
repack = package ->
    open package as Hidden, payload
        pack (Owned) (payload)
main = !->
    number = Number
    payload = Indirect
        pointer: (&number) as *Number
        marker: 65 as I32
    package = pack (Owned) (payload)
    repack package
"#;
    let scratch = Scratch::new();
    for opt_level in [0, 3] {
        let mut config = scratch.config(source);
        config.opt_level = opt_level;
        rock_lib::compile(&config).unwrap_or_else(|errors| {
            panic!("O{opt_level}, fixture {}: {errors:?}", scratch.0.display())
        });
        let output = Command::new(scratch.0.join("main")).output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"A", "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
    }
}

#[test]
fn package_rejects_two_different_concrete_witnesses() {
    reject(
        &format!(
            "{REFERENCES}{}",
            r#"
struct Other
impl Accept for Other
    @accept = other -> 0
main = !->
    left = Number
        value: 42
    right = Other
    pack (Pair) ((&left, &right))
"#
        ),
        DiagnosticCode::Type,
        &["witness", "mismatch", "same"],
    );
}

#[test]
fn package_cleanup_preserves_a_native_tail_result_and_custom_process_status() {
    let source = r#"
extern putchar: I32 -> I32
lang drop
< trait Dispose
    lang method
    ~@dispose: ()
trait Accept
    @accept: &Self -> I64
struct Number
impl Accept for Number
    @accept = other -> 0
struct Indirect T
    < pointer: *T
    < marker: I32
impl Dispose for Indirect T
    ~@dispose = !-> putchar self.marker
type Owned = exists T where T: Accept in Indirect T
repack: Owned -> Owned
repack = package ->
    open package as Hidden, payload
        pack (Owned) (payload)
summarize: Owned -> I64
summarize = package ->
    repack package
    7
main = ->
    number = Number
    payload = Indirect
        pointer: (&number) as *Number
        marker: 65 as I32
    summarize (pack (Owned) (payload))
"#;
    let scratch = Scratch::new();
    for opt_level in [0, 3] {
        let mut config = scratch.config(source);
        config.opt_level = opt_level;
        rock_lib::compile(&config).unwrap_or_else(|errors| {
            panic!("O{opt_level}, fixture {}: {errors:?}", scratch.0.display())
        });
        let output = Command::new(scratch.0.join("main")).output().unwrap();
        assert_eq!(output.status.code(), Some(7), "{output:?}");
        assert_eq!(output.stdout, b"A", "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
    }
}

#[test]
fn package_rejects_nonuniform_inline_hidden_payload() {
    reject(
        &format!(
            "{REFERENCES}{}",
            r#"
type Inline = exists T where T: Accept in T
main = !->
    number = Number
        value: 42
    pack (Inline) (number)
"#
        ),
        DiagnosticCode::Resolve,
        &["uniform", "layout"],
    );
}

#[test]
fn package_does_not_guess_a_witness_absent_from_the_payload() {
    reject(
        &format!(
            "{REFERENCES}{}",
            r#"
type Unused = exists T where T: Accept in I64
main = !->
    pack (Unused) (42)
"#
        ),
        DiagnosticCode::Type,
        &["witness", "infer", "ambiguous"],
    );
}

#[test]
fn package_requires_its_actual_witness_to_implement_the_bound() {
    reject(
        &format!(
            "{REFERENCES}{}",
            r#"
struct Other
main = !->
    value = Other
    pack (Pair) ((&value, &value))
"#
        ),
        DiagnosticCode::Type,
        &["implement", "bound", "evidence"],
    );
}

#[test]
fn owning_package_cannot_be_consumed_twice() {
    reject(
        r#"
lang drop
< trait Dispose
    lang method
    ~@dispose: ()
trait Accept
    @accept: &Self -> I64
struct Number
impl Accept for Number
    @accept = other -> 0
struct Indirect T
    < pointer: *T
impl Dispose for Indirect T
    ~@dispose = !-> ()
type Owned = exists T where T: Accept in Indirect T
repack: Owned -> Owned
repack = package ->
    open package as Hidden, payload
        pack (Owned) (payload)
main = !->
    number = Number
    payload = Indirect
        pointer: (&number) as *Number
    package = pack (Owned) (payload)
    first = repack package
    repack package
"#,
        DiagnosticCode::Borrow,
        &["moved", "move", "consumed"],
    );
}

#[test]
fn package_retains_local_reference_escape_restrictions() {
    reject(
        &format!(
            "{REFERENCES}{}",
            r#"
bad: () -> Pair
bad = ->
    left = Number
        value: 20
    right = Number
        value: 22
    pack (Pair) ((&left, &right))
main = !->
    bad!
"#
        ),
        DiagnosticCode::Borrow,
        &["return reference", "escape", "outlive"],
    );
}

#[test]
fn shared_package_view_cannot_invoke_a_mutable_member() {
    reject(
        r#"
trait Change
    ^@change: I64 -> ()
struct Number
    < value: I64
impl Change for Number
    ^@change = value !-> self.value = value
type Shared = exists T where T: Change in &T
main = !->
    number = Number
        value: 0
    package = pack (Shared) (&number)
    open package as Hidden, value
        value.change 42
"#,
        DiagnosticCode::Type,
        &["mutable", "shared"],
    );
}

#[test]
fn independently_opened_packages_do_not_share_self_evidence() {
    reject(
        &format!(
            "{REFERENCES}{}",
            r#"
main = !->
    number = Number
        value: 42
    first = pack (Pair) ((&number, &number))
    second = pack (Pair) ((&number, &number))
    open first as Left, a
        open second as Right, b
            a.0.accept b.1
"#
        ),
        DiagnosticCode::Type,
        &["witness", "mismatch", "opened"],
    );
}

fn library(root: &Path, name: &str, source: &str, dependencies: Vec<(String, PathBuf)>) -> PathBuf {
    let entry = root.join(format!("{name}.rk"));
    fs::write(&entry, source).unwrap();
    let mut products = rock_lib::compile_with_products(&Config {
        entry_file: entry,
        output_dir: root.into(),
        current_crate_name: Some(name.into()),
        no_std: true,
        no_prelude: true,
        no_link: true,
        emit_llvm: true,
        emit_object: Some(root.join(format!("{name}.o"))),
        extern_artifacts: dependencies,
        ..Config::default()
    })
    .unwrap()
    .products
    .unwrap();
    products.link.object_path = Some(PathBuf::from(format!("{name}.o")));
    let artifact = root.join(format!("{name}.rkca"));
    products.write_artifact_to_path(&artifact).unwrap();
    artifact
}

#[test]
fn packages_cross_source_free_artifacts_and_accept_a_later_implementation() {
    let scratch = Scratch::new();
    let contracts = library(
        &scratch.0,
        "contracts",
        r#"
< trait Accept
    @accept: &Self -> I64
type Pair = exists T where T: Accept in (&T, &T)
< Pair
repack: Pair -> Pair
< repack = package ->
    open package as Hidden, pair
        pack (Pair) (pair)
consume: Pair -> I64
< consume = package ->
    open package as Hidden, pair
        pair.0.accept pair.1
"#,
        vec![],
    );
    let values = library(
        &scratch.0,
        "values",
        r#"
> contracts::Accept
> contracts::Pair
< struct Number
    < value: I64
impl Accept for Number
    @accept = other -> ~I64Add self.value, other.value
make: &Number -> &Number -> Pair
< make = left, right -> pack (Pair) ((left, right))
"#,
        vec![("contracts".into(), contracts.clone())],
    );
    let dependencies = vec![("contracts".into(), contracts), ("values".into(), values)];
    let original_files = dependencies
        .iter()
        .flat_map(|(name, artifact)| [artifact.clone(), scratch.0.join(format!("{name}.o"))])
        .map(|path| {
            let contents = fs::read(&path).unwrap();
            (path, contents)
        })
        .collect::<Vec<_>>();
    for name in ["contracts", "values"] {
        fs::remove_file(scratch.0.join(format!("{name}.rk"))).unwrap();
    }
    let source = r#"
> contracts::Accept
> contracts::Pair
> contracts::consume
> contracts::repack
> values::Number
> values::make
struct Other
    < value: I64
impl Accept for Other
    @accept = other -> ~I64Add self.value, other.value
main = ->
    one = Number
        value: 20
    two = Number
        value: 1
    first = consume (repack (make &one, &two))
    three = Other
        value: 2
    four = Other
        value: 19
    second = consume (pack (Pair) ((&three, &four)))
    ~I64Add first, second
"#;
    for opt_level in [0, 3] {
        let mut config = scratch.config(source);
        config.opt_level = opt_level;
        config.extern_artifacts = dependencies.clone();
        rock_lib::compile(&config).unwrap_or_else(|errors| {
            panic!("O{opt_level}, fixture {}: {errors:?}", scratch.0.display())
        });
        execute(&scratch.0, 42);
        for (path, original) in &original_files {
            assert_eq!(&fs::read(path).unwrap(), original, "{}", path.display());
        }
    }
}

#[test]
fn owned_packages_preserve_actual_drop_across_source_free_artifacts() {
    let scratch = Scratch::new();
    let contracts = library(
        &scratch.0,
        "contracts",
        r#"
extern putchar: I32 -> I32
lang drop
< trait Dispose
    lang method
    ~@dispose: ()
< trait Accept
    @accept: &Self -> I64
< struct Indirect T
    < pointer: *T
    < marker: I32
impl Dispose for Indirect T
    ~@dispose = !-> putchar self.marker
type Owned = exists T where T: Accept in Indirect T
< Owned
repack: Owned -> Owned
< repack = package ->
    open package as Hidden, payload
        pack (Owned) (payload)
"#,
        vec![],
    );
    let values = library(
        &scratch.0,
        "values",
        r#"
> contracts::Accept
> contracts::Indirect
> contracts::Owned
< struct Number
impl Accept for Number
    @accept = other -> 0
make: &Number -> Owned
< make = number ->
    payload = Indirect
        pointer: number as *Number
        marker: 65 as I32
    pack (Owned) (payload)
"#,
        vec![("contracts".into(), contracts.clone())],
    );
    let dependencies = vec![("contracts".into(), contracts), ("values".into(), values)];
    let original_files = dependencies
        .iter()
        .flat_map(|(name, artifact)| [artifact.clone(), scratch.0.join(format!("{name}.o"))])
        .map(|path| {
            let contents = fs::read(&path).unwrap();
            (path, contents)
        })
        .collect::<Vec<_>>();
    for name in ["contracts", "values"] {
        fs::remove_file(scratch.0.join(format!("{name}.rk"))).unwrap();
    }
    let source = r#"
> contracts::repack
> values::Number
> values::make
main = !->
    number = Number
    repack (make &number)
"#;
    for opt_level in [0, 3] {
        let mut config = scratch.config(source);
        config.opt_level = opt_level;
        config.extern_artifacts = dependencies.clone();
        rock_lib::compile(&config).unwrap_or_else(|errors| {
            panic!("O{opt_level}, fixture {}: {errors:?}", scratch.0.display())
        });
        let output = Command::new(scratch.0.join("main")).output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"A", "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        for (path, original) in &original_files {
            assert_eq!(&fs::read(path).unwrap(), original, "{}", path.display());
        }
    }
}

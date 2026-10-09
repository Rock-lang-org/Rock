//! Concrete and descriptor-backed calls must have identical lexical drop timing.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use rock_lib::Config;

const HEADER: &str = r#"
lang drop
< trait Dispose
    lang method
    ~@dispose: ()
< trait Runner
    @run: T -> *I64 -> I64
< struct Service
    < marker: I64
struct Guard
    < drops: *I64
    < marker: I64
impl Dispose for Guard
    ~@dispose = !->
        unsafe *self.drops = ~I64Add (~I64Mul (*self.drops), 10), self.marker
read_count: *I64 -> I64
read_count = pointer -> unsafe *pointer
extern abort: () -> ()
check_count: *I64 -> I64 -> ()
check_count = pointer, expected !->
    if ~I64Ne (read_count pointer), expected then abort!
make_guard: *I64 -> I64 -> Guard
make_guard = pointer, marker ->
    Guard
        drops: pointer
        marker: marker
"#;

const BLOCK_LOCAL: &str = r#"        if true
            guard = make_guard pointer, 1
            ()
        read_count pointer
"#;

const EXPORT: &str = r#"
as_runner: &Service -> &Runner
< as_runner = service ->
    object: &Runner = service
    object
"#;

fn provider(body: &str) -> String {
    format!(
        r#"{HEADER}
impl Runner for Service
    @run = value, pointer ->
{body}{EXPORT}"#
    )
}

fn client(erased: bool, observed: i64, drops: i64) -> String {
    format!(
        r#"
main = ->
    mut drops = 0
    service = Service
        marker: 0
    receiver = {}
    pointer = (&mut drops) as *I64
    observed = receiver.run 42, pointer
    if ~I64Ne observed, {observed} then 1
    else if ~I64Ne drops, {drops} then 2
    else 42
"#,
        if erased {
            "as_runner &service"
        } else {
            "&service"
        }
    )
}

fn payload_client(wide: bool) -> String {
    let fields = if wide {
        "    < padding: (I64, I64)\n"
    } else {
        ""
    };
    let padding = if wide {
        "        padding: (7, 9)\n"
    } else {
        ""
    };
    format!(r#"
struct Later
{fields}    < drops: *I64
impl Dispose for Later
    ~@dispose = !->
        unsafe *self.drops = ~I64Add (*self.drops), 1
{}"#, client(true, 1, 1).replace(
        "    observed = receiver.run 42, pointer",
        &format!("    payload = Later\n{padding}        drops: pointer\n    observed = receiver.run payload, pointer"),
    ))
}

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn scratch() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../.agent-work/fixtures")
        .join(format!(
            "erased-scope-cleanup-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn config(root: &Path, entry: &Path, opt_level: u8) -> Config {
    Config {
        entry_file: entry.into(),
        output_dir: root.into(),
        current_crate_name: Some("scope_cleanup".into()),
        no_std: true,
        no_prelude: true,
        emit_llvm: true,
        opt_level,
        ..Default::default()
    }
}

fn run(config: &Config) {
    rock_lib::compile(config).unwrap();
    let output = Command::new(config.output_dir.join("main"))
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(42),
        "drop must occur before the following read, exactly once: {output:?}"
    );
}

#[test]
fn block_local_drops_before_the_next_statement_at_o0_o3() {
    let root = scratch();
    let entry = root.join("main.rk");
    for erased in [false, true] {
        fs::write(
            &entry,
            format!("{}{}", provider(BLOCK_LOCAL), client(erased, 1, 1)),
        )
        .unwrap();
        for optimization in [0, 3] {
            run(&config(&root, &entry, optimization));
        }
    }
}

#[test]
fn nested_scopes_moves_returns_and_statement_temporaries_at_o0_o3() {
    let cases = [
        (
            r#"        if true
            first = make_guard pointer, 1
            if true
                second = make_guard pointer, 2
                ()
            check_count pointer, 2
            third = make_guard pointer, 3
            ()
        read_count pointer
"#,
            231,
            231,
        ),
        (
            r#"        survivor = if true
            guard = make_guard pointer, 1
            guard
        else
            make_guard pointer, 2
        read_count pointer
"#,
            0,
            1,
        ),
        (
            r#"        outer = make_guard pointer, 1
        if true
            inner = make_guard pointer, 2
            return 42
        read_count pointer
"#,
            42,
            21,
        ),
        (
            r#"        if false
            absent = make_guard pointer, 1
            ()
        read_count pointer
"#,
            0,
            0,
        ),
        (
            r#"        make_guard pointer, 1
        read_count pointer
"#,
            1,
            1,
        ),
    ];
    let root = scratch();
    let entry = root.join("main.rk");
    for (body, observed, drops) in cases {
        for erased in [false, true] {
            fs::write(
                &entry,
                format!("{}{}", provider(body), client(erased, observed, drops)),
            )
            .unwrap();
            for optimization in [0, 3] {
                run(&config(&root, &entry, optimization));
            }
        }
    }
}

#[test]
fn source_free_block_cleanup_keeps_producer_bytes_unchanged_at_o0_o3() {
    let root = scratch();
    let source = root.join("provider.rk");
    fs::write(&source, provider(BLOCK_LOCAL)).unwrap();
    let object = root.join("provider.o");
    let producer = Config {
        no_link: true,
        emit_object: Some(object.clone()),
        ..config(&root, &source, 0)
    };
    let mut products = rock_lib::compile_with_products(&producer)
        .unwrap()
        .products
        .unwrap();
    products.link.object_path = Some("provider.o".into());
    let artifact = root.join("provider.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let object_before = fs::read(&object).unwrap();
    let artifact_before = fs::read(&artifact).unwrap();
    fs::remove_file(&source).unwrap(); // Only source created by this test.
    let entry = root.join("main.rk");
    fs::write(
        &entry,
        format!(
            "> scope_cleanup::Runner\n> scope_cleanup::Service\n> scope_cleanup::as_runner\n{}",
            client(true, 1, 1)
        ),
    )
    .unwrap();
    for optimization in [0, 3] {
        run(&Config {
            current_crate_name: Some("cleanup_client".into()),
            extern_artifacts: vec![("scope_cleanup".into(), artifact.clone())],
            ..config(&root, &entry, optimization)
        });
        assert_eq!(fs::read(&object).unwrap(), object_before);
        assert_eq!(fs::read(&artifact).unwrap(), artifact_before);
    }
}

#[test]
fn source_free_runtime_payload_drops_at_scope_exit_for_two_later_layouts() {
    let root = scratch();
    let source = root.join("provider.rk");
    fs::write(
        &source,
        provider(
            r#"        if true
            local = value
            ()
        read_count pointer
"#,
        ),
    )
    .unwrap();
    let object = root.join("provider.o");
    let mut products = rock_lib::compile_with_products(&Config {
        no_link: true,
        emit_object: Some(object.clone()),
        ..config(&root, &source, 0)
    })
    .unwrap()
    .products
    .unwrap();
    products.link.object_path = Some("provider.o".into());
    let artifact = root.join("provider.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let object_before = fs::read(&object).unwrap();
    let artifact_before = fs::read(&artifact).unwrap();
    fs::remove_file(&source).unwrap(); // Only source created by this test.
    let entry = root.join("main.rk");
    for wide in [false, true] {
        fs::write(&entry, format!("> scope_cleanup::Dispose\n> scope_cleanup::Runner\n> scope_cleanup::Service\n> scope_cleanup::as_runner\n{}", payload_client(wide))).unwrap();
        for optimization in [0, 3] {
            run(&Config {
                current_crate_name: Some("later_cleanup_client".into()),
                extern_artifacts: vec![("scope_cleanup".into(), artifact.clone())],
                ..config(&root, &entry, optimization)
            });
            assert_eq!(fs::read(&object).unwrap(), object_before);
            assert_eq!(fs::read(&artifact).unwrap(), artifact_before);
        }
    }
}

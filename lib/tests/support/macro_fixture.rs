//! Immutable native SDK prerequisites shared across acceptance groups and runs.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{atomic::{AtomicU64, Ordering}, OnceLock};

use rock_lib::{Config, SourceProvider};
use sha2::{Digest, Sha256};

#[path = "std_cache_key.rs"]
mod cache_key;

pub(super) fn build_library(entry: PathBuf, name: &str, output: &Path, dependencies: Vec<(String, PathBuf)>) -> PathBuf {
    fs::create_dir_all(output).unwrap();
    let result = rock_lib::compile_with_products(&Config {
        entry_file: entry, output_dir: output.into(), current_crate_name: Some(name.into()),
        extern_artifacts: dependencies, no_std: true, no_prelude: name != "std", no_proc_execution: true,
        no_link: true, emit_object: Some(output.join(format!("{name}.o"))), ..Default::default()
    }).unwrap_or_else(|diagnostics| panic!("{name}: {diagnostics:?}"));
    let products = result.products.unwrap();
    assert!(products.proc_macros.is_empty(), "the SDK is an ordinary library");
    let artifact = output.join(format!("{name}.rkca"));
    products.write_artifact_to_path(&artifact).unwrap();
    artifact
}

struct Prerequisites {
    directory: PathBuf,
    std: PathBuf,
    sdk: PathBuf,
}

fn outputs_digest(root: &Path) -> Option<String> {
    let mut hash = Sha256::new();
    for file in ["std/std.rkca", "std/std.o", "sdk/rock_macro.rkca", "sdk/rock_macro.o"] {
        let bytes = fs::read(root.join(file)).ok()?;
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    Some(format!("{:x}", hash.finalize()))
}

fn prerequisites() -> &'static Prerequisites {
    static FIXTURE: OnceLock<Prerequisites> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let executable = std::env::current_exe().unwrap();
        let compiler = format!("{:x}", Sha256::digest(fs::read(&executable).unwrap()));
        // Include compiler/schema, source trees, target and the fixed build policy.
        let stamp = format!("macro-fixture-v1:{compiler}:{}:{}:no-link,no-std,no-proc,explicit-prelude",
            cache_key::std_cache_compiler_stamp(), rock_lib::sysroot::host_target_triple());
        let key = format!("{}-{}", cache_key::std_cache_key(&workspace.join("std"), &stamp),
            cache_key::std_cache_key(&workspace.join("rock_macro"), &stamp));
        let profile = rock_shared::process::dev_target_binary_from_exe(&executable, "rockc").unwrap().parent().unwrap().to_path_buf();
        let base = profile.join("test-fixtures/macro-sdk");
        fs::create_dir_all(&base).unwrap();
        // OS lock survives neither a crash nor process exit; partial builds have
        // no valid completion digest and are never consumed by another runner.
        let lock = OpenOptions::new().create(true).truncate(false).read(true).write(true).open(base.join(format!("{key}.lock"))).unwrap();
        lock.lock().unwrap();
        let directory = base.join(key);
        fs::create_dir_all(&directory).unwrap();
        let marker = directory.join("ready");
        let reusable = fs::read_to_string(&marker).ok().zip(outputs_digest(&directory)).is_some_and(|(expected, actual)| expected == actual);
        if !reusable {
            eprintln!("SDK fixture: building std/SDK prerequisites");
            let std = build_library(workspace.join("std/src/lib.rk"), "std", &directory.join("std"), Vec::new());
            build_library(workspace.join("rock_macro/src/lib.rk"), "rock_macro", &directory.join("sdk"), vec![("std".into(), std)]);
            let temporary = directory.join("ready.new");
            fs::write(&temporary, outputs_digest(&directory).unwrap()).unwrap();
            fs::rename(temporary, marker).unwrap();
        } else {
            eprintln!("SDK fixture: reusing validated std/SDK prerequisites");
        }
        Prerequisites { std: directory.join("std/std.rkca"), sdk: directory.join("sdk/rock_macro.rkca"), directory }
    })
}

pub(super) struct SdkCase {
    root: PathBuf,
    pub config: Config,
}

impl SdkCase {
    pub fn new(group: &str, source: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let shared = prerequisites();
        let root = shared.directory.join("runs").join(format!("{group}-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir_all(&root).unwrap();
        let entry = root.join("main.rk");
        let config = Config {
            entry_file: entry.clone(), output_dir: root.join("app"), no_std: true, no_proc_execution: true,
            current_crate_name: Some("sdk_client".into()),
            extern_artifacts: vec![("std".into(), shared.std.clone()), ("rock_macro".into(), shared.sdk.clone())],
            source_providers: vec![SourceProvider::Virtual { path: entry, text: source.into() }], ..Default::default()
        };
        Self { root, config }
    }
}

impl Drop for SdkCase {
    fn drop(&mut self) {
        if !std::thread::panicking() { fs::remove_dir_all(&self.root).unwrap(); }
    }
}

#[test]
fn macro_fixture_completion_digest_detects_missing_and_changed_outputs() {
    let profile = rock_shared::process::dev_target_binary_from_current_exe("rockc").unwrap();
    let directory = profile.parent().unwrap().join("test-fixture-digest").join(std::process::id().to_string());
    fs::create_dir_all(directory.join("std")).unwrap();
    fs::create_dir_all(directory.join("sdk")).unwrap();
    assert!(outputs_digest(&directory).is_none());
    for file in ["std/std.rkca", "std/std.o", "sdk/rock_macro.rkca", "sdk/rock_macro.o"] { fs::write(directory.join(file), file).unwrap(); }
    let first = outputs_digest(&directory).unwrap();
    fs::write(directory.join("sdk/rock_macro.o"), "changed").unwrap();
    assert_ne!(first, outputs_digest(&directory).unwrap());
    fs::remove_dir_all(directory).unwrap();
}

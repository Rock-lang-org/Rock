use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{
    constants::{
        BIN_DIR, COMPONENTS_MANIFEST_NAME, DEFAULT_TOOLCHAIN_FILE, LIB_DIR, ROCKC_BIN_NAME,
        ROCK_BIN_NAME, ROCK_LSP_BIN_NAME, ROCK_TOOLCHAIN_FILE, STD_ARTIFACT_NAME, STD_OBJECT_NAME,
        TOOLCHAIN_MANIFEST_NAME,
    },
    home::RockupHome,
    layout::host_target_triple,
};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(super) fn temp_test_dir(name: &str) -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = canonical_temp_dir().join(format!("rockup_{}_{}", name, id));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

pub(super) fn fake_home(root: PathBuf) -> RockupHome {
    RockupHome { root }
}

pub(super) fn write_fake_toolchain_root(root: &Path) {
    let bin_dir = root.join(BIN_DIR);
    let target_libdir = root
        .join(LIB_DIR)
        .join("rocklib")
        .join(host_target_triple());

    fs::create_dir_all(&bin_dir).unwrap();
    fs::create_dir_all(&target_libdir).unwrap();

    write_fake_tool(&bin_dir.join(ROCK_BIN_NAME), "exit 17");
    write_fake_tool(&bin_dir.join(ROCKC_BIN_NAME), "exit 0");
    write_fake_tool(&bin_dir.join(ROCK_LSP_BIN_NAME), "exit 31");
    fs::write(target_libdir.join(STD_ARTIFACT_NAME), "artifact").unwrap();
    fs::write(target_libdir.join(STD_OBJECT_NAME), "object").unwrap();
    fs::write(target_libdir.join(TOOLCHAIN_MANIFEST_NAME), "{}\n").unwrap();
    fs::write(target_libdir.join(COMPONENTS_MANIFEST_NAME), "{}\n").unwrap();
}

pub(super) fn write_fake_cargo_build_dir(root: &Path) {
    let target_libdir = root
        .join(LIB_DIR)
        .join("rocklib")
        .join(host_target_triple());

    fs::create_dir_all(&target_libdir).unwrap();
    write_fake_tool(&root.join(ROCK_BIN_NAME), "exit 13");
    write_fake_tool(&root.join(ROCKC_BIN_NAME), "exit 0");
    write_fake_tool(&root.join(ROCK_LSP_BIN_NAME), "exit 31");
    fs::write(target_libdir.join(STD_ARTIFACT_NAME), "artifact").unwrap();
    fs::write(target_libdir.join(STD_OBJECT_NAME), "object").unwrap();
    fs::write(target_libdir.join(TOOLCHAIN_MANIFEST_NAME), "{}\n").unwrap();
    fs::write(target_libdir.join(COMPONENTS_MANIFEST_NAME), "{}\n").unwrap();
}

pub(super) fn write_target_component_dir(root: &Path, label: &str) {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join(STD_ARTIFACT_NAME),
        format!("{} artifact\n", label),
    )
    .unwrap();
    fs::write(root.join(STD_OBJECT_NAME), format!("{} object\n", label)).unwrap();
    fs::write(
        root.join(TOOLCHAIN_MANIFEST_NAME),
        format!("{} manifest\n", label),
    )
    .unwrap();
    fs::write(
        root.join(COMPONENTS_MANIFEST_NAME),
        format!("{} components\n", label),
    )
    .unwrap();
}

pub(super) fn write_target_component_source(root: &Path, triple: &str, label: &str) {
    write_target_component_dir(&root.join(LIB_DIR).join("rocklib").join(triple), label);
}

/// Installs the compiled fake tool at `path` (plus the host executable
/// suffix) with `instructions` (see fixtures/fake_tool.rs) beside it.
pub(super) fn write_fake_tool(path: &Path, instructions: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let executable = path.with_file_name(crate::constants::exe_name(
        &path.file_name().unwrap().to_string_lossy(),
    ));
    let _ = fs::remove_file(&executable);
    if cfg!(unix) {
        // Keep writable executable descriptors out of the multithreaded test
        // process: concurrent child spawns could inherit them until exec and
        // fail with ETXTBSY.
        let status = std::process::Command::new("cp")
            .arg("--")
            .arg(fake_tool_binary())
            .arg(&executable)
            .status()
            .unwrap();
        assert!(status.success());
    } else {
        fs::copy(fake_tool_binary(), &executable).unwrap();
    }
    fs::write(path.with_extension("fake"), format!("{instructions}\n")).unwrap();
}

fn fake_tool_binary() -> &'static Path {
    static BINARY: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BINARY.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("rockup_fake_tool_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let source = dir.join("fake_tool.rs");
        fs::write(&source, include_str!("fixtures/fake_tool.rs")).unwrap();
        let binary = dir.join(crate::constants::exe_name("fake_tool"));
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let output = std::process::Command::new(rustc)
            .args(["--edition", "2021", "-o"])
            .arg(&binary)
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        binary
    })
}

pub(super) fn write_project_toolchain(path: &Path, toolchain: &str) {
    fs::write(
        path.join(ROCK_TOOLCHAIN_FILE),
        format!("[toolchain]\nchannel = \"{}\"\n", toolchain),
    )
    .unwrap();
}

pub(super) fn read_default_toolchain(home: &RockupHome) -> String {
    fs::read_to_string(home.root.join(DEFAULT_TOOLCHAIN_FILE))
        .unwrap()
        .trim()
        .to_string()
}

pub(super) fn workspace_std_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("std")
}

// The tools report resolved paths: Darwin's temp dir is a symlink into
// /private, and Windows' may use 8.3 short names.
fn canonical_temp_dir() -> std::path::PathBuf {
    dunce::canonicalize(std::env::temp_dir()).unwrap()
}

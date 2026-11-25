use crate::support::*;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::Ordering;

// Only the Linux backend file exists. The Windows block names a missing file
// and the macOS block imports a missing module: neither may be touched unless
// its target is selected.
const MAIN: &str = r#"
cfg(target_os = "linux")
    mod backend_linux
    > backend_linux::value

cfg(target_os = "windows")
    mod backend_windows
    > backend_windows::value

cfg(target_os = "macos")
    > nowhere::value

main = -> value!
"#;

fn write_project(dir: &Path) -> std::path::PathBuf {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("backend_linux.rk"), "< value = -> 42\n").unwrap();
    let entry = dir.join("main.rk");
    fs::write(&entry, MAIN).unwrap();
    entry
}

fn no_std_config(entry: std::path::PathBuf, dir: &Path, target: &str) -> rock_lib::Config {
    let mut config = test_config(entry, dir.join("build"));
    config.extern_artifacts.clear();
    config.no_std = true;
    config.no_prelude = true;
    config.target = Some(target.to_string());
    config.no_link = true;
    config
}

fn messages(diagnostics: &rock_lib::diagnostic::Diagnostics) -> String {
    diagnostics
        .0
        .iter()
        .map(|diagnostic| diagnostic.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn test_cfg_selects_the_active_backend_module() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    let _cleanup = TestDirCleanup(dir.clone());
    let entry = write_project(&dir);

    let mut config = no_std_config(entry, &dir, "x86_64-unknown-linux-gnu");
    config.no_link = false;
    rock_lib::compile(&config).expect("linux selection should compile");
    let mut command = Command::new(dir.join("build/main"));
    assert_eq!(run_test_command(&mut command).status.code(), Some(42));
}

#[test]
fn test_cfg_loads_inactive_files_only_when_their_target_is_selected() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    let _cleanup = TestDirCleanup(dir.clone());
    let entry = write_project(&dir);

    let error = rock_lib::compile(&no_std_config(
        entry.clone(),
        &dir,
        "x86_64-pc-windows-msvc",
    ))
    .expect_err("windows selection needs the missing backend file");
    assert!(
        messages(&error).contains("backend_windows"),
        "unexpected diagnostics: {}",
        messages(&error)
    );

    let error = rock_lib::compile(&no_std_config(entry, &dir, "aarch64-apple-darwin"))
        .expect_err("macos selection imports a missing module");
    assert!(
        messages(&error).contains("nowhere"),
        "unexpected diagnostics: {}",
        messages(&error)
    );
}

#[test]
fn test_cfg_emits_objects_for_the_selected_target() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    let _cleanup = TestDirCleanup(dir.clone());
    fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.rk");
    fs::write(&entry, "main = -> 7\n").unwrap();

    let mut config = no_std_config(entry, &dir, "aarch64-apple-darwin");
    config.emit_llvm = true;
    rock_lib::compile(&config).expect("darwin object emission should succeed");
    let ir = fs::read_to_string(dir.join("build/main.ll")).unwrap();
    assert!(
        ir.contains("target triple = \"aarch64-apple-darwin\""),
        "{ir}"
    );
    // Mach-O 64-bit magic, little endian.
    let object = fs::read(dir.join("build/main.o")).unwrap();
    assert_eq!(&object[..4], &[0xcf, 0xfa, 0xed, 0xfe]);
}

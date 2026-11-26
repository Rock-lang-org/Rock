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
fn test_extern_link_name_binds_a_foreign_symbol_under_a_rock_name() {
    let status = compile_and_run_without_std(
        r#"
extern "abs" c_abs: I32 -> I32

main = -> (c_abs (~IntWrappingSub (0 as I32), (42 as I32))) as I64
"#,
    );
    assert_eq!(status, 42);
}

#[test]
fn test_externs_sharing_a_symbol_must_agree_on_the_signature() {
    compile_should_fail(
        r#"
extern "abs" abs_i32: I32 -> I32
extern "abs" abs_i64: I64 -> I64

main = !->
    (abs_i32 (1 as I32)).println!
    (abs_i64 1).println!
"#,
        "foreign symbol 'abs' is declared with conflicting signatures",
    );
}

#[test]
fn test_externs_sharing_a_symbol_with_one_signature_link_once() {
    let status = compile_and_run_without_std(
        r#"
extern "abs" first_abs: I32 -> I32
extern "abs" second_abs: I32 -> I32

main = ->
    zero = first_abs (0 as I32)
    seven = second_abs (~IntWrappingSub (0 as I32), (7 as I32))
    (~IntWrappingAdd zero, seven) as I64
"#,
    );
    assert_eq!(status, 7);
}

// The same program runs natively on macOS and Windows in CI.
#[test]
fn test_platform_smoke_program_runs_on_the_host() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    let _cleanup = TestDirCleanup(dir.clone());
    let source =
        fs::read_to_string(workspace_root().join("lib/tests/fixtures/platform_smoke/src/main.rk"))
            .unwrap();
    let output = compile_and_run_in_working_dir(&source, &dir);
    assert!(
        output.status.success(),
        "smoke failed with {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "platform smoke ok"
    );
}

// No native runner exists for macOS or Windows here, so at least keep std
// compiling for every supported target; each backend is selected by cfg.
#[test]
fn test_std_compiles_for_every_supported_target() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    let _cleanup = TestDirCleanup(dir.clone());
    for (target, magic) in [
        ("x86_64-unknown-linux-gnu", &[0x7f, b'E', b'L', b'F'][..]),
        ("aarch64-apple-darwin", &[0xcf, 0xfa, 0xed, 0xfe][..]),
        ("x86_64-pc-windows-msvc", &[0x64, 0x86][..]),
    ] {
        let out = dir.join(target);
        let object = out.join("std.o");
        let mut config = test_config(std_path().join("src/lib.rk"), out.clone());
        config.extern_artifacts.clear();
        config.no_std = true;
        config.no_link = true;
        config.crate_kind = rock_lib::CrateKind::Lib;
        config.current_crate_name = Some("std".to_string());
        config.target = Some(target.to_string());
        config.emit_object = Some(object.clone());
        if let Err(diagnostics) = rock_lib::compile(&config) {
            panic!("std failed for {target}:\n{}", messages(&diagnostics));
        }
        let bytes = fs::read(&object).unwrap();
        assert_eq!(&bytes[..magic.len()], magic, "object format for {target}");
    }
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

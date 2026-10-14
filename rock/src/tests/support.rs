use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::{
    bundled_sysroot::{current_sysroot_layout, STD_CRATE_NAME},
    package::Package,
};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) fn sysroot_env_lock() -> &'static Mutex<()> {
    static LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub(crate) fn temp_test_dir(name: &str) -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rock_cli_{}_{}", name, id));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

pub(crate) fn write_package(
    root: &Path,
    name: &str,
    lib_path: &str,
    deps: &[(&str, String)],
    source: &str,
) {
    write_package_with_options(root, name, lib_path, deps, source, false);
}

pub(super) fn write_package_with_options(
    root: &Path,
    name: &str,
    lib_path: &str,
    deps: &[(&str, String)],
    source: &str,
    no_std: bool,
) {
    let mut manifest = format!(
        "[crate]\nname = \"{}\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"{}\"\n",
        name, lib_path
    );

    if no_std {
        manifest = format!(
            "[crate]\nname = \"{}\"\nversion = \"0.1.0\"\nno_std = true\n\n[lib]\npath = \"{}\"\n",
            name, lib_path
        );
    }

    if !deps.is_empty() {
        manifest.push_str("\n[dependencies]\n");
        for (dep_name, dep_path) in deps {
            manifest.push_str(&format!("{} = {{ path = \"{}\" }}\n", dep_name, dep_path));
        }
    }

    fs::create_dir_all(root).unwrap();
    fs::write(root.join("rock.toml"), manifest).unwrap();

    let lib_file = root.join(lib_path);
    if let Some(parent) = lib_file.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(lib_file, source).unwrap();
}

pub(super) fn assert_sysroot_std_files_exist() {
    let layout = current_sysroot_layout().unwrap();

    assert!(layout.std_artifact.exists());
    assert!(layout.std_object.exists());
    assert!(layout.manifest_path.exists());
    assert!(layout.components_path.exists());
}

pub(super) fn write_answer_std(root: &Path, answer: i32) {
    write_package(
        root,
        STD_CRATE_NAME,
        "lib.rk",
        &[],
        "< mod prelude\n< mod math\n",
    );
    fs::write(root.join("prelude.rk"), "< std::math::answer\n").unwrap();
    fs::write(
        root.join("math.rk"),
        format!("< answer = -> {}\n< answer\n", answer),
    )
    .unwrap();
}

pub(crate) fn load_package(root: PathBuf) -> Package {
    Package::load(root).unwrap()
}

pub(crate) fn run_command_output(command: &mut ProcessCommand) -> Output {
    command
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn child process with captured output: {}", e))
}

pub(crate) fn assert_executable_exit_code(executable: &Path, expected: i32) {
    let mut command = ProcessCommand::new(executable);
    let output = run_command_output(&mut command);

    assert_eq!(
        output.status.code(),
        Some(expected),
        "expected {} to exit with {}; stdout:\n{}\nstderr:\n{}",
        executable.display(),
        expected,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_run_command_output_captures_child_stdout_and_stderr() {
    let mut command = ProcessCommand::new("sh");
    command.args([
        "-c",
        "printf visible-stdout; printf visible-stderr >&2; exit 3",
    ]);

    let output = run_command_output(&mut command);

    assert_eq!(output.status.code(), Some(3));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "visible-stdout");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "visible-stderr");
}

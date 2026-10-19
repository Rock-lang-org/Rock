use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{
    bundled_sysroot::{current_sysroot_layout, STD_CRATE_NAME},
    package::Package,
};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Re-run only this test when it needs process-local environment or cwd state.
/// Overrides belong to the child command, never to the concurrent parent suite.
pub(crate) fn run_isolated_test(name: &str, env: &[(&str, &OsStr)]) -> bool {
    const ISOLATED_TEST: &str = "ROCK_ISOLATED_TEST";
    if std::env::var(ISOLATED_TEST).as_deref() == Ok(name) {
        return false;
    }
    let mut command = ProcessCommand::new(std::env::current_exe().unwrap());
    command.args([name, "--exact", "--nocapture"])
        .env(ISOLATED_TEST, name)
        .env("ROCKC", crate::rockc::resolve_rockc_path().unwrap());
    for (key, value) in env {
        command.env(key, value);
    }
    let output = run_command_output(&mut command);
    assert!(output.status.success(), "isolated test {name} failed; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "isolated test filter must execute exactly one test: {name}");
    true
}

pub(crate) fn temp_test_dir(name: &str) -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rock_cli_{}_{}_{}", name, std::process::id(), id));
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

pub(crate) fn write_package_with_options(
    root: &Path,
    name: &str,
    lib_path: &str,
    deps: &[(&str, String)],
    source: &str,
    no_std: bool,
) {
    let target = if Path::new(lib_path).file_name() == Some(OsStr::new("main.rk")) {
        "[[bin]]"
    } else {
        "[lib]"
    };
    let mut manifest = format!(
        "[crate]\nname = \"{}\"\nversion = \"0.1.0\"\nno_std = {}\n\n{}\npath = \"{}\"\n",
        name, no_std, target, lib_path
    );

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

pub(crate) fn build_project(root: &Path, profile: crate::package::BuildProfile) -> Result<PathBuf, String> {
    let products = crate::build::build_project(root, profile)?;
    let paths = products.paths().cloned().collect::<Vec<_>>();
    assert_eq!(paths.len(), 1, "single-target fixture must have one build output");
    Ok(paths[0].clone())
}

pub(crate) fn run_project(root: &Path, profile: crate::package::BuildProfile, args: &[String]) -> Result<i32, String> {
    crate::build::run_project(root, profile, args, None)
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

pub(crate) fn load_dependency_package(root: PathBuf, project_root: &Path) -> Package {
    let mut package = load_package(root);
    package.use_project_build(&project_root.canonicalize().unwrap());
    package
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
fn isolated_test_overrides_are_child_local() {
    let previous = std::env::var_os("ROCK_TEST_CHILD_VALUE");
    let previous_dir = std::env::current_dir().unwrap();
    if run_isolated_test(
        "tests::support::isolated_test_overrides_are_child_local",
        &[("ROCK_TEST_CHILD_VALUE", OsStr::new("child-only"))],
    ) {
        assert_eq!(std::env::var_os("ROCK_TEST_CHILD_VALUE"), previous);
        assert_eq!(std::env::current_dir().unwrap(), previous_dir);
        return;
    }
    assert_eq!(std::env::var("ROCK_TEST_CHILD_VALUE").unwrap(), "child-only");
    let root = temp_test_dir("isolated_cwd");
    std::env::set_current_dir(&root).unwrap();
    assert_eq!(std::env::current_dir().unwrap(), root);
    std::env::set_current_dir(previous_dir).unwrap();
    fs::remove_dir_all(root).unwrap();
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

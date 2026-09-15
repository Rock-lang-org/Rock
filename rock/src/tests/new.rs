use std::{fs, process::Command};

use crate::{build::build_project, commands::new_project, package::Package};

use super::support::{run_command_output, sysroot_env_lock, temp_test_dir};

#[test]
fn test_new_project_scaffolds_manifest_and_source() {
    let parent = temp_test_dir("new_scaffold");
    let root = new_project(&parent, "hello-rock_2").unwrap();
    let package = Package::load(root.clone()).unwrap();
    assert_eq!(package.manifest.crate_.name, "hello-rock_2");
    assert_eq!(package.manifest.crate_.version, "0.1.0");
    assert!(!package.manifest.crate_.no_std);
    assert_eq!(package.manifest.lib.path, "src/main.rk");
    assert_eq!(
        fs::read_to_string(package.entry_file()).unwrap(),
        "main = !->\n    \"Hello, Rock!\".println!\n"
    );
    assert_eq!(
        fs::read_to_string(root.join(".gitignore")).unwrap(),
        "/build/\n"
    );
    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn test_new_project_rejects_invalid_names_without_creating_files() {
    let parent = temp_test_dir("new_invalid");
    for name in [
        "",
        ".",
        "..",
        "../escape",
        "a/b",
        "a\\b",
        "1app",
        "a b",
        "a\"b",
        "stdlib",
    ] {
        assert!(new_project(&parent, name).is_err(), "accepted {name:?}");
    }
    assert_eq!(fs::read_dir(&parent).unwrap().count(), 0);
    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn test_new_project_preserves_existing_destinations() {
    let parent = temp_test_dir("new_existing");
    let root = new_project(&parent, "hello").unwrap();
    fs::write(root.join("src/main.rk"), "existing source").unwrap();
    assert!(new_project(&parent, "hello").is_err());
    assert_eq!(
        fs::read_to_string(root.join("src/main.rk")).unwrap(),
        "existing source"
    );
    fs::write(parent.join("file"), "existing file").unwrap();
    assert!(new_project(&parent, "file").is_err());
    assert_eq!(
        fs::read_to_string(parent.join("file")).unwrap(),
        "existing file"
    );
    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn test_new_project_hello_world_runs() {
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let parent = temp_test_dir("new_run");
    let root = new_project(&parent, "hello-rock").unwrap();
    let executable = build_project(&root).unwrap();
    let output = run_command_output(&mut Command::new(executable));
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "Hello, Rock!\n");
    fs::remove_dir_all(parent).unwrap();
}

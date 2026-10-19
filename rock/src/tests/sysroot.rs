use std::fs;
use std::process::Command as ProcessCommand;

use super::support::build_project;
use crate::bundled_sysroot::STD_CRATE_NAME;
use crate::package::BuildProfile;

use super::support::{
    assert_executable_exit_code, assert_sysroot_std_files_exist, run_isolated_test,
    temp_test_dir, write_answer_std, write_package,
};

#[test]
fn test_build_project_uses_sysroot_std() {
    let temp_dir = temp_test_dir("auto_std_root");
    write_package(
        &temp_dir,
        "auto_std_root",
        "src/main.rk",
        &[],
        "main = -> 42.println!\n",
    );

    let executable = build_project(&temp_dir, BuildProfile::Debug).unwrap();
    let output = ProcessCommand::new(&executable).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");
    assert_sysroot_std_files_exist();

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_build_project_uses_sysroot_std_for_dependencies() {
    let temp_dir = temp_test_dir("auto_std_dep");
    let dep = temp_dir.join("dep");
    let app = temp_dir.join("app");

    write_package(
        &dep,
        "dep",
        "lib.rk",
        &[],
        "print_answer = -> 42.println!\n< print_answer\n",
    );
    write_package(
        &app,
        "app",
        "src/main.rk",
        &[("dep", dep.display().to_string())],
        "> dep::print_answer\n\nmain = -> print_answer!\n",
    );

    let executable = build_project(&app, BuildProfile::Debug).unwrap();
    let output = ProcessCommand::new(&executable).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");
    assert_sysroot_std_files_exist();

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_build_project_explicit_std_dependency_overrides_sysroot() {
    let temp_dir = temp_test_dir("explicit_std_override");
    let std = temp_dir.join("std");
    let app = temp_dir.join("app");

    write_answer_std(&std, 5);
    write_package(
        &app,
        "app",
        "src/main.rk",
        &[(STD_CRATE_NAME, std.display().to_string())],
        "> std::math::answer\n\nmain = -> answer!\n",
    );

    let executable = build_project(&app, BuildProfile::Debug).unwrap();
    assert_executable_exit_code(&executable, 5);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_explicit_rock_sysroot_does_not_bootstrap_workspace_std() {
    let temp_dir = temp_test_dir("explicit_sysroot_override");
    let explicit_sysroot = temp_dir.join("dev-sysroot");

    write_package(
        &temp_dir.join("app"),
        "app",
        "src/main.rk",
        &[],
        "main = -> 42.println!\n",
    );

    if run_isolated_test(
        "tests::sysroot::test_explicit_rock_sysroot_does_not_bootstrap_workspace_std",
        &[("ROCK_SYSROOT", explicit_sysroot.as_os_str())],
    ) {
        fs::remove_dir_all(temp_dir).unwrap();
        return;
    }
    let error = build_project(&temp_dir.join("app"), BuildProfile::Debug).unwrap_err();

    assert!(error.contains("does not contain a valid bundled std"));

    let _ = fs::remove_dir_all(temp_dir);
}

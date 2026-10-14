use std::fs;
use std::process::Command as ProcessCommand;

use crate::build::build_project;
use crate::bundled_sysroot::STD_CRATE_NAME;
use crate::package::BuildProfile;

use super::support::{
    assert_executable_exit_code, assert_sysroot_std_files_exist, sysroot_env_lock,
    temp_test_dir, write_answer_std, write_package,
};

#[test]
fn test_build_project_uses_sysroot_std() {
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = temp_test_dir("explicit_sysroot_override");
    let explicit_sysroot = temp_dir.join("dev-sysroot");
    let previous = std::env::var_os("ROCK_SYSROOT");

    write_package(
        &temp_dir.join("app"),
        "app",
        "src/main.rk",
        &[],
        "main = -> 42.println!\n",
    );

    std::env::set_var("ROCK_SYSROOT", &explicit_sysroot);
    let error = build_project(&temp_dir.join("app"), BuildProfile::Debug).unwrap_err();

    match previous {
        Some(value) => std::env::set_var("ROCK_SYSROOT", value),
        None => std::env::remove_var("ROCK_SYSROOT"),
    }

    assert!(error.contains("does not contain a valid bundled std"));

    let _ = fs::remove_dir_all(temp_dir);
}

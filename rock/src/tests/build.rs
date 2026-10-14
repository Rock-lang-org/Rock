use std::fs;

use crate::build::{build_project, run_project};

use super::support::{
    assert_executable_exit_code, load_package, sysroot_env_lock, temp_test_dir, write_package,
    write_package_with_options,
};

#[test]
fn test_rock_cargo_toml_does_not_depend_on_rock_lib() {
    let manifest =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();

    assert!(!manifest.contains("rock-lib"));
}

#[test]
fn test_build_project_with_transitive_artifacts() {
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = temp_test_dir("transitive_artifacts");
    let dep_a = temp_dir.join("dep_a");
    let dep_b = temp_dir.join("dep_b");
    let app = temp_dir.join("app");

    write_package(
        &dep_a,
        "dep_a",
        "lib.rk",
        &[],
        "answer = x -> x\n< answer\n",
    );
    write_package(
        &dep_b,
        "dep_b",
        "lib.rk",
        &[("dep_a", dep_a.display().to_string())],
        "> dep_a::answer\n\nrelay = x -> answer x\n< relay\n",
    );
    write_package(
        &app,
        "app",
        "src/main.rk",
        &[("dep_b", dep_b.display().to_string())],
        "> dep_b::relay\n\nmain = -> relay 42\n",
    );

    let executable = build_project(&app).unwrap();
    assert!(executable.exists());
    let dep_a_package = load_package(dep_a.clone());
    let dep_b_package = load_package(dep_b.clone());
    assert!(dep_a_package.artifact_path().exists());
    assert!(dep_b_package.artifact_path().exists());
    assert!(dep_a_package.object_path().exists());
    assert!(dep_b_package.object_path().exists());

    assert_executable_exit_code(&executable, 42);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_build_project_no_std_disables_implicit_std() {
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = temp_test_dir("no_std_opt_out");
    write_package_with_options(
        &temp_dir,
        "no_std_opt_out",
        "src/main.rk",
        &[],
        "main = -> max 41, 42\n",
        true,
    );

    let error = build_project(&temp_dir).unwrap_err();
    assert!(error.contains("root executable for crate 'no_std_opt_out'"));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_no_std_root_with_std_using_dependency_does_not_get_prelude() {
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = temp_test_dir("no_std_with_std_dep_no_prelude");
    let dep = temp_dir.join("dep");
    let app = temp_dir.join("app");

    write_package(
        &dep,
        "dep",
        "lib.rk",
        &[],
        "print_answer = -> 42.println!\n< print_answer\n",
    );
    write_package_with_options(
        &app,
        "app",
        "src/main.rk",
        &[("dep", dep.display().to_string())],
        "> dep::print_answer\n\nmain = -> max 41, 42\n",
        true,
    );

    let error = build_project(&app).unwrap_err();
    assert!(error.contains("root executable for crate 'app'"));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_no_std_root_with_no_std_dependency_does_not_load_explicit_empty_sysroot() {
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = temp_test_dir("no_std_dep_empty_sysroot");
    let explicit_sysroot = temp_dir.join("empty-sysroot");
    let dep = temp_dir.join("dep");
    let app = temp_dir.join("app");
    let previous = std::env::var_os("ROCK_SYSROOT");

    fs::create_dir_all(&explicit_sysroot).unwrap();
    write_package_with_options(
        &dep,
        "dep",
        "lib.rk",
        &[],
        "identity = x -> x\n< identity\n",
        true,
    );
    write_package_with_options(
        &app,
        "app",
        "src/main.rk",
        &[("dep", dep.display().to_string())],
        "> dep::identity\n\nmain = -> identity 11\n",
        true,
    );

    std::env::set_var("ROCK_SYSROOT", &explicit_sysroot);
    let executable = build_project(&app);

    match previous {
        Some(value) => std::env::set_var("ROCK_SYSROOT", value),
        None => std::env::remove_var("ROCK_SYSROOT"),
    }

    assert_executable_exit_code(&executable.unwrap(), 11);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_build_project_reports_missing_rockc_override() {
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = temp_test_dir("missing_rockc_override");
    write_package(
        &temp_dir,
        "missing_rockc",
        "src/main.rk",
        &[],
        "main = -> 0\n",
    );
    let previous = std::env::var_os("ROCKC");

    std::env::set_var("ROCKC", temp_dir.join("does-not-exist-rockc"));
    let result = build_project(&temp_dir);

    match previous {
        Some(value) => std::env::set_var("ROCKC", value),
        None => std::env::remove_var("ROCKC"),
    }

    let error = result.unwrap_err();
    assert!(error.contains("Failed to spawn rockc"));
    assert!(error.contains("root executable for crate 'missing_rockc'"));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_rock_run_propagates_exit_code() {
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = temp_test_dir("run_exit_code");
    write_package(&temp_dir, "run_app", "src/main.rk", &[], "main = -> 7\n");

    let exit_code = run_project(&temp_dir, &[]).unwrap();
    assert_eq!(exit_code, 7);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_rock_run_forwards_args_to_binary() {
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = temp_test_dir("run_args");
    write_package(
        &temp_dir,
        "run_args_app",
        "src/main.rk",
        &[],
        "main = -> 9\n",
    );

    let exit_code = run_project(&temp_dir, &["--flag".to_string(), "value".to_string()]).unwrap();
    assert_eq!(exit_code, 9);

    let _ = fs::remove_dir_all(temp_dir);
}

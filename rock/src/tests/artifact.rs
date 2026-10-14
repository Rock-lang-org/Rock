use std::fs;

use crate::artifact::{collect_dependency_artifacts, ensure_artifact, ArtifactBuildState};
use crate::build::build_project;

use super::support::{
    assert_executable_exit_code, assert_sysroot_std_files_exist, load_package, sysroot_env_lock,
    temp_test_dir, write_package, write_package_with_options,
};

#[test]
fn test_rock_artifact_cache_reuse_and_invalidation() {
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = temp_test_dir("artifact_cache");
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
    let dep_a_package = load_package(dep_a.clone());
    let dep_a_artifact = dep_a_package.artifact_path();
    let dep_a_object = dep_a_package.object_path();
    let dep_a_mtime_before = fs::metadata(&dep_a_artifact).unwrap().modified().unwrap();
    let dep_a_object_mtime_before = fs::metadata(&dep_a_object).unwrap().modified().unwrap();

    assert_executable_exit_code(&executable, 42);

    let _executable = build_project(&app).unwrap();
    let dep_a_mtime_after_no_change = fs::metadata(&dep_a_artifact).unwrap().modified().unwrap();
    let dep_a_object_mtime_after_no_change =
        fs::metadata(&dep_a_object).unwrap().modified().unwrap();
    assert_eq!(dep_a_mtime_before, dep_a_mtime_after_no_change);
    assert_eq!(
        dep_a_object_mtime_before,
        dep_a_object_mtime_after_no_change
    );

    fs::write(dep_a.join("lib.rk"), "answer = x -> 43\n< answer\n").unwrap();

    let executable = build_project(&app).unwrap();
    let dep_a_artifact_mtime_after_change =
        fs::metadata(&dep_a_artifact).unwrap().modified().unwrap();
    let dep_a_object_mtime_after_change = fs::metadata(&dep_a_object).unwrap().modified().unwrap();
    assert!(dep_a_artifact_mtime_after_change > dep_a_mtime_after_no_change);
    assert!(dep_a_object_mtime_after_change > dep_a_object_mtime_after_no_change);

    assert_executable_exit_code(&executable, 43);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_collect_dependency_artifacts_includes_transitive_dependencies() {
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = temp_test_dir("transitive_dependency_artifacts");
    let dep_a = temp_dir.join("dep_a");
    let dep_b = temp_dir.join("dep_b");
    let app = temp_dir.join("app");

    write_package_with_options(
        &dep_a,
        "dep_a",
        "lib.rk",
        &[],
        "answer = x -> x\n< answer\n",
        true,
    );
    write_package_with_options(
        &dep_b,
        "dep_b",
        "lib.rk",
        &[("dep_a", dep_a.display().to_string())],
        "> dep_a::answer\n\nrelay = x -> answer x\n< relay\n",
        true,
    );
    write_package_with_options(
        &app,
        "app",
        "src/main.rk",
        &[("dep_b", dep_b.display().to_string())],
        "> dep_b::relay\n\nmain = -> relay 42\n",
        true,
    );

    let package = load_package(app.clone());
    let mut state = ArtifactBuildState::default();
    let artifacts = collect_dependency_artifacts(&package, &mut state).unwrap();
    let artifact_names = artifacts
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(artifact_names, vec!["dep_a", "dep_b"]);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_rock_artifact_command_builds_cache_file() {
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = temp_test_dir("artifact_command");
    write_package(
        &temp_dir,
        "dep_only",
        "lib.rk",
        &[],
        "identity = x -> x\n< identity\n",
    );

    let mut state = ArtifactBuildState::default();
    let artifact_path = ensure_artifact(&temp_dir, &mut state).unwrap();

    assert!(artifact_path.exists());
    assert!(load_package(temp_dir.clone()).object_path().exists());
    assert_sysroot_std_files_exist();

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_missing_dependency_object_triggers_rebuild() {
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = temp_test_dir("missing_object_rebuild");
    let dep = temp_dir.join("dep");
    let app = temp_dir.join("app");

    write_package(&dep, "dep", "lib.rk", &[], "answer = x -> x\n< answer\n");
    write_package(
        &app,
        "app",
        "src/main.rk",
        &[("dep", dep.display().to_string())],
        "> dep::answer\n\nmain = -> answer 5\n",
    );

    let executable = build_project(&app).unwrap();
    let dep_package = load_package(dep.clone());
    let dep_object = dep_package.object_path();
    assert!(dep_object.exists());
    assert_executable_exit_code(&executable, 5);

    fs::remove_file(&dep_object).unwrap();
    assert!(!dep_object.exists());

    let executable = build_project(&app).unwrap();
    assert!(dep_object.exists());
    assert_executable_exit_code(&executable, 5);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_missing_product_artifact_triggers_rebuild() {
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = temp_test_dir("missing_product_artifact_rebuild");
    let dep = temp_dir.join("dep");
    let app = temp_dir.join("app");

    write_package(&dep, "dep", "lib.rk", &[], "answer = x -> x\n< answer\n");
    write_package(
        &app,
        "app",
        "src/main.rk",
        &[("dep", dep.display().to_string())],
        "> dep::answer\n\nmain = -> answer 6\n",
    );

    let executable = build_project(&app).unwrap();
    let dep_package = load_package(dep.clone());
    let dep_artifact = dep_package.artifact_path();
    assert!(dep_artifact.exists());
    assert_executable_exit_code(&executable, 6);

    fs::remove_file(&dep_artifact).unwrap();
    assert!(!dep_artifact.exists());

    let executable = build_project(&app).unwrap();
    assert!(dep_artifact.exists());
    assert_executable_exit_code(&executable, 6);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_source_change_after_artifact_overwrite_triggers_rebuild() {
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = temp_test_dir("artifact_overwrite_source_rebuild");
    let dep = temp_dir.join("dep");
    let app = temp_dir.join("app");

    write_package(&dep, "dep", "lib.rk", &[], "answer = -> 7\n< answer\n");
    write_package(
        &app,
        "app",
        "src/main.rk",
        &[("dep", dep.display().to_string())],
        "> dep::answer\n\nmain = -> answer!\n",
    );

    let executable = build_project(&app).unwrap();
    let dep_package = load_package(dep.clone());
    let dep_artifact = dep_package.artifact_path();
    assert!(dep_artifact.exists());
    assert_executable_exit_code(&executable, 7);

    fs::write(&dep_artifact, b"not a product artifact").unwrap();
    fs::write(dep.join("lib.rk"), "answer = -> 8\n< answer\n").unwrap();

    let executable = build_project(&app).unwrap();
    assert!(dep_artifact.exists());
    assert_executable_exit_code(&executable, 8);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_corrupt_product_artifact_without_source_change_triggers_rebuild() {
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = temp_test_dir("corrupt_product_artifact_rebuild");
    let dep = temp_dir.join("dep");
    let app = temp_dir.join("app");

    write_package(&dep, "dep", "lib.rk", &[], "answer = -> 7\n< answer\n");
    write_package(
        &app,
        "app",
        "src/main.rk",
        &[("dep", dep.display().to_string())],
        "> dep::answer\n\nmain = -> answer!\n",
    );

    let executable = build_project(&app).unwrap();
    let dep_package = load_package(dep.clone());
    let dep_artifact = dep_package.artifact_path();
    assert!(dep_artifact.exists());
    assert_executable_exit_code(&executable, 7);

    fs::write(&dep_artifact, b"not a product artifact").unwrap();

    let executable = build_project(&app).unwrap();
    assert!(dep_artifact.exists());
    assert_executable_exit_code(&executable, 7);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_ensure_artifact_reuses_dependency_artifacts_after_dependency_sources_are_deleted() {
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = temp_test_dir("artifact_dependency_bootstrap");
    let dep = temp_dir.join("dep");
    let app = temp_dir.join("app");

    write_package(&dep, "dep", "lib.rk", &[], "answer = -> 5\n< answer\n");
    write_package(
        &app,
        "app",
        "src/main.rk",
        &[("dep", dep.display().to_string())],
        "> dep::answer\n\nmain = -> answer!\n",
    );

    let mut state = ArtifactBuildState::default();
    let dep_artifact = ensure_artifact(&dep, &mut state).unwrap();
    assert!(dep_artifact.exists());

    fs::remove_file(dep.join("lib.rk")).unwrap();
    fs::remove_file(dep.join("rock.toml")).unwrap();

    let app_artifact = ensure_artifact(&app, &mut state).unwrap();
    assert!(app_artifact.exists());

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_ensure_artifact_loads_sysroot_std_for_no_std_root_with_std_using_dependency() {
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = temp_test_dir("artifact_dependency_sysroot_std");
    let dep = temp_dir.join("dep");
    let app = temp_dir.join("app");

    write_package(
        &dep,
        "dep",
        "lib.rk",
        &[],
        "answer = -> Option::Some 5\n< answer\n",
    );
    write_package_with_options(
        &app,
        "app",
        "src/main.rk",
        &[("dep", dep.display().to_string())],
        "> dep::answer\n\nmain = -> 0\n",
        true,
    );

    let mut state = ArtifactBuildState::default();
    let artifact = ensure_artifact(&app, &mut state).unwrap();
    assert!(artifact.exists());

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_build_project_reuses_source_free_artifacts_without_dependency_sources() {
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = temp_test_dir("source_free_artifact_reuse");
    let dep = temp_dir.join("dep");
    let app = temp_dir.join("app");

    write_package(&dep, "dep", "lib.rk", &[], "answer = -> 5\n< answer\n");
    write_package(
        &app,
        "app",
        "src/main.rk",
        &[("dep", dep.display().to_string())],
        "> dep::answer\n\nmain = -> answer!\n",
    );

    let executable = build_project(&app).unwrap();
    let dep_package = load_package(dep.clone());
    assert!(dep_package.artifact_path().exists());
    assert!(dep_package.object_path().exists());

    assert_executable_exit_code(&executable, 5);

    fs::remove_file(dep.join("lib.rk")).unwrap();

    let executable = build_project(&app).unwrap();
    assert!(dep_package.artifact_path().exists());
    assert!(dep_package.object_path().exists());
    assert_executable_exit_code(&executable, 5);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_artifact_consumer_calls_dependency_qualified_impl_owner() {
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = temp_test_dir("artifact_dependency_qualified_impl_owner");
    let dep = temp_dir.join("dep");
    let app = temp_dir.join("app");

    write_package(
        &dep,
        "dep",
        "lib.rk",
        &[],
        "struct DepBox T
    value: T
< DepBox

impl DepBox T
    new = value ->
        DepBox
            value: value

trait Answer T
    @get: T
    @answer: T
    @answer = -> self.get!
< Answer

impl Answer T for DepBox T
    @get = -> @value
",
    );
    write_package(
        &app,
        "app",
        "src/main.rk",
        &[("dep", dep.display().to_string())],
        "> dep::DepBox
> dep::Answer

struct Box
    value: I64

main = ->
    b = DepBox::new 23
    b.answer!
",
    );

    let executable = build_project(&app).unwrap();
    assert_executable_exit_code(&executable, 23);

    fs::remove_file(dep.join("lib.rk")).unwrap();

    let executable = build_project(&app).unwrap();
    assert_executable_exit_code(&executable, 23);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_artifact_consumer_uses_generic_impl_inherited_default_method() {
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = temp_test_dir("artifact_generic_inherited_default");
    let dep = temp_dir.join("dep");
    let app = temp_dir.join("app");

    write_package(
        &dep,
        "dep",
        "lib.rk",
        &[],
        "struct DepBox T
    value: T
< DepBox

impl DepBox T
    new = value ->
        DepBox
            value: value

trait Echo T
    @echo: T -> T
    @echo = value -> value
< Echo

impl Echo T for DepBox T
",
    );
    write_package(
        &app,
        "app",
        "src/main.rk",
        &[("dep", dep.display().to_string())],
        "> dep::DepBox
> dep::Echo

main = ->
    b = DepBox::new 0
    b.echo 31
",
    );

    let executable = build_project(&app).unwrap();
    assert_executable_exit_code(&executable, 31);

    fs::remove_file(dep.join("lib.rk")).unwrap();

    let executable = build_project(&app).unwrap();
    assert_executable_exit_code(&executable, 31);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_artifact_consumer_uses_concrete_object_backed_inherited_default() {
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = temp_test_dir("artifact_concrete_object_backed_default");
    let dep = temp_dir.join("dep");
    let app = temp_dir.join("app");

    write_package(
        &dep,
        "dep",
        "lib.rk",
        &[],
        "struct Token
< Token

trait Code
    @code: I64
    @code = -> 44
< Code

impl Code for Token
",
    );
    write_package(
        &app,
        "app",
        "src/main.rk",
        &[("dep", dep.display().to_string())],
        "> dep::Token
> dep::Code

main = ->
    token = Token
    token.code!
",
    );

    let executable = build_project(&app).unwrap();
    let dep_package = load_package(dep.clone());
    assert!(dep_package.artifact_path().exists());
    assert!(dep_package.object_path().exists());
    assert_executable_exit_code(&executable, 44);

    fs::remove_file(dep.join("lib.rk")).unwrap();

    let executable = build_project(&app).unwrap();
    assert!(dep_package.artifact_path().exists());
    assert!(dep_package.object_path().exists());
    assert_executable_exit_code(&executable, 44);

    let _ = fs::remove_dir_all(temp_dir);
}

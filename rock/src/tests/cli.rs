use std::fs;

use crate::{
    cli::{Command, Config},
    commands::{expand_project, format_project},
    package::Package,
};

use super::support::{sysroot_env_lock, temp_test_dir, write_package};

#[test]
fn test_new_command_requires_project_name() {
    let config = <Config as clap::Parser>::try_parse_from(["rock", "new", "hello-rock"]).unwrap();
    assert!(
        matches!(config.command, Command::New { project_name } if project_name == "hello-rock")
    );
    assert!(<Config as clap::Parser>::try_parse_from(["rock", "new"]).is_err());
}

#[test]
fn test_run_command_parses_trailing_args() {
    let config =
        <Config as clap::Parser>::try_parse_from(["rock", "run", "--", "--flag", "value"]).unwrap();

    match config.command {
        Command::Run { args } => {
            assert_eq!(args, vec!["--flag".to_string(), "value".to_string()]);
        }
        command => panic!("Expected run command, got {:?}", command),
    }
}

#[test]
fn test_unimplemented_test_command_is_not_exposed() {
    assert!(<Config as clap::Parser>::try_parse_from(["rock", "test"]).is_err());
}

#[test]
fn test_format_project_formats_entry_file_with_rockc() {
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = temp_test_dir("format_project");
    write_package(
        &temp_dir,
        "format_app",
        "src/main.rk",
        &[],
        "main  =  ->\n    0\n",
    );
    let entry_file = temp_dir.join("src/main.rk");
    let previous_dir = std::env::current_dir().unwrap();

    std::env::set_current_dir(&temp_dir).unwrap();
    let result = format_project();
    std::env::set_current_dir(previous_dir).unwrap();

    result.unwrap();
    let formatted = fs::read_to_string(&entry_file).unwrap();
    assert!(formatted.contains("main = ->"));
    assert_ne!(formatted, "main  =  ->\n    0\n");

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_expand_project_with_dependency_does_not_require_artifacts() {
    let _guard = sysroot_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = temp_test_dir("expand_project_dep");
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
    let dep_package = Package::load(dep.clone()).unwrap();
    let previous_dir = std::env::current_dir().unwrap();

    std::env::set_current_dir(&app).unwrap();
    let result = expand_project();
    std::env::set_current_dir(previous_dir).unwrap();

    result.unwrap();
    assert!(!dep_package.artifact_path().exists());

    let _ = fs::remove_dir_all(temp_dir);
}

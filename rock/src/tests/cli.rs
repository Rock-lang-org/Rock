use std::fs;

use crate::{
    cli::{Command, Config},
    commands::{expand_project, format_project},
    package::{BuildProfile, Package},
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
        Command::Run { args, .. } => {
            assert_eq!(args, vec!["--flag".to_string(), "value".to_string()]);
        }
        command => panic!("Expected run command, got {:?}", command),
    }
}

#[test]
fn test_build_profiles_parse_for_build_run_and_artifact() {
    for command in ["build", "run", "artifact"] {
        for (flags, expected) in [
            (vec![], BuildProfile::Debug),
            (vec!["--debug"], BuildProfile::Debug),
            (vec!["--release"], BuildProfile::Release),
        ] {
            let mut args = vec!["rock", command];
            args.extend(flags);
            let config = <Config as clap::Parser>::try_parse_from(args).unwrap();
            let profile = match config.command {
                Command::Build { profile } | Command::Run { profile, .. } | Command::Artifact { profile } => profile,
                other => panic!("Expected build profile, got {other:?}"),
            };
            assert_eq!(profile.selected(), expected);
        }
        for flags in [
            vec!["--debug", "--release"],
            vec!["--release", "--debug"],
        ] {
            let mut args = vec!["rock", command];
            args.extend(flags);
            assert!(<Config as clap::Parser>::try_parse_from(args).is_err());
        }
    }
    let config = <Config as clap::Parser>::try_parse_from([
        "rock", "run", "--release", "--", "--debug", "--release",
    ]).unwrap();
    match config.command {
        Command::Run { profile, args } => {
            assert_eq!(profile.selected(), BuildProfile::Release);
            assert_eq!(args, ["--debug", "--release"]);
        }
        other => panic!("Expected run command, got {other:?}"),
    }
}

#[test]
fn test_numeric_optimization_flags_are_unknown() {
    for command in [None, Some("build"), Some("run"), Some("artifact")] {
        for flags in [
            vec!["-O"],
            vec!["-O", "0"],
            vec!["-O", "3"],
            vec!["-O0"],
            vec!["-O1"],
            vec!["-O2"],
            vec!["-O3"],
            vec!["--opt-level"],
            vec!["--opt-level", "3"],
            vec!["--opt-level=0"],
            vec!["--opt-level=3"],
        ] {
            let mut args = vec!["rock"];
            args.extend(command);
            args.extend(flags);
            let error = <Config as clap::Parser>::try_parse_from(args).unwrap_err();
            assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
        }
        let mut cli = <Config as clap::CommandFactory>::command();
        let help = match command {
            Some(name) => cli.find_subcommand_mut(name).unwrap().render_long_help(),
            None => cli.render_long_help(),
        }.to_string();
        assert!(!help.contains("-O"));
        assert!(!help.contains("--opt-level"));
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

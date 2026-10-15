use std::{fs, process::Command};

use crate::{
    cli::{Command as CliCommand, Config},
    commands::clean_project,
    rockc::{ensure_dev_binary_built, resolve_rockc_path},
};

use super::support::{run_command_output, sysroot_env_lock, temp_test_dir, write_package_with_options};

#[test]
fn test_clean_command_accepts_no_build_profile_flags() {
    let config = <Config as clap::Parser>::try_parse_from(["rock", "clean"]).unwrap();
    assert!(matches!(config.command, CliCommand::Clean));
    for flag in ["--debug", "--release"] {
        assert!(<Config as clap::Parser>::try_parse_from(["rock", "clean", flag]).is_err());
    }
}

#[test]
fn test_clean_removes_all_profiles_but_preserves_sources_and_other_caches() {
    let root = temp_test_dir("clean_profiles");
    let app = root.join("app");
    let dependency = root.join("dependency");
    write_package_with_options(&app, "app", "src/main.rk", &[], "main = !-> ()\n", true);
    fs::create_dir_all(dependency.join("build")).unwrap();
    fs::write(dependency.join("build/keep"), "other project's cache").unwrap();
    for profile in ["debug", "release"] {
        let directory = app.join("build").join(profile).join("deps/dep/objects");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("lib.o"), "object").unwrap();
    }
    assert!(clean_project(&app).unwrap());
    assert!(!app.join("build").exists());
    assert!(app.join("rock.toml").exists());
    assert_eq!(fs::read_to_string(app.join("src/main.rk")).unwrap(), "main = !-> ()\n");
    assert!(dependency.join("build/keep").exists());
    assert!(!clean_project(&app).unwrap());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn test_clean_requires_a_project_and_refuses_a_build_file() {
    let root = temp_test_dir("clean_validation");
    fs::create_dir_all(root.join("build")).unwrap();
    fs::write(root.join("build/keep"), "not a Rock project's build").unwrap();
    assert!(clean_project(&root).is_err());
    assert!(root.join("build/keep").exists());
    fs::remove_dir_all(root.join("build")).unwrap();
    write_package_with_options(&root, "app", "src/main.rk", &[], "main = !-> ()\n", true);
    fs::write(root.join("build"), "not a directory").unwrap();
    assert!(clean_project(&root).unwrap_err().contains("Refusing to clean"));
    assert_eq!(fs::read_to_string(root.join("build")).unwrap(), "not a directory");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn test_clean_does_not_follow_build_symlinks() {
    use std::os::unix::fs::symlink;

    let root = temp_test_dir("clean_symlinks");
    let app = root.join("app");
    let outside = root.join("outside");
    write_package_with_options(&app, "app", "src/main.rk", &[], "main = !-> ()\n", true);
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("keep"), "preserve").unwrap();
    symlink(&outside, app.join("build")).unwrap();
    assert!(clean_project(&app).unwrap_err().contains("Refusing to clean"));
    assert!(outside.join("keep").exists());
    fs::remove_file(app.join("build")).unwrap();
    fs::create_dir_all(app.join("build")).unwrap();
    symlink(&outside, app.join("build/link")).unwrap();
    assert!(clean_project(&app).unwrap());
    assert!(outside.join("keep").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn test_cli_clean_removes_central_dependency_cache_and_rebuilds() {
    let _guard = sysroot_env_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = temp_test_dir("cli_clean_rebuild");
    let dep = root.join("dep");
    let app = root.join("app");
    write_package_with_options(&dep, "dep", "lib.rk", &[], "< answer = -> 42\n", true);
    write_package_with_options(
        &app, "app", "src/main.rk", &[("dep", dep.display().to_string())],
        "> dep::answer\nmain = -> answer!\n", true,
    );
    ensure_dev_binary_built("rock").unwrap();
    let rock = rock_shared::process::dev_target_binary_from_current_exe("rock").unwrap();
    let rockc = resolve_rockc_path().unwrap();
    for profile in ["--debug", "--release"] {
        let output = run_command_output(Command::new(&rock).current_dir(&app).args(["build", profile]).env("ROCKC", &rockc));
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert!(app.join("build").join(profile.trim_start_matches("--")).join("deps").exists());
        assert!(!dep.join("build").exists());
    }
    for expected in ["removed", "already clean"] {
        let output = run_command_output(
            Command::new(&rock).current_dir(&app).arg("clean")
                .env("ROCKC", root.join("missing-rockc"))
                .env("ROCK_SYSROOT", root.join("missing-sysroot")),
        );
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert!(String::from_utf8_lossy(&output.stderr).contains(expected));
        assert!(output.stdout.is_empty());
        assert!(!app.join("build").exists());
        assert!(dep.join("lib.rk").exists());
    }
    let output = run_command_output(Command::new(&rock).current_dir(&app).arg("build").env("ROCKC", &rockc));
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stderr).contains("◆ compile dep"));
    assert!(!dep.join("build").exists());
    fs::remove_dir_all(root).unwrap();
}

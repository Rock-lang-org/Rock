use std::fs;

use super::support::{build_project, run_project};
use crate::package::BuildProfile;

use super::support::{
    assert_executable_exit_code, load_dependency_package, load_package, run_command_output, run_isolated_test, temp_test_dir, write_package,
    write_package_with_options,
};

fn write_multi_target_package(root: &std::path::Path, library: bool) {
    fs::write(root.join("rock.toml"), format!(
        "[crate]\nname = \"mixed\"\nversion = \"0.1.0\"\nno_std = true\n\n{}\n[[bin]]\nname = \"server\"\npath = \"server/main.rk\"\n\n[[bin]]\nname = \"client\"\npath = \"client/main.rk\"\n",
        if library { "[lib]\npath = \"src/lib.rk\"\n" } else { "" },
    )).unwrap();
    if library {
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rk"), "< answer = -> 40\n< other = -> 41\n").unwrap();
    }
    for (name, value, function) in [("server", 40, "answer"), ("client", 41, "other")] {
        fs::create_dir_all(root.join(name)).unwrap();
        let source = if library {
            format!("> mixed::{}\nmain = -> {}!\n", function, function)
        } else {
            format!("main = -> {}\n", value)
        };
        fs::write(root.join(name).join("main.rk"), source).unwrap();
    }
}

#[test]
fn test_multiple_binaries_with_or_without_a_library_build_in_both_profiles() {
    for library in [false, true] {
        let root = temp_test_dir("multiple_targets");
        write_multi_target_package(&root, library);
        for profile in [BuildProfile::Debug, BuildProfile::Release] {
            let products = crate::build::build_project(&root, profile).unwrap();
            assert_eq!(products.library.is_some(), library);
            assert_eq!(products.binaries.len(), 2);
            for (binary, expected) in products.binaries.iter().zip([40, 41]) {
                assert_eq!(binary.path, root.join("build").join(profile.directory()).join(&binary.name));
                assert_executable_exit_code(&binary.path, expected);
                assert_eq!(crate::build::run_project(&root, profile, &[], Some(&binary.name)).unwrap(), expected);
            }
        }
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn test_run_requires_a_known_binary_when_multiple_targets_exist() {
    let root = temp_test_dir("multiple_target_selection");
    write_multi_target_package(&root, true);
    for (name, message) in [(None, "rock run --bin <name>"), (Some("unknown"), "No binary named 'unknown'")] {
        let error = crate::build::run_project(&root, BuildProfile::Debug, &[], name).unwrap_err();
        assert!(error.contains(message), "{error}");
        assert!(!root.join("build").exists());
    }
    fs::write(root.join("client/main.rk"), "main = -> missing_name\n").unwrap();
    assert_eq!(crate::build::run_project(&root, BuildProfile::Debug, &[], Some("server")).unwrap(), 40);
    assert!(!root.join("build/debug/client").exists());
    assert!(crate::build::build_project(&root, BuildProfile::Debug).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn test_mixed_dependency_only_builds_its_library() {
    let root = temp_test_dir("mixed_dependency");
    let dep = root.join("dep");
    fs::create_dir_all(&dep).unwrap();
    write_multi_target_package(&dep, true);
    fs::write(dep.join("server/main.rk"), "main = -> missing_name\n").unwrap();
    let app = root.join("app");
    write_package_with_options(
        &app, "app", "src/main.rk", &[("mixed", dep.display().to_string())],
        "> mixed::answer\nmain = -> answer!\n", true,
    );
    let executable = build_project(&app, BuildProfile::Debug).unwrap();
    assert_executable_exit_code(&executable, 40);
    let package = load_dependency_package(dep.clone(), &app);
    assert!(package.artifact_path().exists());
    assert!(!package.build_dir().join("server").exists());
    assert!(!dep.join("build").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn test_mixed_package_with_one_binary_runs_without_a_selector() {
    let root = temp_test_dir("mixed_one_binary");
    write_package_with_options(&root, "mixed", "src/lib.rk", &[], "< answer = -> 42\n", true);
    let manifest = root.join("rock.toml");
    let text = fs::read_to_string(&manifest).unwrap();
    fs::write(manifest, format!("{}\n[[bin]]\n", text)).unwrap();
    fs::write(root.join("src/main.rk"), "> mixed::answer\nmain = -> answer!\n").unwrap();
    assert_eq!(run_project(&root, BuildProfile::Debug, &[]).unwrap(), 42);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn test_cli_build_lists_all_products_and_run_selects_a_binary() {
    let root = temp_test_dir("multi_target_cli");
    write_multi_target_package(&root, true);
    crate::rockc::ensure_dev_binary_built("rock").unwrap();
    let rock = rock_shared::process::dev_target_binary_from_current_exe("rock").unwrap();
    let output = run_command_output(std::process::Command::new(&rock).current_dir(&root).arg("build"));
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let paths = String::from_utf8(output.stdout).unwrap();
    assert_eq!(paths.lines().collect::<Vec<_>>(), [
        load_package(root.clone()).artifact_path().display().to_string(),
        root.join("build/debug/server").display().to_string(),
        root.join("build/debug/client").display().to_string(),
    ]);
    assert!(String::from_utf8_lossy(&output.stderr).contains("3/3 crates · 100%"));
    let output = run_command_output(std::process::Command::new(&rock).current_dir(&root).args(["run", "--bin", "client"]));
    assert_eq!(output.status.code(), Some(41), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stderr).contains("2/2 crates · 100%"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn test_library_builds_and_runs_without_linking_in_both_profiles() {
    let root = temp_test_dir("library_build_profiles");
    let dep = root.join("dep");
    let library = root.join("library");
    write_package_with_options(&dep, "dep", "src/lib.rk", &[], "< answer = -> 42\n", true);
    write_package_with_options(
        &library, "library", "src/lib.rk", &[("dep", dep.display().to_string())],
        "> dep::answer\n< relay = -> answer!\n", true,
    );
    for profile in [BuildProfile::Debug, BuildProfile::Release] {
        let mut package = load_package(library.clone());
        package.profile = profile;
        let artifact = build_project(&library, profile).unwrap();
        assert_eq!(artifact, package.artifact_path());
        assert!(artifact.is_file());
        assert!(package.object_path().is_file());
        assert!(!package.build_dir().join("lib").exists());
        let modified = fs::metadata(&artifact).unwrap().modified().unwrap();
        assert_eq!(run_project(&library, profile, &["unused".to_string()]).unwrap(), 0);
        assert_eq!(fs::metadata(&artifact).unwrap().modified().unwrap(), modified);
        let mut dependency = load_dependency_package(dep.clone(), &library);
        dependency.profile = profile;
        assert!(dependency.artifact_path().is_file());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn test_library_run_cli_builds_and_reports_no_binary() {
    let root = temp_test_dir("run_library_cli");
    write_package_with_options(&root, "library", "src/lib.rk", &[], "< answer = -> 42\n", true);
    crate::rockc::ensure_dev_binary_built("rock").unwrap();
    let rock = rock_shared::process::dev_target_binary_from_current_exe("rock").unwrap();
    let output = run_command_output(std::process::Command::new(rock)
        .current_dir(&root)
        .args(["run", "--release", "--", "unused"]));
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("there is no binary to execute"));
    let mut package = load_package(root.clone());
    package.profile = BuildProfile::Release;
    assert!(package.artifact_path().is_file());
    assert!(package.object_path().is_file());
    assert!(!package.build_dir().join("lib").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn test_library_run_propagates_build_failure() {
    let root = temp_test_dir("run_invalid_library");
    write_package_with_options(&root, "library", "src/lib.rk", &[], "< answer = -> undefined!\n", true);
    assert!(run_project(&root, BuildProfile::Debug, &[]).is_err());
    assert!(!load_package(root.clone()).artifact_path().exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn test_binary_crate_cannot_be_used_as_a_dependency() {
    let root = temp_test_dir("binary_dependency");
    let dep = root.join("dep");
    let app = root.join("app");
    write_package_with_options(&dep, "dep", "src/main.rk", &[], "main = -> 42\n", true);
    write_package_with_options(
        &app, "app", "src/main.rk", &[("dep", dep.display().to_string())], "main = -> 0\n", true,
    );
    let error = build_project(&app, BuildProfile::Debug).unwrap_err();
    assert!(error.contains("has no library target and cannot be used as a dependency"), "{error}");
    assert!(!load_dependency_package(dep, &app).artifact_path().exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn test_default_target_entries_build_and_missing_entries_fail() {
    for (section, path, source) in [
        ("bin", "src/main.rk", "main = -> 23\n"),
        ("lib", "src/lib.rk", "< answer = -> 23\n"),
    ] {
        let root = temp_test_dir("default_target_entry");
        fs::write(root.join("rock.toml"), format!(
            "[crate]\nname = \"example\"\nversion = \"0.1.0\"\nno_std = true\n\n{}\n", if section == "bin" { "[[bin]]" } else { "[lib]" },
        )).unwrap();
        let error = build_project(&root, BuildProfile::Debug).unwrap_err();
        assert!(error.contains(path), "{error}");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join(path), source).unwrap();
        let output = build_project(&root, BuildProfile::Debug).unwrap();
        assert!(output.is_file());
        if section == "bin" {
            assert_executable_exit_code(&output, 23);
        } else {
            assert_eq!(output, load_package(root.clone()).artifact_path());
        }
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn test_binary_target_requires_main_function() {
    let root = temp_test_dir("binary_missing_main");
    write_package_with_options(&root, "app", "src/main.rk", &[], "< answer = -> 42\n", true);
    let error = build_project(&root, BuildProfile::Debug).unwrap_err();
    assert!(error.contains("main"), "{error}");
    assert!(!root.join("build/debug/app").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn test_rock_cargo_toml_does_not_depend_on_rock_lib() {
    let manifest =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();

    assert!(!manifest.contains("rock-lib"));
}

#[test]
fn test_debug_and_release_builds_keep_transitive_dependency_caches_separate() {
    let temp_dir = temp_test_dir("build_profile_caches");
    let dep_a = temp_dir.join("dep_a");
    let dep_b = temp_dir.join("dep_b");
    let app = temp_dir.join("app");
    write_package_with_options(&dep_a, "dep_a", "lib.rk", &[], "< answer = -> 42\n", true);
    write_package_with_options(
        &dep_b, "dep_b", "lib.rk", &[("dep_a", dep_a.display().to_string())],
        "> dep_a::answer\n< relay = -> answer!\n", true,
    );
    write_package_with_options(
        &app, "app", "src/main.rk", &[("dep_b", dep_b.display().to_string())],
        "> dep_b::relay\nmain = -> relay!\n", true,
    );

    let mut cached_products = Vec::new();
    for profile in [BuildProfile::Debug, BuildProfile::Release] {
        let executable = build_project(&app, profile).unwrap();
        assert_eq!(executable, app.join("build").join(profile.directory()).join("app"));
        assert_executable_exit_code(&executable, 42);
        for root in [&dep_a, &dep_b] {
            let mut package = load_dependency_package(root.clone(), &app);
            package.profile = profile;
            assert!(package.build_dir().starts_with(app.join("build").join(profile.directory()).join("deps")));
            assert!(!root.join("build").exists());
            for path in [package.artifact_path(), package.object_path()] {
                assert!(path.exists(), "missing {}", path.display());
                cached_products.push((path.clone(), fs::metadata(path).unwrap().modified().unwrap()));
            }
        }
    }
    for profile in [BuildProfile::Debug, BuildProfile::Release] {
        assert_eq!(run_project(&app, profile, &[]).unwrap(), 42);
        for (path, modified) in &cached_products {
            assert_eq!(fs::metadata(path).unwrap().modified().unwrap(), *modified, "rebuilt {}", path.display());
        }
    }
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_build_project_with_transitive_artifacts() {
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

    let executable = build_project(&app, BuildProfile::Debug).unwrap();
    assert!(executable.exists());
    let dep_a_package = load_dependency_package(dep_a.clone(), &app);
    let dep_b_package = load_dependency_package(dep_b.clone(), &app);
    assert!(dep_a_package.artifact_path().exists());
    assert!(dep_b_package.artifact_path().exists());
    assert!(dep_a_package.object_path().exists());
    assert!(dep_b_package.object_path().exists());
    assert!(!dep_a.join("build").exists());
    assert!(!dep_b.join("build").exists());

    assert_executable_exit_code(&executable, 42);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_build_project_no_std_disables_implicit_std() {
    let temp_dir = temp_test_dir("no_std_opt_out");
    write_package_with_options(
        &temp_dir,
        "no_std_opt_out",
        "src/main.rk",
        &[],
        "main = -> max 41, 42\n",
        true,
    );

    let error = build_project(&temp_dir, BuildProfile::Debug).unwrap_err();
    assert!(error.contains("root executable for crate 'no_std_opt_out'"));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_no_std_root_with_std_using_dependency_does_not_get_prelude() {
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

    let error = build_project(&app, BuildProfile::Debug).unwrap_err();
    assert!(error.contains("root executable for crate 'app'"));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_no_std_root_with_no_std_dependency_does_not_load_explicit_empty_sysroot() {
    let temp_dir = temp_test_dir("no_std_dep_empty_sysroot");
    let explicit_sysroot = temp_dir.join("empty-sysroot");
    let dep = temp_dir.join("dep");
    let app = temp_dir.join("app");

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

    if run_isolated_test(
        "tests::build::test_no_std_root_with_no_std_dependency_does_not_load_explicit_empty_sysroot",
        &[("ROCK_SYSROOT", explicit_sysroot.as_os_str())],
    ) {
        fs::remove_dir_all(temp_dir).unwrap();
        return;
    }
    let executable = build_project(&app, BuildProfile::Debug);

    assert_executable_exit_code(&executable.unwrap(), 11);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_build_project_reports_missing_rockc_override() {
    let temp_dir = temp_test_dir("missing_rockc_override");
    write_package(
        &temp_dir,
        "missing_rockc",
        "src/main.rk",
        &[],
        "main = -> 0\n",
    );
    if run_isolated_test(
        "tests::build::test_build_project_reports_missing_rockc_override",
        &[("ROCKC", temp_dir.join("does-not-exist-rockc").as_os_str())],
    ) {
        fs::remove_dir_all(temp_dir).unwrap();
        return;
    }
    let result = build_project(&temp_dir, BuildProfile::Debug);

    let error = result.unwrap_err();
    assert!(error.contains("Failed to spawn rockc"));
    assert!(error.contains("root executable for crate 'missing_rockc'"));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_rock_run_propagates_exit_code() {
    let temp_dir = temp_test_dir("run_exit_code");
    write_package(&temp_dir, "run_app", "src/main.rk", &[], "main = -> 7\n");

    let exit_code = run_project(&temp_dir, BuildProfile::Debug, &[]).unwrap();
    assert_eq!(exit_code, 7);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_rock_run_forwards_args_to_binary() {
    let temp_dir = temp_test_dir("run_args");
    write_package(
        &temp_dir,
        "run_args_app",
        "src/main.rk",
        &[],
        "main = -> 9\n",
    );

    let exit_code = run_project(&temp_dir, BuildProfile::Debug, &["--flag".to_string(), "value".to_string()]).unwrap();
    assert_eq!(exit_code, 9);

    let _ = fs::remove_dir_all(temp_dir);
}

#[cfg(unix)]
#[test]
fn test_shared_build_does_not_write_to_dependency_sources() {
    use std::os::unix::fs::PermissionsExt;

    let root = temp_test_dir("readonly_dependency");
    let dep = root.join("dep");
    let app = root.join("app");
    write_package_with_options(&dep, "dep", "lib.rk", &[], "< answer = -> 42\n", true);
    write_package_with_options(
        &app, "app", "src/main.rk", &[("dep", dep.display().to_string())],
        "> dep::answer\nmain = -> answer!\n", true,
    );
    let permissions = fs::metadata(&dep).unwrap().permissions();
    fs::set_permissions(&dep, fs::Permissions::from_mode(0o555)).unwrap();
    let result = build_project(&app, BuildProfile::Debug);
    fs::set_permissions(&dep, permissions).unwrap();
    let executable = result.unwrap();
    assert_executable_exit_code(&executable, 42);
    assert!(!dep.join("build").exists());
    assert!(load_dependency_package(dep, &app).artifact_path().exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn test_shared_build_cache_distinguishes_same_name_and_version_at_different_paths() {
    let root = temp_test_dir("dependency_path_identity");
    let first = root.join("first");
    let second = root.join("second");
    let app = root.join("app");
    write_package_with_options(&first, "dep", "lib.rk", &[], "< answer = -> 42\n", true);
    write_package_with_options(&second, "dep", "lib.rk", &[], "< answer = -> 43\n", true);
    let mut products = Vec::new();
    for (dep, expected) in [(&first, 42), (&second, 43), (&first, 42)] {
        write_package_with_options(
            &app, "app", "src/main.rk", &[("dep", dep.display().to_string())],
            "> dep::answer\nmain = -> answer!\n", true,
        );
        let executable = build_project(&app, BuildProfile::Debug).unwrap();
        assert_executable_exit_code(&executable, expected);
        let package = load_dependency_package(dep.clone(), &app);
        assert!(package.artifact_path().exists());
        assert!(package.object_path().exists());
        assert!(!dep.join("build").exists());
        products.push(package.artifact_path());
    }
    assert_ne!(products[0], products[1]);
    assert_eq!(products[0], products[2]);
    fs::remove_dir_all(root).unwrap();
}

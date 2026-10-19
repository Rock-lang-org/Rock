use std::fs;

use crate::{
    constants::{
        BIN_DIR, DEFAULT_TOOLCHAIN_FILE, LIB_DIR, ROCKUP_TOOLCHAIN_ENV, ROCK_BIN_NAME,
        ROCK_LSP_BIN_NAME, STD_ARTIFACT_NAME, TOOLCHAINS_DIR,
    },
    layout::host_target_triple,
    selection::active_toolchain_name,
    toolchain::{
        detect_install_source, install_toolchain, list_toolchains, proxy_toolchain_command,
        proxy_toolchain_command_from_dir, remove_toolchain, run_toolchain_command,
        set_default_toolchain, InstallSourceKind,
    },
};

use super::support::{
    fake_home, read_default_toolchain, temp_test_dir, write_fake_cargo_build_dir,
    write_fake_toolchain_root, write_project_toolchain, write_script,
};

#[test]
fn test_detect_install_source_accepts_toolchain_root() {
    let dir = temp_test_dir("toolchain_root_source");
    write_fake_toolchain_root(&dir);

    let source = detect_install_source(&dir).unwrap();
    assert!(matches!(source.kind, InstallSourceKind::ToolchainRoot));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn test_detect_install_source_accepts_cargo_build_dir() {
    let dir = temp_test_dir("cargo_build_source");
    write_fake_cargo_build_dir(&dir);

    let source = detect_install_source(&dir).unwrap();
    assert!(matches!(source.kind, InstallSourceKind::CargoBuildDir));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn test_install_toolchain_normalizes_cargo_build_dir_layout() {
    let temp_dir = temp_test_dir("install_toolchain");
    let home = fake_home(temp_dir.join("home"));
    let source = temp_dir.join("source");
    write_fake_cargo_build_dir(&source);

    let installed = install_toolchain(&home, "stable", &source).unwrap();

    assert!(installed.join(BIN_DIR).join(ROCK_BIN_NAME).exists());
    assert!(installed
        .join(BIN_DIR)
        .join(crate::constants::ROCKC_BIN_NAME)
        .exists());
    assert!(installed.join(BIN_DIR).join(ROCK_LSP_BIN_NAME).exists());
    assert!(installed
        .join(LIB_DIR)
        .join("rocklib")
        .join(host_target_triple())
        .join(STD_ARTIFACT_NAME)
        .exists());
    assert!(home.root.join(BIN_DIR).join(ROCK_BIN_NAME).exists());
    assert!(home
        .root
        .join(BIN_DIR)
        .join(crate::constants::ROCKC_BIN_NAME)
        .exists());
    assert!(home.root.join(BIN_DIR).join(ROCK_LSP_BIN_NAME).exists());

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_install_toolchain_sets_default_for_first_install() {
    let temp_dir = temp_test_dir("install_sets_default");
    let home = fake_home(temp_dir.join("home"));
    let source = temp_dir.join("source");
    write_fake_toolchain_root(&source);

    install_toolchain(&home, "stable", &source).unwrap();
    assert_eq!(read_default_toolchain(&home), "stable");

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_install_toolchain_keeps_existing_default() {
    let temp_dir = temp_test_dir("install_keeps_default");
    let home = fake_home(temp_dir.join("home"));
    let stable_source = temp_dir.join("stable_source");
    let nightly_source = temp_dir.join("nightly_source");
    write_fake_toolchain_root(&stable_source);
    write_fake_toolchain_root(&nightly_source);

    install_toolchain(&home, "stable", &stable_source).unwrap();
    install_toolchain(&home, "nightly", &nightly_source).unwrap();
    assert_eq!(read_default_toolchain(&home), "stable");

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_remove_toolchain_deletes_installed_dir() {
    let temp_dir = temp_test_dir("remove_toolchain_dir");
    let home = fake_home(temp_dir.join("home"));
    let source = temp_dir.join("source");
    write_fake_toolchain_root(&source);

    let installed = install_toolchain(&home, "stable", &source).unwrap();
    let removed = remove_toolchain(&home, "stable").unwrap();
    assert_eq!(removed, installed);
    assert!(!removed.exists());

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_remove_toolchain_promotes_next_default() {
    let temp_dir = temp_test_dir("remove_toolchain_promotes_default");
    let home = fake_home(temp_dir.join("home"));
    let stable_source = temp_dir.join("stable_source");
    let nightly_source = temp_dir.join("nightly_source");
    write_fake_toolchain_root(&stable_source);
    write_fake_toolchain_root(&nightly_source);

    install_toolchain(&home, "stable", &stable_source).unwrap();
    install_toolchain(&home, "nightly", &nightly_source).unwrap();
    remove_toolchain(&home, "stable").unwrap();
    assert_eq!(read_default_toolchain(&home), "nightly");

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_remove_toolchain_clears_default_when_last_is_removed() {
    let temp_dir = temp_test_dir("remove_toolchain_clears_default");
    let home = fake_home(temp_dir.join("home"));
    let source = temp_dir.join("source");
    write_fake_toolchain_root(&source);

    install_toolchain(&home, "stable", &source).unwrap();
    remove_toolchain(&home, "stable").unwrap();
    assert!(!home.root.join(DEFAULT_TOOLCHAIN_FILE).exists());

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_active_toolchain_prefers_env_override() {
    let temp_dir = temp_test_dir("active_toolchain_env");
    let home = fake_home(temp_dir.clone());
    fs::create_dir_all(&home.root).unwrap();
    fs::write(home.root.join(DEFAULT_TOOLCHAIN_FILE), "stable\n").unwrap();

    let active = active_toolchain_name(&home, Some("nightly".to_string()), None).unwrap();
    assert_eq!(active, "nightly");

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_active_toolchain_prefers_project_pin_over_default() {
    let temp_dir = temp_test_dir("active_toolchain_project");
    let home = fake_home(temp_dir.join("home"));
    let project_dir = temp_dir.join("project");
    fs::create_dir_all(&home.root).unwrap();
    fs::create_dir_all(&project_dir).unwrap();
    fs::write(home.root.join(DEFAULT_TOOLCHAIN_FILE), "stable\n").unwrap();
    write_project_toolchain(&project_dir, "nightly");

    let active = active_toolchain_name(&home, None, Some(project_dir.as_path())).unwrap();
    assert_eq!(active, "nightly");

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_active_toolchain_uses_nearest_project_pin() {
    let temp_dir = temp_test_dir("active_toolchain_nearest_project");
    let home = fake_home(temp_dir.join("home"));
    let project_dir = temp_dir.join("project");
    let nested_dir = project_dir.join("crates").join("app");
    fs::create_dir_all(&home.root).unwrap();
    fs::create_dir_all(&nested_dir).unwrap();
    fs::write(home.root.join(DEFAULT_TOOLCHAIN_FILE), "stable\n").unwrap();
    write_project_toolchain(&project_dir, "nightly");
    write_project_toolchain(&nested_dir, "dev");

    let active = active_toolchain_name(&home, None, Some(nested_dir.as_path())).unwrap();
    assert_eq!(active, "dev");

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_list_toolchains_marks_active_default() {
    let temp_dir = temp_test_dir("list_toolchains");
    let home = fake_home(temp_dir.join("home"));
    write_fake_toolchain_root(&home.root.join(TOOLCHAINS_DIR).join("stable"));
    write_fake_toolchain_root(&home.root.join(TOOLCHAINS_DIR).join("nightly"));
    set_default_toolchain(&home, "stable").unwrap();

    let toolchains = list_toolchains(&home).unwrap();
    assert_eq!(toolchains.len(), 2);
    assert!(toolchains
        .iter()
        .any(|entry| entry.name == "stable" && entry.is_active));
    assert!(toolchains
        .iter()
        .any(|entry| entry.name == "nightly" && !entry.is_active));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_run_toolchain_command_prefers_toolchain_bin_in_path() {
    let temp_dir = temp_test_dir("run_toolchain_command");
    let home = fake_home(temp_dir.join("home"));
    let source = temp_dir.join("source");
    let log_path = temp_dir.join("run.log");

    write_fake_toolchain_root(&source);
    write_script(
        &source.join(BIN_DIR).join(ROCK_BIN_NAME),
        &format!("printf '%s' \"$1\" > {}\nexit 23\n", log_path.display()),
    );
    install_toolchain(&home, "stable", &source).unwrap();

    let status =
        run_toolchain_command(&home, "stable", &["rock".to_string(), "build".to_string()]).unwrap();
    assert_eq!(status.code(), Some(23));
    assert_eq!(fs::read_to_string(log_path).unwrap(), "build");

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_proxy_toolchain_command_uses_default_selection() {
    let temp_dir = temp_test_dir("proxy_command");
    let home = fake_home(temp_dir.join("home"));
    let source = temp_dir.join("source");
    write_fake_toolchain_root(&source);
    install_toolchain(&home, "stable", &source).unwrap();
    set_default_toolchain(&home, "stable").unwrap();

    let status = proxy_toolchain_command(&home, ROCK_BIN_NAME, &[]).unwrap();
    assert_eq!(status.code(), Some(17));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_proxy_toolchain_command_supports_rock_lsp() {
    let temp_dir = temp_test_dir("proxy_lsp_command");
    let home = fake_home(temp_dir.join("home"));
    let source = temp_dir.join("source");
    write_fake_toolchain_root(&source);
    install_toolchain(&home, "stable", &source).unwrap();
    set_default_toolchain(&home, "stable").unwrap();

    let status = proxy_toolchain_command(&home, ROCK_LSP_BIN_NAME, &[]).unwrap();
    assert_eq!(status.code(), Some(31));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_proxy_toolchain_command_uses_project_pin() {
    let temp_dir = temp_test_dir("proxy_project_pin");
    let home = fake_home(temp_dir.join("home"));
    let stable_source = temp_dir.join("stable_source");
    let nightly_source = temp_dir.join("nightly_source");
    let project_dir = temp_dir.join("project");

    write_fake_toolchain_root(&stable_source);
    write_fake_toolchain_root(&nightly_source);
    write_script(
        &stable_source.join(BIN_DIR).join(ROCK_BIN_NAME),
        "exit 17\n",
    );
    write_script(
        &nightly_source.join(BIN_DIR).join(ROCK_BIN_NAME),
        "exit 29\n",
    );
    install_toolchain(&home, "stable", &stable_source).unwrap();
    install_toolchain(&home, "nightly", &nightly_source).unwrap();
    set_default_toolchain(&home, "stable").unwrap();
    fs::create_dir_all(&project_dir).unwrap();
    write_project_toolchain(&project_dir, "nightly");

    let status =
        proxy_toolchain_command_from_dir(&home, ROCK_BIN_NAME, &[], project_dir.as_path()).unwrap();
    assert_eq!(status.code(), Some(29));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_run_command_sets_env_selected_toolchain() {
    let temp_dir = temp_test_dir("run_env_toolchain");
    let home = fake_home(temp_dir.join("home"));
    let source = temp_dir.join("source");
    let log_path = temp_dir.join("env.log");

    write_fake_toolchain_root(&source);
    write_script(
        &source.join(BIN_DIR).join(ROCK_BIN_NAME),
        &format!(
            "printf '%s' \"${}\" > {}\nexit 0\n",
            ROCKUP_TOOLCHAIN_ENV,
            log_path.display()
        ),
    );
    install_toolchain(&home, "stable", &source).unwrap();

    let status = run_toolchain_command(&home, "stable", &[ROCK_BIN_NAME.to_string()]).unwrap();
    assert!(status.success());
    assert_eq!(fs::read_to_string(log_path).unwrap(), "stable");

    let _ = fs::remove_dir_all(temp_dir);
}

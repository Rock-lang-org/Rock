use std::fs;

use crate::{
    constants::{LIB_DIR, STD_ARTIFACT_NAME, TOOLCHAINS_DIR},
    target::{add_target_component_from_dir, detect_target_component_source},
    toolchain::{install_toolchain, set_default_toolchain},
};

use super::support::{
    fake_home, temp_test_dir, write_fake_toolchain_root, write_project_toolchain,
    write_target_component_dir, write_target_component_source,
};

#[test]
fn test_detect_target_component_source_accepts_direct_dir() {
    let dir = temp_test_dir("target_component_direct");
    let component_dir = dir.join("wasm32-unknown-unknown");
    write_target_component_dir(&component_dir, "wasm");

    let source = detect_target_component_source(&component_dir, "wasm32-unknown-unknown").unwrap();
    assert_eq!(source, component_dir);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn test_detect_target_component_source_accepts_build_root() {
    let dir = temp_test_dir("target_component_build_root");
    write_target_component_source(&dir, "wasm32-unknown-unknown", "wasm");

    let source = detect_target_component_source(&dir, "wasm32-unknown-unknown").unwrap();
    assert_eq!(
        source,
        dir.join(LIB_DIR)
            .join("rocklib")
            .join("wasm32-unknown-unknown")
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn test_add_target_component_installs_into_selected_toolchain() {
    let temp_dir = temp_test_dir("add_target_component");
    let home = fake_home(temp_dir.join("home"));
    let toolchain_source = temp_dir.join("toolchain_source");
    let component_source = temp_dir.join("component_source");
    let triple = "wasm32-unknown-unknown";

    write_fake_toolchain_root(&toolchain_source);
    write_target_component_source(&component_source, triple, "wasm");
    install_toolchain(&home, "stable", &toolchain_source).unwrap();
    set_default_toolchain(&home, "stable").unwrap();

    let installed =
        add_target_component_from_dir(&home, triple, None, &component_source, temp_dir.as_path())
            .unwrap();
    assert_eq!(
        fs::read_to_string(installed.join(STD_ARTIFACT_NAME)).unwrap(),
        "wasm artifact\n"
    );

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_add_target_component_uses_project_pin() {
    let temp_dir = temp_test_dir("add_target_component_project_pin");
    let home = fake_home(temp_dir.join("home"));
    let stable_source = temp_dir.join("stable_source");
    let nightly_source = temp_dir.join("nightly_source");
    let component_source = temp_dir.join("component_source");
    let project_dir = temp_dir.join("project");
    let triple = "wasm32-unknown-unknown";

    write_fake_toolchain_root(&stable_source);
    write_fake_toolchain_root(&nightly_source);
    write_target_component_source(&component_source, triple, "wasm");
    install_toolchain(&home, "stable", &stable_source).unwrap();
    install_toolchain(&home, "nightly", &nightly_source).unwrap();
    set_default_toolchain(&home, "stable").unwrap();
    fs::create_dir_all(&project_dir).unwrap();
    write_project_toolchain(&project_dir, "nightly");

    let installed = add_target_component_from_dir(
        &home,
        triple,
        None,
        &component_source,
        project_dir.as_path(),
    )
    .unwrap();
    assert!(installed.starts_with(home.root.join(TOOLCHAINS_DIR).join("nightly")));
    assert!(!home
        .root
        .join(TOOLCHAINS_DIR)
        .join("stable")
        .join(LIB_DIR)
        .join("rocklib")
        .join(triple)
        .join(STD_ARTIFACT_NAME)
        .exists());

    let _ = fs::remove_dir_all(temp_dir);
}

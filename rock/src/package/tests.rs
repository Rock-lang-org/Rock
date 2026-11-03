use std::fs;

use crate::tests::support::{load_package, temp_test_dir, write_package};

use super::BuildProfile;

#[test]
fn test_shared_build_layout_isolates_same_named_packages_and_profiles() {
    let root = temp_test_dir("shared_build_layout");
    let app = root.join("app");
    let first = root.join("first");
    let second = root.join("second");
    for (path, name) in [(&app, "app"), (&first, "dep"), (&second, "dep")] {
        write_package(path, name, "lib.rk", &[], "< answer = -> 42\n");
    }
    let project = load_package(app.clone());
    let mut a = load_package(first.clone());
    let mut b = load_package(second.clone());
    a.use_project_build(&project.root_dir);
    b.use_project_build(&project.root_dir);
    assert_ne!(a.artifact_path(), b.artifact_path());
    assert_ne!(a.object_path(), b.object_path());
    let debug = a.artifact_path();
    a.profile = BuildProfile::Release;
    assert_ne!(a.artifact_path(), debug);
    assert!(a.build_dir().starts_with(app.join("build/release/deps")));
    assert!(!first.join("build").exists());
    assert!(!second.join("build").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn test_manifest_text_cannot_escape_the_shared_build_directory() {
    let root = temp_test_dir("safe_build_names");
    write_package(&root, "app", "lib.rk", &[], "< answer = -> 42\n");
    let mut package = load_package(root.clone());
    package.manifest.crate_.name = "../../escape".to_string();
    package.manifest.crate_.version = "../../../escape".to_string();
    assert_eq!(
        package.artifact_path().parent().unwrap(),
        package.build_dir().join("artifacts")
    );
    fs::remove_dir_all(root).unwrap();
}

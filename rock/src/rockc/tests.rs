use super::*;
use crate::package::BuildProfile;
use crate::tests::support::{load_package, temp_test_dir, write_package};

fn args_as_strings(invocation: &RockcInvocation) -> Vec<String> {
    invocation
        .args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn test_rockc_path_from_test_binary_uses_profile_dir() {
    let current = PathBuf::from("/workspace/target/debug/deps/rock-abc123");

    assert_eq!(
        rockc_path_from_current_exe(&current).unwrap(),
        PathBuf::from("/workspace/target/debug/rockc")
    );
}

#[test]
fn test_rockc_path_from_binary_uses_same_directory() {
    let current = PathBuf::from("/workspace/target/release/rock");

    assert_eq!(
        rockc_path_from_current_exe(&current).unwrap(),
        PathBuf::from("/workspace/target/release/rockc")
    );
}

#[test]
fn test_invocations_and_output_paths_use_the_selected_profile() {
    let temp_dir = temp_test_dir("rockc_build_profiles");
    write_package(&temp_dir, "app", "src/main.rk", &[], "main = !-> ()\n");
    let mut package = load_package(temp_dir.clone());
    let executable = PathBuf::from("rockc");
    for profile in [BuildProfile::Debug, BuildProfile::Release] {
        package.profile = profile;
        assert_eq!(
            package.build_dir(),
            package.root_dir.join("build").join(profile.directory())
        );
        assert!(package.artifact_path().starts_with(package.build_dir()));
        assert!(package.object_path().starts_with(package.build_dir()));
        for invocation in [
            build_root_invocation(executable.clone(), &package, &[]),
            build_dependency_artifact_invocation(executable.clone(), &package, &[]),
        ] {
            let args = args_as_strings(&invocation);
            assert!(args.contains(&profile.flag().to_string()));
            let other = if profile == BuildProfile::Debug {
                "--release"
            } else {
                "--debug"
            };
            assert!(!args
                .iter()
                .any(|arg| arg == other || arg == "-O" || arg == "--opt-level"));
        }
    }
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_dependency_invocation_uses_extern_artifact_for_all_deps() {
    let temp_dir = temp_test_dir("rockc_dep_invocation");
    write_package(
        &temp_dir,
        "dep_b",
        "lib.rk",
        &[],
        "relay = x -> x\n< relay\n",
    );
    let package = load_package(temp_dir.clone());
    let invocation = build_dependency_artifact_invocation(
        PathBuf::from("/workspace/target/debug/rockc"),
        &package,
        &[
            ExternArtifact {
                name: "dep_a".to_string(),
                path: PathBuf::from("/tmp/dep_a.rkca"),
            },
            ExternArtifact {
                name: STD_CRATE_NAME.to_string(),
                path: PathBuf::from("/tmp/std.rkca"),
            },
        ],
    );
    let args = args_as_strings(&invocation);

    assert_eq!(
        invocation.executable,
        PathBuf::from("/workspace/target/debug/rockc")
    );
    assert!(args
        .windows(2)
        .any(|pair| pair[0] == "--crate-name" && pair[1] == "dep_b"));
    assert!(args.windows(2).any(|pair| {
        pair[0] == "--entry-file" && pair[1] == package.entry_file().to_string_lossy().as_ref()
    }));
    assert!(args.contains(&"--no-link".to_string()));
    assert!(args.windows(2).any(|pair| {
        pair[0] == "--emit-artifact"
            && pair[1] == package.artifact_path().to_string_lossy().as_ref()
    }));
    assert!(args.windows(2).any(|pair| {
        pair[0] == "--emit-object" && pair[1] == package.object_path().to_string_lossy().as_ref()
    }));
    assert!(!args.contains(&"--extern-product-artifact".to_string()));
    assert!(args
        .windows(2)
        .any(|pair| pair[0] == "--extern-artifact" && pair[1] == "dep_a=/tmp/dep_a.rkca"));
    assert!(args
        .windows(2)
        .any(|pair| { pair[0] == "--extern-artifact" && pair[1] == "std=/tmp/std.rkca" }));

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_root_invocation_uses_extern_artifact_for_all_deps() {
    let temp_dir = temp_test_dir("rockc_root_invocation");
    write_package(&temp_dir, "app", "src/main.rk", &[], "main = -> 0\n");
    let package = load_package(temp_dir.clone());
    let invocation = build_root_invocation(
        PathBuf::from("/workspace/target/debug/rockc"),
        &package,
        &[
            ExternArtifact {
                name: "dep".to_string(),
                path: PathBuf::from("/tmp/dep.rkca"),
            },
            ExternArtifact {
                name: STD_CRATE_NAME.to_string(),
                path: PathBuf::from("/tmp/std.rkca"),
            },
        ],
    );
    let args = args_as_strings(&invocation);

    assert!(args.windows(2).any(|pair| {
        pair[0] == "--entry-file" && pair[1] == package.entry_file().to_string_lossy().as_ref()
    }));
    assert!(args.windows(2).any(|pair| {
        pair[0] == "--output-dir"
            && pair[1] == package.binary_build_dir().to_string_lossy().as_ref()
    }));
    assert!(!args.contains(&"--extern-product-artifact".to_string()));
    assert!(args
        .windows(2)
        .any(|pair| pair[0] == "--extern-artifact" && pair[1] == "dep=/tmp/dep.rkca"));
    assert!(args
        .windows(2)
        .any(|pair| { pair[0] == "--extern-artifact" && pair[1] == "std=/tmp/std.rkca" }));
    assert!(!args.contains(&"--emit-artifact".to_string()));
    assert!(!args.contains(&"--no-link".to_string()));

    let _ = std::fs::remove_dir_all(temp_dir);
}

use super::{
    host_target_triple, resolve_sysroot_from, sysroot_from_executable, SysrootLayout,
    SysrootResolution, SysrootSource,
};
use std::{fs, path::PathBuf};

#[test]
fn test_sysroot_resolution_explicit_sources() {
    assert!(SysrootResolution {
        path: PathBuf::from("/cli"),
        source: SysrootSource::Cli,
    }
    .is_explicit());
    assert!(!SysrootResolution {
        path: PathBuf::from("/toolchains/stable"),
        source: SysrootSource::ExecutableRelative,
    }
    .is_explicit());
}

#[test]
fn test_resolve_sysroot_prefers_cli_over_env() {
    let resolution = resolve_sysroot_from(
        Some(PathBuf::from("/cli").as_path()),
        Some(PathBuf::from("/env")),
        Some(PathBuf::from("/target-dir")),
        Some(PathBuf::from("/current-dir-target")),
        PathBuf::from("/toolchains/stable/bin/rockc").as_path(),
    )
    .unwrap();

    assert_eq!(resolution.path, PathBuf::from("/cli"));
    assert_eq!(resolution.source, SysrootSource::Cli);
}

#[test]
fn test_resolve_sysroot_uses_executable_relative_fallback() {
    let resolution = resolve_sysroot_from(
        None,
        None,
        None,
        None,
        PathBuf::from("/toolchains/stable/bin/rockc").as_path(),
    )
    .unwrap();

    assert_eq!(resolution.path, PathBuf::from("/toolchains/stable"));
    assert_eq!(resolution.source, SysrootSource::ExecutableRelative);
}

#[test]
fn test_installed_toolchain_ignores_workspace_and_cargo_target_dirs() {
    for binary in ["rock", "rockc", "rock-lsp"] {
        for cargo_target in [None, Some(PathBuf::from("/cargo-target"))] {
            let resolution = resolve_sysroot_from(
                None,
                None,
                cargo_target,
                Some(PathBuf::from("/workspace/target")),
                &PathBuf::from("/toolchains/stable/bin").join(binary),
            )
            .unwrap();

            assert_eq!(resolution.path, PathBuf::from("/toolchains/stable"));
            assert_eq!(resolution.source, SysrootSource::ExecutableRelative);
        }
    }
}

#[test]
fn test_explicit_env_sysroot_overrides_installed_toolchain() {
    let resolution = resolve_sysroot_from(
        None,
        Some(PathBuf::from("/explicit")),
        Some(PathBuf::from("/cargo-target")),
        Some(PathBuf::from("/workspace/target")),
        &PathBuf::from("/toolchains/stable/bin/rock"),
    )
    .unwrap();

    assert_eq!(resolution.path, PathBuf::from("/explicit"));
    assert_eq!(resolution.source, SysrootSource::Env);
}

#[test]
fn test_dev_binary_preserves_cargo_target_override() {
    let resolution = resolve_sysroot_from(
        None,
        None,
        Some(PathBuf::from("/cargo-target")),
        Some(PathBuf::from("/workspace/target")),
        &PathBuf::from("/workspace/target/debug/rock"),
    )
    .unwrap();

    assert_eq!(resolution.path, PathBuf::from("/cargo-target"));
    assert_eq!(resolution.source, SysrootSource::CargoTargetDir);
}

#[test]
fn test_sysroot_from_executable_uses_parent_of_bin_dir() {
    let sysroot =
        sysroot_from_executable(PathBuf::from("/toolchains/stable/bin/rockc").as_path()).unwrap();

    assert_eq!(sysroot, PathBuf::from("/toolchains/stable"));
}

#[test]
fn test_sysroot_layout_matches_contract() {
    let target = host_target_triple();
    let layout = SysrootLayout::new(PathBuf::from("/toolchains/stable"), target.clone());

    assert_eq!(layout.target_triple, target);
    assert_eq!(
        layout.target_libdir,
        PathBuf::from("/toolchains/stable/lib/rocklib").join(&layout.target_triple)
    );
    assert_eq!(layout.std_artifact, layout.target_libdir.join("std.rkca"));
    assert_eq!(layout.std_object, layout.target_libdir.join("std.o"));
    assert_eq!(
        layout.manifest_path,
        layout.target_libdir.join("manifest.json")
    );
    assert_eq!(
        layout.components_path,
        layout.target_libdir.join("components.json")
    );
}

#[test]
fn test_sysroot_from_current_dir_finds_workspace_target() {
    let unique = format!("rock-sysroot-test-{}", std::process::id());
    let base = std::env::temp_dir().join(unique);
    let nested = base.join("projects").join("demo");
    let target = base.join("target");
    let layout = SysrootLayout::new(target.clone(), host_target_triple());

    fs::create_dir_all(&nested).unwrap();
    fs::create_dir_all(&layout.target_libdir).unwrap();
    fs::write(&layout.std_artifact, b"artifact").unwrap();

    let resolution = resolve_sysroot_from(
        None,
        None,
        None,
        Some(target.clone()),
        base.join("target/debug/rockc").as_path(),
    )
    .unwrap();
    assert_eq!(resolution.path, target);
    assert_eq!(resolution.source, SysrootSource::CurrentDirTarget);

    let _ = fs::remove_dir_all(&base);
}

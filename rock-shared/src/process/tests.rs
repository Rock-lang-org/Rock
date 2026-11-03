use super::{dev_target_binary_from_exe, dev_target_binary_from_exes};
use std::{fs, path::PathBuf};

#[test]
fn test_dev_target_binary_from_test_binary_uses_profile_dir() {
    let current = PathBuf::from("/workspace/target/debug/deps/rock-abc123");

    assert_eq!(
        dev_target_binary_from_exe(&current, "rockc").unwrap(),
        PathBuf::from("/workspace/target/debug/rockc")
    );
}

#[test]
fn test_dev_target_binary_from_binary_uses_same_directory() {
    let current = PathBuf::from("/workspace/target/release/rock");

    assert_eq!(
        dev_target_binary_from_exe(&current, "rockc").unwrap(),
        PathBuf::from("/workspace/target/release/rockc")
    );
}

#[test]
fn test_dev_target_binary_from_artifact_dir_uses_profile_dir() {
    for profile in ["debug", "release"] {
        let profile_dir = PathBuf::from("/workspace/target").join(profile);
        let current = profile_dir.join("build/rock/abc123/out/rock-abc123");
        assert_eq!(
            dev_target_binary_from_exe(&current, "rockc").unwrap(),
            profile_dir.join(format!("rockc{}", std::env::consts::EXE_SUFFIX))
        );
    }
}

#[test]
fn test_dev_target_binary_from_unrelated_out_directory_stays_adjacent() {
    let current = PathBuf::from("/workspace/custom/rock/abc123/out/rock");
    assert_eq!(
        dev_target_binary_from_exe(&current, "rockc").unwrap(),
        current
            .parent()
            .unwrap()
            .join(format!("rockc{}", std::env::consts::EXE_SUFFIX))
    );
}

#[test]
fn test_dev_target_binary_uses_invocation_path_when_current_exe_is_sandboxed() {
    let base = std::env::temp_dir().join(format!("rock-process-test-{}", std::process::id()));
    let profile_dir = base.join("target/debug");
    let rockc = profile_dir.join("rockc");
    fs::create_dir_all(&profile_dir).unwrap();
    fs::write(&rockc, b"").unwrap();

    let resolved = dev_target_binary_from_exes(
        &base.join(".sandbox/rock"),
        Some(&profile_dir.join("rock")),
        "rockc",
    )
    .unwrap();

    assert_eq!(resolved, rockc);
    let _ = fs::remove_dir_all(base);
}

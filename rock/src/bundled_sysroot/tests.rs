use super::*;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use rock_shared::sysroot::{SysrootLayout, PRODUCT_ARTIFACT_FORMAT_VERSION};

use super::sysroot_std_is_fresh;

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn write_complete_sysroot_std(artifact_format_version: &str) -> (PathBuf, SysrootLayout) {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = canonical_temp_dir().join(format!(
        "rock_cli_sysroot_format_{}_{}",
        std::process::id(),
        id
    ));
    let layout = SysrootLayout::new(root.clone(), "test-target".to_string());
    fs::create_dir_all(&layout.target_libdir).unwrap();
    fs::write(&layout.std_artifact, "artifact").unwrap();
    fs::write(&layout.std_object, "object").unwrap();
    fs::write(
        &layout.manifest_path,
        format!(
            concat!(
                "{{\n",
                "  \"std\": {{\n",
                "    \"artifact_format_version\": {}\n",
                "  }}\n",
                "}}\n"
            ),
            artifact_format_version
        ),
    )
    .unwrap();
    fs::write(&layout.components_path, "{}").unwrap();

    (root, layout)
}

#[test]
fn test_sysroot_std_is_not_fresh_with_stale_artifact_format() {
    let (root, layout) =
        write_complete_sysroot_std(&(PRODUCT_ARTIFACT_FORMAT_VERSION - 1).to_string());

    assert!(!sysroot_std_is_fresh(&layout, None).unwrap());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn test_sysroot_std_is_not_fresh_with_malformed_artifact_format() {
    for artifact_format_version in [
        format!("{}.1", PRODUCT_ARTIFACT_FORMAT_VERSION),
        format!("{}garbage", PRODUCT_ARTIFACT_FORMAT_VERSION),
    ] {
        let (root, layout) = write_complete_sysroot_std(&artifact_format_version);

        assert!(!sysroot_std_is_fresh(&layout, None).unwrap());

        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn test_sysroot_std_is_fresh_with_current_artifact_format() {
    let (root, layout) = write_complete_sysroot_std(&PRODUCT_ARTIFACT_FORMAT_VERSION.to_string());

    assert!(sysroot_std_is_fresh(&layout, None).unwrap());

    let _ = fs::remove_dir_all(root);
}

pub(crate) fn current_sysroot_layout() -> Result<SysrootLayout, String> {
    current_sysroot_resolution().map(|resolution| resolution.into_layout())
}

#[test]
fn test_dev_sysroot_std_without_release_profile_is_rebuilt() {
    let (root, layout) = write_complete_sysroot_std(&PRODUCT_ARTIFACT_FORMAT_VERSION.to_string());
    let std_root = root.join("std-src");
    fs::create_dir_all(&std_root).unwrap();
    fs::write(std_root.join("rock.toml"), "[crate]\nname = \"std\"\n").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    // The packaged std must be newer than its sources to be fresh.
    fs::write(&layout.std_artifact, "artifact").unwrap();
    fs::write(&layout.std_object, "object").unwrap();

    // An installed toolchain cannot rebuild std, so it is used as shipped.
    assert!(sysroot_std_is_fresh(&layout, None).unwrap());
    assert!(!sysroot_std_is_fresh(&layout, Some(&std_root)).unwrap());

    let manifest = fs::read_to_string(&layout.manifest_path).unwrap().replacen(
        "\"std\": {\n",
        &format!(
            "\"std\": {{\n    \"profile\": \"{}\",\n",
            rock_shared::sysroot::STD_PROFILE
        ),
        1,
    );
    fs::write(&layout.manifest_path, manifest).unwrap();
    assert!(sysroot_std_is_fresh(&layout, Some(&std_root)).unwrap());

    let _ = fs::remove_dir_all(root);
}

// The tools report resolved paths: Darwin's temp dir is a symlink into
// /private, and Windows' may use 8.3 short names.
fn canonical_temp_dir() -> std::path::PathBuf {
    dunce::canonicalize(std::env::temp_dir()).unwrap()
}

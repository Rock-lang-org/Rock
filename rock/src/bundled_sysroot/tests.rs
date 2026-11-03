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
    let root = std::env::temp_dir().join(format!(
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

use super::collect_package_source_inputs;
use std::{fs, path::PathBuf};

#[test]
fn test_collect_package_source_inputs_includes_manifest_and_rock_sources() {
    let root = std::env::temp_dir().join(format!("rock-shared-fs-test-{}", std::process::id()));
    let src = root.join("src");
    let build = root.join("build");
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&build).unwrap();
    fs::write(root.join("rock.toml"), b"[crate]").unwrap();
    fs::write(src.join("lib.rk"), b"").unwrap();
    fs::write(build.join("ignored.rk"), b"").unwrap();

    let inputs = collect_package_source_inputs(&root, "build").unwrap();

    assert_eq!(
        inputs,
        vec![root.join("rock.toml"), PathBuf::from(src.join("lib.rk"))]
    );

    let _ = fs::remove_dir_all(root);
}

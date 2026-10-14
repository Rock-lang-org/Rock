use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::{
    constants::{
        COMPONENTS_MANIFEST_NAME, LIB_DIR, STD_ARTIFACT_NAME, STD_OBJECT_NAME,
        TOOLCHAIN_MANIFEST_NAME,
    },
    dev::package_dev_std,
    layout::host_target_triple,
};

use super::support::{temp_test_dir, workspace_std_root, write_script};

#[test]
fn test_package_dev_std_writes_sysroot_layout() {
    let temp_dir = temp_test_dir("package_dev_std");
    let sysroot = temp_dir.join("dev-sysroot");
    let target = host_target_triple();
    let rockc = ensure_rockc_binary();

    // Layout/source-copy behavior needs a real artifact, not the full std.
    // The relocation test below still packages the complete shipped library.
    let std = temp_dir.join("std");
    fs::create_dir_all(std.join("src")).unwrap();
    fs::write(
        std.join("rock.toml"),
        "[crate]\nname = \"std\"\nversion = \"0.1.0\"\n\n[lib]\npath = \"src/lib.rk\"\n",
    )
    .unwrap();
    fs::write(std.join("src/lib.rk"), "< mod values\n").unwrap();
    fs::create_dir_all(std.join("src/values")).unwrap();
    fs::write(std.join("src/values/mod.rk"), "< answer = -> 42\n").unwrap();

    let packaged =
        package_dev_std(&std, &sysroot, Some(target.clone()), true, Some(&rockc)).unwrap();

    assert_eq!(
        packaged,
        sysroot
            .canonicalize()
            .unwrap()
            .join(LIB_DIR)
            .join("rocklib")
            .join(&target)
    );
    assert!(packaged.join(STD_ARTIFACT_NAME).exists());
    assert!(packaged.join(STD_OBJECT_NAME).exists());
    assert!(packaged.join(TOOLCHAIN_MANIFEST_NAME).exists());
    assert!(packaged.join(COMPONENTS_MANIFEST_NAME).exists());
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(packaged.join(TOOLCHAIN_MANIFEST_NAME)).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["std"]["artifact"], "std.rkca");
    assert_eq!(manifest["std"]["object"], "std.o");
    let components: serde_json::Value = serde_json::from_slice(
        &fs::read(packaged.join(COMPONENTS_MANIFEST_NAME)).unwrap(),
    )
    .unwrap();
    assert_eq!(components["components"]["std"]["present"], true);
    assert_eq!(components["components"]["std"]["artifact"], "std.rkca");
    assert_eq!(components["components"]["std"]["object"], "std.o");
    assert!(sysroot
        .join("src")
        .join("std")
        .join("rock.toml")
        .exists());
    assert_eq!(
        fs::read(sysroot.join("src/std/src/lib.rk")).unwrap(),
        fs::read(std.join("src/lib.rk")).unwrap()
    );
    assert_eq!(
        fs::read(sysroot.join("src/std/src/values/mod.rk")).unwrap(),
        fs::read(std.join("src/values/mod.rk")).unwrap()
    );

    let _ = fs::remove_dir_all(temp_dir);
}

fn ensure_rockc_binary() -> PathBuf {
    let rockc = rock_shared::process::dev_target_binary_from_current_exe("rockc").unwrap();
    if !rockc.exists() {
        let status = Command::new("cargo")
            .args(["build", "-p", "rockc"])
            .current_dir(workspace_root())
            .status()
            .unwrap();
        assert!(status.success());
    }
    assert!(
        rockc.exists(),
        "missing rockc binary at {}",
        rockc.display()
    );
    rockc
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn test_package_dev_std_passes_absolute_product_paths_for_relative_sysroot() {
    let temp_dir = temp_test_dir("package_dev_std_relative_sysroot");
    let capture_file = temp_dir.join("rockc-args.txt");
    let fake_rockc = temp_dir.join("rockc");
    write_script(
        &fake_rockc,
        &format!(
            r#"capture={}
object=
artifact=
previous=
for arg in "$@"; do
  if [ "$previous" = "--emit-object" ]; then
    object="$arg"
  fi
  if [ "$previous" = "--emit-artifact" ]; then
    artifact="$arg"
  fi
  previous="$arg"
done
printf '%s\n%s\n' "$object" "$artifact" > "$capture"
mkdir -p "$(dirname "$object")"
mkdir -p "$(dirname "$artifact")"
touch "$object"
touch "$artifact"
exit 0
"#,
            capture_file.display()
        ),
    );
    let relative_sysroot = PathBuf::from(format!(
        "target/rockup_relative_sysroot_{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&relative_sysroot);

    package_dev_std(
        &workspace_std_root(),
        &relative_sysroot,
        Some(host_target_triple()),
        false,
        Some(&fake_rockc),
    )
    .unwrap();

    let captured = fs::read_to_string(&capture_file).unwrap();
    for path in captured.lines() {
        assert!(
            PathBuf::from(path).is_absolute(),
            "expected absolute product path, got {}",
            path
        );
    }

    let _ = fs::remove_dir_all(relative_sysroot);
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_std_package_invocation_uses_rockc_product_outputs() {
    let temp_dir = temp_test_dir("std_package_invocation");
    let sysroot = temp_dir.join("dev-sysroot");
    let std = workspace_std_root();
    let target = host_target_triple();
    let layout = crate::layout::ToolchainLayout::new(sysroot.clone());
    assert_eq!(layout.target_triple, target);

    let invocation =
        crate::dev::build_std_package_invocation(PathBuf::from("/tmp/rockc"), &std, &layout)
            .unwrap();
    let args = invocation.args_as_strings();

    assert_eq!(invocation.executable, PathBuf::from("/tmp/rockc"));
    assert!(args.windows(2).any(|pair| pair[0] == "--entry-file"
        && pair[1] == std.join("src/lib.rk").to_string_lossy().as_ref()));
    assert!(args
        .windows(2)
        .any(|pair| pair[0] == "--crate-name" && pair[1] == "std"));
    assert!(args.contains(&"--no-std".to_string()));
    assert!(args.contains(&"--no-prelude".to_string()));
    assert!(args.contains(&"--no-link".to_string()));
    assert!(args.windows(2).any(|pair| pair[0] == "--emit-object"
        && pair[1] == layout.std_object.to_string_lossy().as_ref()));
    assert!(args.windows(2).any(|pair| pair[0] == "--emit-artifact"
        && pair[1] == layout.std_artifact.to_string_lossy().as_ref()));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_packaged_std_artifact_remains_valid_after_target_component_relocation() {
    let temp_dir = temp_test_dir("package_dev_std_relocatable");
    let source_sysroot = temp_dir.join("source-sysroot");
    let copied_component = temp_dir.join("copied-component");
    let target = host_target_triple();
    let rockc = ensure_rockc_binary();

    let packaged = package_dev_std(
        &workspace_std_root(),
        &source_sysroot,
        Some(target),
        false,
        Some(&rockc),
    )
    .unwrap();

    crate::target::install_target_component_dir(&packaged, &copied_component).unwrap();
    fs::remove_dir_all(&packaged).unwrap();

    let status = Command::new(&rockc)
        .arg("--validate-artifact")
        .arg(copied_component.join(STD_ARTIFACT_NAME))
        .status()
        .unwrap();

    assert!(status.success());

    let _ = fs::remove_dir_all(temp_dir);
}

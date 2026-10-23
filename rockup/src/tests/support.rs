use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{
    constants::{
        BIN_DIR, COMPONENTS_MANIFEST_NAME, DEFAULT_TOOLCHAIN_FILE, LIB_DIR, ROCKC_BIN_NAME,
        ROCK_BIN_NAME, ROCK_LSP_BIN_NAME, ROCK_TOOLCHAIN_FILE, STD_ARTIFACT_NAME,
        STD_OBJECT_NAME, TOOLCHAIN_MANIFEST_NAME,
    },
    home::RockupHome,
    layout::host_target_triple,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(super) fn temp_test_dir(name: &str) -> PathBuf {
    loop {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("rockup_{}_{}_{}", name, std::process::id(), id));
        match fs::create_dir(&dir) {
            Ok(()) => return dir,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => panic!("failed to create test directory {}: {error}", dir.display()),
        }
    }
}

#[test]
fn temporary_directories_are_process_isolated() {
    const PROBE: &str = "ROCKUP_TEST_DIRECTORY_PROBE";
    const PREFIX: &str = "ROCKUP_TEST_DIRECTORY=";
    if std::env::var_os(PROBE).is_some() {
        let directory = temp_test_dir("process_isolation");
        fs::write(directory.join("owner"), std::process::id().to_string()).unwrap();
        println!("\n{PREFIX}{}", directory.display());
        return;
    }

    let directories = (0..2).map(|_| {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["tests::support::temporary_directories_are_process_isolated", "--exact", "--nocapture"])
            .env(PROBE, "1")
            .output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let stdout = String::from_utf8(output.stdout).unwrap();
        let directory = PathBuf::from(stdout.lines().find_map(|line| line.strip_prefix(PREFIX)).unwrap());
        let owner = fs::read_to_string(directory.join("owner")).unwrap();
        (directory, owner)
    }).collect::<Vec<_>>();
    assert_ne!(directories[0].0, directories[1].0);
    for (directory, owner) in directories {
        assert_eq!(fs::read_to_string(directory.join("owner")).unwrap(), owner);
        fs::remove_dir_all(directory).unwrap();
    }
}

pub(super) fn fake_home(root: PathBuf) -> RockupHome {
    RockupHome { root }
}

pub(super) fn write_fake_toolchain_root(root: &Path) {
    let bin_dir = root.join(BIN_DIR);
    let target_libdir = root
        .join(LIB_DIR)
        .join("rocklib")
        .join(host_target_triple());

    fs::create_dir_all(&bin_dir).unwrap();
    fs::create_dir_all(&target_libdir).unwrap();

    write_script(&bin_dir.join(ROCK_BIN_NAME), "exit 17\n");
    write_script(&bin_dir.join(ROCKC_BIN_NAME), "exit 0\n");
    write_script(&bin_dir.join(ROCK_LSP_BIN_NAME), "exit 31\n");
    fs::write(target_libdir.join(STD_ARTIFACT_NAME), "artifact").unwrap();
    fs::write(target_libdir.join(STD_OBJECT_NAME), "object").unwrap();
    fs::write(target_libdir.join(TOOLCHAIN_MANIFEST_NAME), "{}\n").unwrap();
    fs::write(target_libdir.join(COMPONENTS_MANIFEST_NAME), "{}\n").unwrap();
}

pub(super) fn write_fake_cargo_build_dir(root: &Path) {
    let target_libdir = root
        .join(LIB_DIR)
        .join("rocklib")
        .join(host_target_triple());

    fs::create_dir_all(&target_libdir).unwrap();
    write_script(&root.join(ROCK_BIN_NAME), "exit 13\n");
    write_script(&root.join(ROCKC_BIN_NAME), "exit 0\n");
    write_script(&root.join(ROCK_LSP_BIN_NAME), "exit 31\n");
    fs::write(target_libdir.join(STD_ARTIFACT_NAME), "artifact").unwrap();
    fs::write(target_libdir.join(STD_OBJECT_NAME), "object").unwrap();
    fs::write(target_libdir.join(TOOLCHAIN_MANIFEST_NAME), "{}\n").unwrap();
    fs::write(target_libdir.join(COMPONENTS_MANIFEST_NAME), "{}\n").unwrap();
}

pub(super) fn write_target_component_dir(root: &Path, label: &str) {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join(STD_ARTIFACT_NAME),
        format!("{} artifact\n", label),
    )
    .unwrap();
    fs::write(root.join(STD_OBJECT_NAME), format!("{} object\n", label)).unwrap();
    fs::write(
        root.join(TOOLCHAIN_MANIFEST_NAME),
        format!("{} manifest\n", label),
    )
    .unwrap();
    fs::write(
        root.join(COMPONENTS_MANIFEST_NAME),
        format!("{} components\n", label),
    )
    .unwrap();
}

pub(super) fn write_target_component_source(root: &Path, triple: &str, label: &str) {
    write_target_component_dir(&root.join(LIB_DIR).join("rocklib").join(triple), label);
}

pub(super) fn write_script(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }

    fs::write(path, format!("#!/bin/sh\n{}", body)).unwrap();
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }
}

pub(super) fn write_project_toolchain(path: &Path, toolchain: &str) {
    fs::write(
        path.join(ROCK_TOOLCHAIN_FILE),
        format!("[toolchain]\nchannel = \"{}\"\n", toolchain),
    )
    .unwrap();
}

pub(super) fn read_default_toolchain(home: &RockupHome) -> String {
    fs::read_to_string(home.root.join(DEFAULT_TOOLCHAIN_FILE))
        .unwrap()
        .trim()
        .to_string()
}

pub(super) fn workspace_std_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("std")
}

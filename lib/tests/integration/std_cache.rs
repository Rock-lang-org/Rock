use crate::support::*;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;
use crate::std_cache_key::std_cache_key;

fn write_minimal_std_artifact_for_cache_test(path: &Path) {
    rock_lib::products::CompilerProducts {
        crate_identity: rock_lib::products::ProductCrateIdentity::local("std".to_string()),
        identity_table: rock_lib::products::ProductIdentityTable::default(),
        interface: rock_lib::products::ProductInterface::default(),
        bodies: rock_lib::products::ProductBodies::default(),
        link: rock_lib::products::ProductLinkData {
            object_path: Some(PathBuf::from("std.o")),
            ..rock_lib::products::ProductLinkData::default()
        },
        dependencies: Vec::new(),
        source_fingerprint: rock_lib::products::ProductSourceFingerprint::default(),
        infix_precedence: std::collections::BTreeMap::new(),
        proc_macros: Vec::new(),
    }
    .write_artifact_to_path(path)
    .unwrap();
}

#[test]
fn std_cache_key_changes_when_std_source_changes() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    let std_dir = dir.join("std");
    std::fs::create_dir_all(std_dir.join("src/values")).unwrap();
    std::fs::write(std_dir.join("src/lib.rk"), "mod values\n").unwrap();
    let values = std_dir.join("src/values/mod.rk");
    std::fs::write(&values, "answer = -> 0\n").unwrap();

    let first = std_cache_key(&std_dir, "compiler-stamp");
    let second = std_cache_key(&std_dir, "compiler-stamp");
    assert_eq!(first, second);

    std::fs::write(&values, "answer = -> 1\n").unwrap();
    let changed = std_cache_key(&std_dir, "compiler-stamp");

    assert_ne!(first, changed);
    assert_ne!(changed, std_cache_key(&std_dir, "rebuilt-compiler"));

    std::fs::write(std_dir.join("rock.toml"), "[crate]\nname = \"std\"\n").unwrap();
    let with_manifest = std_cache_key(&std_dir, "compiler-stamp");
    assert_ne!(changed, with_manifest);
    std::fs::write(
        std_dir.join("rock.toml"),
        "[crate]\nname = \"renamed\"\n",
    )
    .unwrap();
    assert_ne!(
        with_manifest,
        std_cache_key(&std_dir, "compiler-stamp")
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn std_cache_rejects_missing_ready_marker() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();
    write_minimal_std_artifact_for_cache_test(&dir.join("std.rkca"));
    std::fs::write(dir.join("std.o"), b"object").unwrap();

    assert!(!std_cache_entry_is_ready(
        &dir.join("std.rkca"),
        &dir.join("std.o"),
        &dir.join("ready")
    ));
}

#[test]
fn std_cache_stale_lock_recovery_does_not_spin_forever() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();
    let artifact_dir = dir.join("cache");
    let lock_path = artifact_dir.with_extension("lock");
    std::fs::write(&lock_path, "pid=0\ncreated_unix_nanos=0\n").unwrap();

    assert!(recover_stale_cache_lock(&lock_path, Duration::from_secs(1)));
    assert!(!lock_path.exists());
}

#[test]
fn std_cache_publish_replaces_stale_ready_entry_atomically() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    let artifact_dir = dir.join("cache");
    let build_dir = dir.join("build");
    let ready_path = artifact_dir.join("ready");
    std::fs::create_dir_all(&artifact_dir).unwrap();
    std::fs::create_dir_all(&build_dir).unwrap();
    std::fs::write(artifact_dir.join("ready"), b"ready").unwrap();
    std::fs::write(artifact_dir.join("std.rkca"), b"invalid stale artifact").unwrap();
    std::fs::write(artifact_dir.join("std.o"), b"stale object").unwrap();
    write_minimal_std_artifact_for_cache_test(&build_dir.join("std.rkca"));
    std::fs::write(build_dir.join("std.o"), b"fresh object").unwrap();

    publish_std_cache_dir(&build_dir, &artifact_dir, &ready_path)
        .expect("cache publish should replace stale ready entry");

    assert!(std_cache_entry_is_ready(
        &artifact_dir.join("std.rkca"),
        &artifact_dir.join("std.o"),
        &ready_path,
    ));
    assert_eq!(
        std::fs::read(artifact_dir.join("std.o")).unwrap(),
        b"fresh object"
    );
}

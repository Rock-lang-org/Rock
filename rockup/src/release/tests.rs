use super::*;

const TARGET: &str = "x86_64-unknown-linux-gnu";

fn github_release(tag: &str) -> serde_json::Value {
    let archive = format!("rock-{}-{}.tar.gz", tag, TARGET);
    serde_json::json!({
        "tag_name": tag,
        "draft": false,
        "prerelease": false,
        "assets": [{"name": archive}, {"name": format!("{}.sha256", archive)}],
    })
}

#[test]
fn available_versions_require_installable_tags_and_both_assets() {
    let stable = github_release("v1.2.3");
    let mut prerelease = github_release("v1.3.0-rc.1");
    prerelease["prerelease"] = true.into();
    let mut draft = github_release("v1.4.0");
    draft["draft"] = true.into();
    let mut missing_checksum = github_release("v1.2.2");
    missing_checksum["assets"].as_array_mut().unwrap().pop();
    let mut wrong_platform = github_release("v1.2.1");
    wrong_platform["assets"] = serde_json::json!([
        {"name": "rock-v1.2.1-aarch64-apple-darwin.tar.gz"},
        {"name": "rock-v1.2.1-aarch64-apple-darwin.tar.gz.sha256"},
    ]);
    let body = serde_json::to_vec(&vec![
        stable.clone(),
        prerelease,
        draft,
        missing_checksum,
        wrong_platform,
        github_release("nightly"),
        github_release("stable"),
        github_release("1.2.3"),
        github_release("v1.2.3\n"),
        stable,
    ])
    .unwrap();
    let available = collect_available_releases(TARGET, |page| {
        assert_eq!(page, 1);
        Ok(body.clone())
    })
    .unwrap();
    assert_eq!(available.len(), 2);
    assert_eq!(available[0].tag, "v1.2.3");
    assert!(!available[0].prerelease);
    assert_eq!(available[1].tag, "v1.3.0-rc.1");
    assert!(available[1].prerelease);
}

#[test]
fn available_versions_paginate_before_filtering() {
    let mut draft = github_release("v2.0.0");
    draft["draft"] = true.into();
    let mut pages = Vec::new();
    let available = collect_available_releases(TARGET, |page| {
        pages.push(page);
        Ok(serde_json::to_vec(&match page {
            1 => vec![draft.clone(); 100],
            2 => vec![github_release("v1.0.0")],
            _ => panic!("unexpected page {}", page),
        })
        .unwrap())
    })
    .unwrap();
    assert_eq!(pages, [1, 2]);
    assert_eq!(available.len(), 1);
    assert_eq!(available[0].tag, "v1.0.0");
}

#[test]
fn available_versions_handle_empty_invalid_and_failed_responses() {
    assert!(collect_available_releases(TARGET, |_| Ok(b"[]".to_vec()))
        .unwrap()
        .is_empty());
    for body in ["not json", "{}", "[{\"tag_name\":\"v1.0.0\"}]"] {
        assert!(collect_available_releases(TARGET, |_| Ok(body.as_bytes().to_vec())).is_err());
    }
    assert!(matches!(
        collect_available_releases(TARGET, |_| Err("offline".into())),
        Err(error) if error == "offline"
    ));
}

#[test]
fn download_meter_shows_percentage_and_both_speeds() {
    let mut output = Vec::new();
    show_download_progress(
        b" 50 2048k 50 1024k 0 0 256k 0 0:00:08 0:00:04 0:00:04 300k",
        &mut output,
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "\r\x1b[2K[==========          ]  50% 1024k/2048k | avg 256k/s | current 300k/s"
    );
}

#[test]
fn download_meter_suppresses_zero_rows_with_blank_times() {
    let mut output = Vec::new();
    for _ in 0..4 {
        show_download_progress(
            b"  0      0   0      0   0      0      0      0                              0",
            &mut output,
        )
        .unwrap();
    }
    assert!(output.is_empty());
    show_download_progress(
        b"73 38.73M 73 28.52M 0 0 203.9k 0 0:03:14 0:02:23 0:00:51 215.0k",
        &mut output,
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "\r\x1b[2K[==============      ]  73% 28.52M/38.73M | avg 203.9k/s | current 215.0k/s"
    );
}

#[test]
fn download_meter_handles_unknown_size_and_preserves_errors() {
    let mut output = Vec::new();
    for line in [
        "% Total    % Received % Xferd  Average Speed   Time    Time     Time  Current",
        "                                 Dload  Upload   Total   Spent    Left  Speed",
        "",
    ] {
        show_download_progress(line.as_bytes(), &mut output).unwrap();
    }
    assert!(output.is_empty());
    show_download_progress(
        b"0 0 0 1024 0 0 512 0 --:--:-- 0:00:02 --:--:-- 512",
        &mut output,
    )
    .unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("1024 received"));
    assert!(!text.contains('%'));
    let mut output = Vec::new();
    show_download_progress(b"curl: (22) HTTP error 404", &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "\r\x1b[2Kcurl: (22) HTTP error 404\n"
    );
}

#[test]
fn release_names_and_latest_redirect() {
    for name in ["1.2.3", "v1.2.3"] {
        assert_eq!(release_name(name).unwrap(), "v1.2.3");
    }
    assert_eq!(release_name("stable").unwrap(), "stable");
    assert_eq!(release_name("1").unwrap(), "v1");
    assert_eq!(release_name("v1.2").unwrap(), "v1.2");
    assert_eq!(release_name("1.2.3+build.4").unwrap(), "v1.2.3+build.4");
    for name in [
        "",
        "..",
        "../stable",
        "/tmp/toolchain",
        "a/b",
        "a\\b",
        "-stable",
        "v",
        "nightly",
        "v1.2.x",
    ] {
        assert!(release_name(name).is_err(), "{name}");
    }
    assert_eq!(
        latest_tag_from_url(&format!("{RELEASES}/tag/v1.2.3")).unwrap(),
        "v1.2.3"
    );
    for url in [
        "http://github.com/Rock-lang-org/Rock/releases/tag/v1.2.3",
        "https://evil.test/tag/v1.2.3",
        "https://github.com/Rock-lang-org/Rock/releases/latest",
        "https://github.com/Rock-lang-org/Rock/releases/tag/1.2.3",
    ] {
        assert!(latest_tag_from_url(url).is_err());
    }
}

#[test]
fn archive_entries_reject_traversal_links_and_escaped_names() {
    use tar::EntryType;
    assert_eq!(
        safe_entry_path(EntryType::Regular, b"./bin/rock").unwrap(),
        PathBuf::from("bin").join("rock")
    );
    assert!(safe_entry_path(EntryType::Directory, b"./").is_ok());
    for name in [
        &b"/bin/rock"[..],
        b"../rock",
        b"bin/../../rock",
        b"bin/evil\nname",
        b"bin/a b",
        "bin/\u{e9}".as_bytes(),
        b"bin/\xff",
        b"",
    ] {
        assert!(
            safe_entry_path(EntryType::Regular, name).is_err(),
            "{}",
            String::from_utf8_lossy(name)
        );
    }
    for kind in [
        EntryType::Symlink,
        EntryType::Link,
        EntryType::Block,
        EntryType::Char,
        EntryType::Fifo,
    ] {
        assert!(safe_entry_path(kind, b"bin/rock").is_err());
    }
}

fn write_archive(path: &Path, entries: &[(&str, tar::EntryType, &[u8], u32)]) {
    let file = fs::File::create(path).unwrap();
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
    let mut builder = tar::Builder::new(encoder);
    for (name, kind, data, mode) in entries {
        let mut header = tar::Header::new_ustar();
        header.set_entry_type(*kind);
        header.set_mode(*mode);
        header.set_size(data.len() as u64);
        if *kind == tar::EntryType::Symlink {
            header.set_link_name("../outside").unwrap();
        }
        // Raw names: set_path would refuse the unsafe ones under test.
        header.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
        header.set_cksum();
        builder.append(&header, *data).unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap();
}

#[test]
fn archives_extract_regular_files_and_reject_unsafe_entries_before_writing() {
    let root = canonical_temp_dir().join(format!("rockup-extract-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let good = root.join("good.tar.gz");
    write_archive(
        &good,
        &[
            ("bin/", tar::EntryType::Directory, b"", 0o755),
            ("bin/rock", tar::EntryType::Regular, b"binary", 0o755),
            ("lib/notes.txt", tar::EntryType::Regular, b"text", 0o644),
        ],
    );
    let out = root.join("good");
    extract_verified_archive(&good, &out).unwrap();
    assert_eq!(fs::read(out.join("bin/rock")).unwrap(), b"binary");
    assert_eq!(fs::read(out.join("lib/notes.txt")).unwrap(), b"text");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &str| fs::metadata(out.join(path)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode("bin/rock"), 0o755);
        assert_eq!(mode("lib/notes.txt"), 0o644);
    }

    for (label, entries) in [
        (
            "link",
            vec![
                ("bin/rock", tar::EntryType::Regular, &b"binary"[..], 0o755),
                ("bin/escape", tar::EntryType::Symlink, &b""[..], 0o777),
            ],
        ),
        (
            "traversal",
            vec![
                ("bin/rock", tar::EntryType::Regular, &b"binary"[..], 0o755),
                ("../escape", tar::EntryType::Regular, &b"x"[..], 0o644),
            ],
        ),
        (
            "duplicate",
            vec![
                ("bin/rock", tar::EntryType::Regular, &b"one"[..], 0o755),
                ("bin/rock", tar::EntryType::Regular, &b"two"[..], 0o755),
            ],
        ),
    ] {
        let archive = root.join(format!("{label}.tar.gz"));
        write_archive(&archive, &entries);
        let out = root.join(label);
        assert!(extract_verified_archive(&archive, &out).is_err(), "{label}");
        if label != "duplicate" {
            // Rejected during validation: nothing was written.
            assert!(!out.exists(), "{label}");
        }
    }
    assert!(!root.join("escape").exists());

    let empty = root.join("empty.tar.gz");
    write_archive(&empty, &[]);
    assert!(extract_verified_archive(&empty, &root.join("empty")).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn checksums_are_computed_in_process() {
    let root = canonical_temp_dir().join(format!("rockup-checksum-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let asset = root.join("asset");
    fs::write(&asset, b"abc").unwrap();
    let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    assert_eq!(sha256_hex(&asset).unwrap(), expected);
    let sidecar = root.join("asset.sha256");
    fs::write(&sidecar, format!("{expected}  asset\n")).unwrap();
    verify_checksum(&asset, &sidecar, "asset").unwrap();
    fs::write(&sidecar, format!("{}  asset\n", "0".repeat(64))).unwrap();
    assert!(verify_checksum(&asset, &sidecar, "asset").is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn publication_rolls_back_and_staging_excludes_concurrent_operations() {
    let root = canonical_temp_dir().join(format!("rockup-rollback-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let mut stage = Stage::new(&root, "test").unwrap();
    assert!(Stage::new(&root, "test").is_err());
    let old = root.join("stable");
    let new = stage.path.join("new");
    fs::create_dir(&old).unwrap();
    fs::write(old.join("version"), "old").unwrap();
    fs::create_dir(&new).unwrap();
    fs::write(new.join("version"), "new").unwrap();
    assert!(stage
        .publish(&new, &old, || Err("injected failure".into()))
        .is_err());
    assert_eq!(fs::read_to_string(old.join("version")).unwrap(), "old");
    assert_eq!(fs::read_to_string(new.join("version")).unwrap(), "new");
    stage.publish(&new, &old, || Ok(())).unwrap();
    assert_eq!(fs::read_to_string(old.join("version")).unwrap(), "new");
    drop(stage);
    assert!(!root.join(".rockup-test").exists());
    fs::remove_dir_all(root).unwrap();
}

// The tools report resolved paths: Darwin's temp dir is a symlink into
// /private, and Windows' may use 8.3 short names.
fn canonical_temp_dir() -> std::path::PathBuf {
    dunce::canonicalize(std::env::temp_dir()).unwrap()
}

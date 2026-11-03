use super::*;

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
    let available = collect_available_releases(|page| {
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
    let available = collect_available_releases(|page| {
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
    assert!(collect_available_releases(|_| Ok(b"[]".to_vec()))
        .unwrap()
        .is_empty());
    for body in ["not json", "{}", "[{\"tag_name\":\"v1.0.0\"}]"] {
        assert!(collect_available_releases(|_| Ok(body.as_bytes().to_vec())).is_err());
    }
    assert!(matches!(
        collect_available_releases(|_| Err("offline".into())),
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
fn archive_listing_rejects_traversal_links_and_escaped_names() {
    assert!(validate_archive_listing("./\n./bin/rock\n", "drwx metadata\n-rwx metadata\n").is_ok());
    for name in [
        "/bin/rock",
        "../rock",
        "bin/../../rock",
        "bin/evil\\nname",
        "bin/a b",
        "bin/\u{e9}",
    ] {
        assert!(
            validate_archive_listing(name, "-rwx metadata").is_err(),
            "{name}"
        );
    }
    for kind in ['l', 'h', 'b', 'c', 'p', 's'] {
        assert!(validate_archive_listing("bin/rock", &format!("{kind} metadata")).is_err());
    }
    assert!(validate_archive_listing("", "").is_err());
}

#[test]
fn publication_rolls_back_and_staging_excludes_concurrent_operations() {
    let root = std::env::temp_dir().join(format!("rockup-rollback-{}", std::process::id()));
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

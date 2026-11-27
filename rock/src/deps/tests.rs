use super::*;

#[test]
fn pinned_dependency_fetches_exact_commit_and_reuses_checkout() {
    let root = std::env::temp_dir().join(format!("rock-git-fetch-test-{}", std::process::id()));
    let source = root.join("source");
    std::fs::create_dir_all(&source).unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .current_dir(&source)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };
    git(&["init", "--quiet"]);
    std::fs::write(source.join("rock.toml"), "pinned").unwrap();
    git(&["add", "rock.toml"]);
    git(&[
        "-c",
        "user.name=test",
        "-c",
        "user.email=test@example.invalid",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-qm",
        "fixture",
    ]);
    let rev = git(&["rev-parse", "HEAD"]);
    let cache = root.join("cache");
    let url = format!("file://{}", source.display());
    let checkout = fetch_into_cache(&cache, &url, &rev).unwrap();
    assert_eq!(
        std::fs::read_to_string(checkout.join("rock.toml")).unwrap(),
        "pinned"
    );
    assert_eq!(
        fetch_into_cache(&cache, "unavailable", &rev).unwrap(),
        checkout
    );
    std::fs::write(checkout.join(".git/info/exclude"), "extra.rk\n").unwrap();
    std::fs::write(checkout.join("extra.rk"), "ignored").unwrap();
    assert!(!cached_checkout_matches(&checkout, &rev).unwrap());
    std::fs::remove_file(checkout.join("extra.rk")).unwrap();

    // An edited cache no longer matches the pin and is fetched again; it
    // survives a failed refetch instead of being deleted up front.
    std::fs::write(checkout.join("rock.toml"), "tampered").unwrap();
    assert!(fetch_into_cache(&cache, "file:///nonexistent-rock-repo", &rev).is_err());
    assert_eq!(
        std::fs::read_to_string(checkout.join("rock.toml")).unwrap(),
        "tampered"
    );
    let refetched = fetch_into_cache(&cache, &url, &rev).unwrap();
    assert_ne!(refetched, checkout);
    assert_eq!(
        std::fs::read_to_string(checkout.join("rock.toml")).unwrap(),
        "tampered"
    );
    assert_eq!(
        fetch_into_cache(&cache, "unavailable", &rev).unwrap(),
        refetched
    );
    assert_eq!(
        std::fs::read_to_string(refetched.join("rock.toml")).unwrap(),
        "pinned"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn interrupted_checkout_is_replaced_on_next_fetch() {
    let root = std::env::temp_dir().join(format!("rock-git-partial-test-{}", std::process::id()));
    let source = root.join("source");
    std::fs::create_dir_all(&source).unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .current_dir(&source)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };
    git(&["init", "--quiet"]);
    std::fs::write(source.join("rock.toml"), "pinned").unwrap();
    git(&["add", "rock.toml"]);
    git(&[
        "-c",
        "user.name=test",
        "-c",
        "user.email=test@example.invalid",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-qm",
        "fixture",
    ]);
    let rev = git(&["rev-parse", "HEAD"]);
    let cache = root.join("cache");
    std::fs::create_dir_all(cache.join("leftover")).unwrap();
    let url = format!("file://{}", source.display());
    let checkout = fetch_into_cache(&cache, &url, &rev).unwrap();
    assert_eq!(
        std::fs::read_to_string(checkout.join("rock.toml")).unwrap(),
        "pinned"
    );
    assert!(!checkout.join("leftover").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn git_checkout_path_dependencies_cannot_escape_the_checkout() {
    let root = std::env::temp_dir().join(format!("rock-git-escape-test-{}", std::process::id()));
    let checkout = root.join("build/git/owner/repo/0123456789abcdef0123456789abcdef01234567");
    std::fs::create_dir_all(checkout.join("inner")).unwrap();
    std::fs::create_dir_all(root.join("outside")).unwrap();
    let path_dep = |path: &str| Dependency {
        path: Some(path.into()),
        version: None,
        git: None,
        rev: None,
    };
    let inner = resolve_dependency_root(&checkout, &root, "inner", &path_dep("inner"));
    assert_eq!(
        inner.unwrap(),
        checkout.join("inner").canonicalize().unwrap()
    );
    let escaped =
        resolve_dependency_root(&checkout, &root, "out", &path_dep("../../../../../outside"));
    assert!(escaped.unwrap_err().contains("escapes its Git checkout"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn pinned_dependency_uses_project_cache_without_network() {
    let root = std::env::temp_dir().join(format!("rock-git-cache-test-{}", std::process::id()));
    let scratch = root.join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    let git = |dir: &Path, args: &[&str]| {
        let output = Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };
    git(&scratch, &["init", "--quiet"]);
    std::fs::write(scratch.join("rock.toml"), "").unwrap();
    git(&scratch, &["add", "rock.toml"]);
    git(
        &scratch,
        &[
            "-c",
            "user.name=Rock",
            "-c",
            "user.email=rock@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "pin",
        ],
    );
    let rev = git(&scratch, &["rev-parse", "HEAD"]);
    let rev = rev.as_str();
    let cached = root.join("build/git/owner/repo").join(rev);
    std::fs::create_dir_all(cached.parent().unwrap()).unwrap();
    std::fs::rename(&scratch, &cached).unwrap();
    let dependency = Dependency {
        git: Some("https://github.com/owner/repo".into()),
        rev: Some(rev.into()),
        path: None,
        version: None,
    };
    let result = resolve_dependency_root(&root.join("child"), &root, "repo", &dependency);
    assert_eq!(result.unwrap(), cached.canonicalize().unwrap());
    std::fs::remove_dir_all(root).unwrap();
}

/// Commits `rock.toml` plus `links` (name, target) as symbolic links into a
/// fresh repository at `source` and returns the commit ID.
#[cfg(unix)]
fn commit_fixture(source: &Path, links: &[(&str, &str)]) -> String {
    std::fs::create_dir_all(source).unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .current_dir(source)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };
    git(&["init", "--quiet"]);
    std::fs::write(source.join("rock.toml"), "").unwrap();
    for (name, target) in links {
        std::os::unix::fs::symlink(target, source.join(name)).unwrap();
    }
    git(&["add", "--all"]);
    git(&[
        "-c",
        "user.name=test",
        "-c",
        "user.email=test@example.invalid",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-qm",
        "fixture",
    ]);
    git(&["rev-parse", "HEAD"])
}

// Creating symlinks on Windows needs Developer Mode or elevation.
#[cfg(unix)]
#[test]
fn checkouts_reject_symbolic_links_leaving_the_checkout() {
    let root = std::env::temp_dir().join(format!("rock-git-symlink-test-{}", std::process::id()));
    let cases: [(&str, &[(&str, &str)], bool); 3] = [
        ("internal", &[("alias.toml", "rock.toml")], true),
        ("escaping", &[("secret", "../../../../etc/hostname")], false),
        ("dangling", &[("missing", "nowhere")], false),
    ];
    for (name, links, accepted) in cases {
        let source = root.join(name).join("source");
        let rev = commit_fixture(&source, links);
        let url = format!("file://{}", source.display());
        let result = fetch_into_cache(&root.join(name).join("cache"), &url, &rev);
        if accepted {
            assert!(result.is_ok(), "{name}: {result:?}");
        } else {
            assert!(
                result
                    .unwrap_err()
                    .contains("symbolic link leaving its checkout"),
                "{name}"
            );
            assert!(!root.join(name).join("cache").exists(), "{name}");
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn git_cache_dir_normalizes_repository_spelling() {
    let rev = "0123456789ABCDEF0123456789ABCDEF01234567";
    let expected = Path::new("/p/build/git/owner/repo").join(rev.to_ascii_lowercase());
    for url in [
        "https://github.com/owner/repo",
        "https://github.com/Owner/Repo",
        "https://github.com/OWNER/repo.git",
    ] {
        assert_eq!(
            git_cache_dir(Path::new("/p"), url, rev).unwrap(),
            expected,
            "{url}"
        );
    }
}

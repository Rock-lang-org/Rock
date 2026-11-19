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
    let rev = "0123456789abcdef0123456789abcdef01234567";
    let cached = root.join("build/git/owner/repo").join(rev);
    std::fs::create_dir_all(&cached).unwrap();
    std::fs::write(cached.join("rock.toml"), "").unwrap();
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

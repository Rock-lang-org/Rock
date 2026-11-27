use std::path::{Path, PathBuf};
use std::process::Command;

use rock_shared::manifest::Dependency;

pub(crate) fn resolve_dependency_root(
    package_root: &Path,
    project_root: &Path,
    dependency_name: &str,
    dependency: &Dependency,
) -> Result<PathBuf, String> {
    if let (Some(url), Some(rev)) = (&dependency.git, &dependency.rev) {
        return fetch_git_dependency(project_root, url, rev);
    }
    if let Some(path) = &dependency.path {
        let candidate = PathBuf::from(path);
        let resolved = if candidate.is_relative() {
            package_root.join(candidate)
        } else {
            candidate
        };

        let resolved = dunce::canonicalize(&resolved).map_err(|e| {
            format!(
                "Failed to resolve dependency '{}' at {}: {}",
                dependency_name,
                resolved.display(),
                e
            )
        })?;
        // A fetched Git checkout may only reference paths inside itself.
        if let Some(checkout) = git_checkout_root(project_root, package_root) {
            if !resolved.starts_with(&checkout) {
                return Err(format!(
                    "Dependency '{}' at {} escapes its Git checkout {}",
                    dependency_name,
                    resolved.display(),
                    checkout.display()
                ));
            }
        }
        return Ok(resolved);
    }

    if let Some(version) = &dependency.version {
        return Err(format!(
            "Registry dependency '{}@{}' is not supported yet",
            dependency_name, version
        ));
    }

    Err(format!(
        "Dependency '{}' must have either 'path' or 'version'",
        dependency_name
    ))
}

/// Returns the pinned checkout directory when `package_root` lies inside a
/// fetched Git dependency of this project.
fn git_checkout_root(project_root: &Path, package_root: &Path) -> Option<PathBuf> {
    let git_root = dunce::canonicalize(project_root.join("build").join("git")).ok()?;
    let package_root = dunce::canonicalize(&package_root).ok()?;
    let relative = package_root.strip_prefix(&git_root).ok()?;
    let mut checkout = git_root;
    let mut components = relative.components();
    for _ in 0..3 {
        checkout.push(components.next()?);
    }
    Some(checkout)
}

fn fetch_git_dependency(root: &Path, url: &str, rev: &str) -> Result<PathBuf, String> {
    fetch_into_cache(&git_cache_dir(root, url, rev)?, url, rev)
}

/// Base cache path `build/git/<owner>/<repo>/<rev>`, normalized like GitHub:
/// repositories: owner and repository names are case-insensitive and an
/// optional `.git` suffix names the same repository.
fn git_cache_dir(root: &Path, url: &str, rev: &str) -> Result<PathBuf, String> {
    let repo = url
        .strip_prefix("https://github.com/")
        .ok_or("Invalid GitHub repository")?
        .to_ascii_lowercase();
    let repo = repo.strip_suffix(".git").unwrap_or(&repo);
    Ok(root
        .join("build")
        .join("git")
        .join(repo)
        .join(rev.to_ascii_lowercase()))
}

/// A unique sibling path of `dir` for staging or publishing a checkout.
fn sibling_scratch_dir(dir: &Path, purpose: &str) -> Result<PathBuf, String> {
    let parent = dir.parent().ok_or("Missing Git cache parent")?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    Ok(parent.join(format!(".{purpose}-{}-{nanos}", std::process::id())))
}

fn fetch_into_cache(cache: &Path, url: &str, rev: &str) -> Result<PathBuf, String> {
    // Also accept an existing checkout at the base cache path.
    if cache.join("rock.toml").is_file() && cached_checkout_matches(cache, rev)? {
        return dunce::canonicalize(&cache).map_err(|error| error.to_string());
    }
    let parent = cache.parent().ok_or("Missing Git cache parent")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let base = cache
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Missing Git cache name")?;
    let generation = format!("{base}-checkout");
    let prefix = format!(".{generation}-");
    let mut checkouts = std::fs::read_dir(parent)
        .map_err(|error| error.to_string())?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    checkouts.sort_unstable();
    for checkout in checkouts {
        if checkout
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with(&prefix))
            && checkout.join("rock.toml").is_file()
            && cached_checkout_matches(&checkout, rev)?
        {
            return dunce::canonicalize(&checkout).map_err(|error| error.to_string());
        }
    }
    let stage = sibling_scratch_dir(cache, "fetch")?;
    std::fs::create_dir(&stage).map_err(|error| error.to_string())?;
    let git = |args: &[&str]| -> Result<(), String> {
        let output = Command::new("git")
            .current_dir(&stage)
            .env("GIT_TERMINAL_PROMPT", "0")
            .args(args)
            .output()
            .map_err(|error| format!("Could not run git: {error}"))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "Git dependency {url}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    };
    let result = (|| {
        git(&["init", "--quiet"])?;
        git(&["fetch", "--quiet", "--depth=1", "--", url, rev])?;
        git(&["checkout", "--quiet", "--detach", rev])?;
        // Submodules would pull unpinned, non-GitHub sources; refuse them.
        if stage.join(".gitmodules").exists() {
            return Err(format!(
                "Git dependency {url} uses submodules, which are not supported"
            ));
        }
        if !stage.join("rock.toml").is_file() {
            return Err(format!("Git dependency {url} has no root rock.toml"));
        }
        // Sources are read at build time; a link out of the checkout would
        // let a dependency pull arbitrary local files into the build.
        if let Some(link) = escaping_symlink(&stage)? {
            return Err(format!(
                "Git dependency {url} contains a symbolic link leaving its checkout: {}",
                link.strip_prefix(&stage).unwrap_or(&link).display()
            ));
        }
        // Publish at a new path. Builds that already resolved a checkout
        // keep their path, even if a later build needs a fresh copy.
        let checkout = sibling_scratch_dir(cache, &generation)?;
        std::fs::rename(&stage, &checkout).map_err(|error| error.to_string())?;
        dunce::canonicalize(&checkout).map_err(|error| error.to_string())
    })();
    if stage.is_dir() {
        let _ = std::fs::remove_dir_all(&stage);
    }
    result
}

/// Finds a symbolic link under `root` (outside `.git`) whose target is
/// missing or lies outside `root`.
fn escaping_symlink(root: &Path) -> Result<Option<PathBuf>, String> {
    let root = dunce::canonicalize(&root).map_err(|error| error.to_string())?;
    let mut pending = vec![root.clone()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            let kind = entry.file_type().map_err(|error| error.to_string())?;
            if kind.is_symlink() {
                let inside =
                    dunce::canonicalize(&path).is_ok_and(|target| target.starts_with(&root));
                if !inside {
                    return Ok(Some(path));
                }
            } else if kind.is_dir() && !(dir == root && entry.file_name() == ".git") {
                pending.push(path);
            }
        }
    }
    Ok(None)
}

/// Whether `cache` is a Git checkout of exactly `rev` without local edits.
/// Failing to run Git is an error rather than a mismatch, so a missing Git
/// never discards a valid checkout.
fn cached_checkout_matches(cache: &Path, rev: &str) -> Result<bool, String> {
    let git_dir = cache.join(".git");
    if !git_dir.is_dir() {
        return Ok(false);
    }
    // Explicit repository paths keep Git from walking up into an enclosing
    // project repository.
    let git = |args: &[&str]| -> Result<Option<String>, String> {
        let output = Command::new("git")
            .arg("--git-dir")
            .arg(&git_dir)
            .arg("--work-tree")
            .arg(cache)
            .env("GIT_TERMINAL_PROMPT", "0")
            .args(args)
            .output()
            .map_err(|error| format!("Could not run git to verify {}: {error}", cache.display()))?;
        Ok(output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string()))
    };
    Ok(
        git(&["rev-parse", "HEAD"])?.is_some_and(|head| head.eq_ignore_ascii_case(rev))
            && git(&[
                "status",
                "--porcelain",
                "--untracked-files=all",
                "--ignored",
            ])?
            .is_some_and(|status| status.is_empty())
            && escaping_symlink(cache)?.is_none(),
    )
}

#[cfg(test)]
mod tests;

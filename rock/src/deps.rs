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

        let resolved = resolved.canonicalize().map_err(|e| {
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

/// Returns `build/git/<owner>/<repo>/<rev>` when `package_root` lies inside a
/// fetched Git dependency of this project.
fn git_checkout_root(project_root: &Path, package_root: &Path) -> Option<PathBuf> {
    let git_root = project_root.join("build").join("git").canonicalize().ok()?;
    let package_root = package_root.canonicalize().ok()?;
    let relative = package_root.strip_prefix(&git_root).ok()?;
    let mut checkout = git_root;
    let mut components = relative.components();
    for _ in 0..3 {
        checkout.push(components.next()?);
    }
    Some(checkout)
}

fn fetch_git_dependency(root: &Path, url: &str, rev: &str) -> Result<PathBuf, String> {
    let repo = url
        .strip_prefix("https://github.com/")
        .ok_or("Invalid GitHub repository")?;
    let cache = root
        .join("build")
        .join("git")
        .join(repo)
        .join(rev.to_ascii_lowercase());
    fetch_into_cache(&cache, url, rev)
}

fn fetch_into_cache(cache: &Path, url: &str, rev: &str) -> Result<PathBuf, String> {
    if cache.join("rock.toml").is_file() {
        // Reuse a checkout only while it still sits on the pinned commit with
        // pristine sources; otherwise drop it and fetch a fresh copy.
        if cached_checkout_matches(cache, rev) {
            return cache.canonicalize().map_err(|error| error.to_string());
        }
        std::fs::remove_dir_all(cache).map_err(|error| error.to_string())?;
    }
    let parent = cache.parent().ok_or("Missing Git cache parent")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let stage = parent.join(format!(
        ".fetch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos()
    ));
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
        // Clear a leftover from an interrupted run so the rename can land.
        if cache.exists() && !cache.join("rock.toml").is_file() {
            std::fs::remove_dir_all(cache).map_err(|error| error.to_string())?;
        }
        match std::fs::rename(&stage, cache) {
            Ok(()) => {}
            Err(_) if cache.join("rock.toml").is_file() => {}
            Err(error) => return Err(error.to_string()),
        }
        cache.canonicalize().map_err(|error| error.to_string())
    })();
    if stage.is_dir() {
        let _ = std::fs::remove_dir_all(&stage);
    }
    result
}

fn cached_checkout_matches(cache: &Path, rev: &str) -> bool {
    let git = |args: &[&str]| {
        Command::new("git")
            .current_dir(cache)
            .env("GIT_TERMINAL_PROMPT", "0")
            .args(args)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
    };
    git(&["rev-parse", "HEAD"]).is_some_and(|head| head.eq_ignore_ascii_case(rev))
        && git(&["status", "--porcelain", "--untracked-files=all"])
            .is_some_and(|status| status.is_empty())
}

#[cfg(test)]
mod tests;

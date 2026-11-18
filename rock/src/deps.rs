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

        return resolved.canonicalize().map_err(|e| {
            format!(
                "Failed to resolve dependency '{}' at {}: {}",
                dependency_name,
                resolved.display(),
                e
            )
        });
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

fn fetch_git_dependency(root: &Path, url: &str, rev: &str) -> Result<PathBuf, String> {
    let repo = url.strip_prefix("https://github.com/").ok_or("Invalid GitHub repository")?;
    let cache = root.join("build").join("git").join(repo).join(rev.to_ascii_lowercase());
    fetch_into_cache(&cache, url, rev)
}

fn fetch_into_cache(cache: &Path, url: &str, rev: &str) -> Result<PathBuf, String> {
    if cache.join("rock.toml").is_file() {
        return cache.canonicalize().map_err(|error| error.to_string());
    }
    let parent = cache.parent().ok_or("Missing Git cache parent")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let stage = parent.join(format!(".fetch-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?.as_nanos()));
    std::fs::create_dir(&stage).map_err(|error| error.to_string())?;
    let git = |args: &[&str]| -> Result<(), String> {
        let output = Command::new("git").current_dir(&stage)
            .env("GIT_TERMINAL_PROMPT", "0").args(args).output()
            .map_err(|error| format!("Could not run git: {error}"))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!("Git dependency {url}: {}", String::from_utf8_lossy(&output.stderr).trim()))
        }
    };
    let result = (|| {
        git(&["init", "--quiet"])?;
        git(&["fetch", "--quiet", "--depth=1", "--", url, rev])?;
        git(&["checkout", "--quiet", "--detach", rev])?;
        if !stage.join("rock.toml").is_file() {
            return Err(format!("Git dependency {url} has no root rock.toml"));
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

#[cfg(test)]
mod tests;

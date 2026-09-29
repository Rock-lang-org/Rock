//! Shared sysroot contract.
//!
//! ```
//! let _vendor = rock_shared::sysroot::host_target_vendor();
//! let _env = rock_shared::sysroot::host_target_env();
//! ```

use std::{
    env,
    path::{Path, PathBuf},
};

pub const ROCK_SYSROOT_ENV: &str = "ROCK_SYSROOT";
pub const CARGO_TARGET_DIR_ENV: &str = "CARGO_TARGET_DIR";
pub const STDLIB_CRATE_NAME: &str = "stdlib";
pub const STDLIB_ARTIFACT_FILE_NAME: &str = "stdlib.rkca";
pub const STDLIB_OBJECT_FILE_NAME: &str = "stdlib.o";
pub const TOOLCHAIN_MANIFEST_FILE_NAME: &str = "manifest.json";
pub const COMPONENTS_MANIFEST_FILE_NAME: &str = "components.json";
pub const PRODUCT_ARTIFACT_FORMAT_VERSION: u32 = 47;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysrootSource {
    Cli,
    Env,
    CargoTargetDir,
    CurrentDirTarget,
    ExecutableRelative,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SysrootResolution {
    pub path: PathBuf,
    pub source: SysrootSource,
}

impl SysrootResolution {
    pub fn is_explicit(&self) -> bool {
        matches!(
            self.source,
            SysrootSource::Cli | SysrootSource::Env | SysrootSource::CargoTargetDir
        )
    }

    pub fn into_layout(self) -> SysrootLayout {
        SysrootLayout::new(self.path, host_target_triple())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SysrootLayout {
    pub sysroot: PathBuf,
    pub target_triple: String,
    pub target_libdir: PathBuf,
    pub stdlib_artifact: PathBuf,
    pub stdlib_object: PathBuf,
    pub manifest_path: PathBuf,
    pub components_path: PathBuf,
}

impl SysrootLayout {
    pub fn new(sysroot: PathBuf, target_triple: String) -> Self {
        let target_libdir = sysroot.join("lib").join("rocklib").join(&target_triple);

        Self {
            sysroot,
            target_triple,
            stdlib_artifact: target_libdir.join(STDLIB_ARTIFACT_FILE_NAME),
            stdlib_object: target_libdir.join(STDLIB_OBJECT_FILE_NAME),
            manifest_path: target_libdir.join(TOOLCHAIN_MANIFEST_FILE_NAME),
            components_path: target_libdir.join(COMPONENTS_MANIFEST_FILE_NAME),
            target_libdir,
        }
    }
}

pub fn resolve_sysroot(cli_sysroot: Option<&Path>) -> Result<SysrootResolution, String> {
    let current_exe =
        env::current_exe().map_err(|e| format!("Failed to resolve current executable: {}", e))?;
    let current_dir = env::current_dir()
        .map_err(|e| format!("Failed to resolve current working directory: {}", e))?;

    resolve_sysroot_from(
        cli_sysroot,
        env::var_os(ROCK_SYSROOT_ENV).map(PathBuf::from),
        env::var_os(CARGO_TARGET_DIR_ENV).map(PathBuf::from),
        sysroot_from_current_dir(&current_dir),
        &current_exe,
    )
}

pub fn resolve_sysroot_from(
    cli_sysroot: Option<&Path>,
    env_sysroot: Option<PathBuf>,
    cargo_target_dir: Option<PathBuf>,
    current_dir_sysroot: Option<PathBuf>,
    executable: &Path,
) -> Result<SysrootResolution, String> {
    if let Some(path) = cli_sysroot {
        return Ok(SysrootResolution {
            path: path.to_path_buf(),
            source: SysrootSource::Cli,
        });
    }

    if let Some(path) = env_sysroot {
        return Ok(SysrootResolution {
            path,
            source: SysrootSource::Env,
        });
    }

    // Installed toolchains use <sysroot>/bin. Their stdlib must not come
    // from a checkout or Cargo's build directory, even if the bundle is missing.
    if executable
        .parent()
        .is_some_and(|parent| parent.ends_with("bin"))
    {
        return Ok(SysrootResolution {
            path: sysroot_from_executable(executable)?,
            source: SysrootSource::ExecutableRelative,
        });
    }

    if let Some(path) = cargo_target_dir {
        return Ok(SysrootResolution {
            path,
            source: SysrootSource::CargoTargetDir,
        });
    }

    if let Some(path) = current_dir_sysroot {
        return Ok(SysrootResolution {
            path,
            source: SysrootSource::CurrentDirTarget,
        });
    }

    Ok(SysrootResolution {
        path: sysroot_from_executable(executable)?,
        source: SysrootSource::ExecutableRelative,
    })
}

pub fn sysroot_from_executable(executable: &Path) -> Result<PathBuf, String> {
    let bin_dir = executable.parent().ok_or_else(|| {
        format!(
            "Failed to derive sysroot from executable {}",
            executable.display()
        )
    })?;

    bin_dir.parent().map(Path::to_path_buf).ok_or_else(|| {
        format!(
            "Failed to derive sysroot parent from executable directory {}",
            bin_dir.display()
        )
    })
}

fn sysroot_from_current_dir(current_dir: &Path) -> Option<PathBuf> {
    for ancestor in current_dir.ancestors() {
        let candidate = ancestor.join("target");
        if candidate.exists() {
            return Some(candidate);
        }
    }

    None
}

pub fn host_target_triple() -> String {
    let arch = env::consts::ARCH;
    let vendor = host_target_vendor();
    let os = env::consts::OS;

    match os {
        "linux" => format!(
            "{}-{}-linux-{}",
            arch,
            vendor,
            host_target_env().unwrap_or("gnu")
        ),
        "macos" => format!("{}-apple-darwin", arch),
        "windows" => format!(
            "{}-{}-windows-{}",
            arch,
            vendor,
            host_target_env().unwrap_or("msvc")
        ),
        other => match host_target_env() {
            Some(target_env) => format!("{}-{}-{}-{}", arch, vendor, other, target_env),
            None => format!("{}-{}-{}", arch, vendor, other),
        },
    }
}

pub fn host_target_vendor() -> &'static str {
    if cfg!(target_vendor = "apple") {
        "apple"
    } else if cfg!(target_vendor = "pc") {
        "pc"
    } else {
        "unknown"
    }
}

pub fn host_target_env() -> Option<&'static str> {
    if cfg!(target_env = "gnu") {
        Some("gnu")
    } else if cfg!(target_env = "musl") {
        Some("musl")
    } else if cfg!(target_env = "msvc") {
        Some("msvc")
    } else if cfg!(target_env = "sgx") {
        Some("sgx")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{
        host_target_triple, resolve_sysroot_from, sysroot_from_executable, SysrootLayout,
        SysrootResolution, SysrootSource,
    };
    use std::{fs, path::PathBuf};

    #[test]
    fn test_sysroot_resolution_explicit_sources() {
        assert!(SysrootResolution {
            path: PathBuf::from("/cli"),
            source: SysrootSource::Cli,
        }
        .is_explicit());
        assert!(!SysrootResolution {
            path: PathBuf::from("/toolchains/stable"),
            source: SysrootSource::ExecutableRelative,
        }
        .is_explicit());
    }

    #[test]
    fn test_resolve_sysroot_prefers_cli_over_env() {
        let resolution = resolve_sysroot_from(
            Some(PathBuf::from("/cli").as_path()),
            Some(PathBuf::from("/env")),
            Some(PathBuf::from("/target-dir")),
            Some(PathBuf::from("/current-dir-target")),
            PathBuf::from("/toolchains/stable/bin/rockc").as_path(),
        )
        .unwrap();

        assert_eq!(resolution.path, PathBuf::from("/cli"));
        assert_eq!(resolution.source, SysrootSource::Cli);
    }

    #[test]
    fn test_resolve_sysroot_uses_executable_relative_fallback() {
        let resolution = resolve_sysroot_from(
            None,
            None,
            None,
            None,
            PathBuf::from("/toolchains/stable/bin/rockc").as_path(),
        )
        .unwrap();

        assert_eq!(resolution.path, PathBuf::from("/toolchains/stable"));
        assert_eq!(resolution.source, SysrootSource::ExecutableRelative);
    }

    #[test]
    fn test_installed_toolchain_ignores_workspace_and_cargo_target_dirs() {
        for binary in ["rock", "rockc", "rock-lsp"] {
            for cargo_target in [None, Some(PathBuf::from("/cargo-target"))] {
                let resolution = resolve_sysroot_from(
                    None,
                    None,
                    cargo_target,
                    Some(PathBuf::from("/workspace/target")),
                    &PathBuf::from("/toolchains/stable/bin").join(binary),
                )
                .unwrap();

                assert_eq!(resolution.path, PathBuf::from("/toolchains/stable"));
                assert_eq!(resolution.source, SysrootSource::ExecutableRelative);
            }
        }
    }

    #[test]
    fn test_explicit_env_sysroot_overrides_installed_toolchain() {
        let resolution = resolve_sysroot_from(
            None,
            Some(PathBuf::from("/explicit")),
            Some(PathBuf::from("/cargo-target")),
            Some(PathBuf::from("/workspace/target")),
            &PathBuf::from("/toolchains/stable/bin/rock"),
        )
        .unwrap();

        assert_eq!(resolution.path, PathBuf::from("/explicit"));
        assert_eq!(resolution.source, SysrootSource::Env);
    }

    #[test]
    fn test_dev_binary_preserves_cargo_target_override() {
        let resolution = resolve_sysroot_from(
            None,
            None,
            Some(PathBuf::from("/cargo-target")),
            Some(PathBuf::from("/workspace/target")),
            &PathBuf::from("/workspace/target/debug/rock"),
        )
        .unwrap();

        assert_eq!(resolution.path, PathBuf::from("/cargo-target"));
        assert_eq!(resolution.source, SysrootSource::CargoTargetDir);
    }

    #[test]
    fn test_sysroot_from_executable_uses_parent_of_bin_dir() {
        let sysroot =
            sysroot_from_executable(PathBuf::from("/toolchains/stable/bin/rockc").as_path())
                .unwrap();

        assert_eq!(sysroot, PathBuf::from("/toolchains/stable"));
    }

    #[test]
    fn test_sysroot_layout_matches_contract() {
        let target = host_target_triple();
        let layout = SysrootLayout::new(PathBuf::from("/toolchains/stable"), target.clone());

        assert_eq!(layout.target_triple, target);
        assert_eq!(
            layout.target_libdir,
            PathBuf::from("/toolchains/stable/lib/rocklib").join(&layout.target_triple)
        );
        assert_eq!(
            layout.stdlib_artifact,
            layout.target_libdir.join("stdlib.rkca")
        );
        assert_eq!(layout.stdlib_object, layout.target_libdir.join("stdlib.o"));
        assert_eq!(
            layout.manifest_path,
            layout.target_libdir.join("manifest.json")
        );
        assert_eq!(
            layout.components_path,
            layout.target_libdir.join("components.json")
        );
    }

    #[test]
    fn test_sysroot_from_current_dir_finds_workspace_target() {
        let unique = format!("rock-sysroot-test-{}", std::process::id());
        let base = std::env::temp_dir().join(unique);
        let nested = base.join("projects").join("demo");
        let target = base.join("target");
        let layout = SysrootLayout::new(target.clone(), host_target_triple());

        fs::create_dir_all(&nested).unwrap();
        fs::create_dir_all(&layout.target_libdir).unwrap();
        fs::write(&layout.stdlib_artifact, b"artifact").unwrap();

        let resolution = resolve_sysroot_from(
            None,
            None,
            None,
            Some(target.clone()),
            base.join("target/debug/rockc").as_path(),
        )
        .unwrap();
        assert_eq!(resolution.path, target);
        assert_eq!(resolution.source, SysrootSource::CurrentDirTarget);

        let _ = fs::remove_dir_all(&base);
    }
}

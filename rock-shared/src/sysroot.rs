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
pub const STD_CRATE_NAME: &str = "std";
pub const STD_ARTIFACT_FILE_NAME: &str = "std.rkca";
pub const STD_OBJECT_FILE_NAME: &str = "std.o";
pub const TOOLCHAIN_MANIFEST_FILE_NAME: &str = "manifest.json";
pub const COMPONENTS_MANIFEST_FILE_NAME: &str = "components.json";
pub const PRODUCT_ARTIFACT_FORMAT_VERSION: u32 = 52;

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
    pub std_artifact: PathBuf,
    pub std_object: PathBuf,
    pub manifest_path: PathBuf,
    pub components_path: PathBuf,
}

impl SysrootLayout {
    pub fn new(sysroot: PathBuf, target_triple: String) -> Self {
        let target_libdir = sysroot.join("lib").join("rocklib").join(&target_triple);

        Self {
            sysroot,
            target_triple,
            std_artifact: target_libdir.join(STD_ARTIFACT_FILE_NAME),
            std_object: target_libdir.join(STD_OBJECT_FILE_NAME),
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

    // Installed toolchains use <sysroot>/bin. Their std must not come
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
mod tests;

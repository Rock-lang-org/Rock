use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(test)]
use std::{collections::HashSet, sync::Mutex};

use rock_shared::{process, sysroot::STD_CRATE_NAME};

use crate::package::Package;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExternArtifact {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RockcInvocation {
    pub(crate) executable: PathBuf,
    pub(crate) args: Vec<OsString>,
}

impl RockcInvocation {
    fn new(executable: PathBuf) -> Self {
        Self {
            executable,
            args: Vec::new(),
        }
    }

    fn arg(mut self, value: impl Into<OsString>) -> Self {
        self.args.push(value.into());
        self
    }

    fn name_path_arg(name: &str, path: &Path) -> OsString {
        OsString::from(format!("{}={}", name, path.display()))
    }

    fn into_command(self) -> Command {
        let mut command = Command::new(self.executable);
        command.args(self.args);
        command
    }
}

pub(crate) fn resolve_rockc_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("ROCKC") {
        return Ok(PathBuf::from(path));
    }

    #[cfg(test)]
    ensure_dev_binary_built("rockc")?;

    process::dev_target_binary_from_current_exe("rockc")
}

#[cfg(test)]
pub(crate) fn ensure_dev_binary_built(package: &str) -> Result<(), String> {
    static BUILT_PACKAGES: std::sync::OnceLock<Mutex<HashSet<String>>> = std::sync::OnceLock::new();
    let built_packages = BUILT_PACKAGES.get_or_init(|| Mutex::new(HashSet::new()));
    let mut built_packages = built_packages
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if built_packages.contains(package) {
        return Ok(());
    }

    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("Failed to resolve workspace root for dev binary build")?;
    let binary = process::dev_target_binary_from_current_exe(package)?;
    let mut command = Command::new("cargo");
    command
        .current_dir(workspace_root)
        .args(["build", "-p", package]);
    if binary.parent().and_then(|dir| dir.file_name()) == Some(std::ffi::OsStr::new("release")) {
        command.arg("--release");
    }
    let output = command
        .output()
        .map_err(|e| format!("Failed to spawn cargo build for {}: {}", package, e))?;
    if !output.status.success() {
        return Err(format!(
            "cargo build -p {} failed with status {}; stdout:\n{}\nstderr:\n{}",
            package,
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    built_packages.insert(package.to_string());
    Ok(())
}

#[cfg(test)]
fn rockc_path_from_current_exe(current_exe: &Path) -> Result<PathBuf, String> {
    process::dev_target_binary_from_exe(current_exe, "rockc")
}

pub(crate) fn build_dependency_artifact_invocation(
    executable: PathBuf,
    package: &Package,
    dependency_artifacts: &[ExternArtifact],
) -> RockcInvocation {
    let mut invocation = RockcInvocation::new(executable)
        .arg(package.profile.flag())
        .arg("--crate-name")
        .arg(package.manifest.crate_.name.clone())
        .arg("--entry-file")
        .arg(package.entry_file())
        .arg("--output-dir")
        .arg(package.object_dir())
        .arg("--no-link")
        .arg("--emit-object")
        .arg(package.object_path())
        .arg("--emit-artifact")
        .arg(package.artifact_path());

    if package.manifest.crate_.no_std || package.manifest.crate_.name == STD_CRATE_NAME {
        invocation = invocation.arg("--no-std");
    }
    if package.manifest.crate_.no_std {
        invocation = invocation.arg("--no-prelude");
    }

    for artifact in dependency_artifacts {
        invocation = invocation
            .arg("--extern-artifact")
            .arg(RockcInvocation::name_path_arg(
                &artifact.name,
                &artifact.path,
            ));
    }

    invocation
}

pub(crate) fn build_root_invocation(
    executable: PathBuf,
    package: &Package,
    artifacts: &[ExternArtifact],
) -> RockcInvocation {
    let mut invocation = RockcInvocation::new(executable)
        .arg(package.profile.flag())
        .arg("--entry-file")
        .arg(package.entry_file())
        .arg("--output-dir")
        .arg(package.binary_build_dir());

    if package.manifest.crate_.no_std || package.manifest.crate_.name == STD_CRATE_NAME {
        invocation = invocation.arg("--no-std");
    }
    if package.manifest.crate_.no_std {
        invocation = invocation.arg("--no-prelude");
    }

    for artifact in artifacts {
        invocation = invocation
            .arg("--extern-artifact")
            .arg(RockcInvocation::name_path_arg(
                &artifact.name,
                &artifact.path,
            ));
    }

    invocation
}

pub(crate) fn run_invocation(
    invocation: RockcInvocation,
    package: &Package,
    progress: Option<&crate::progress::BuildProgress>,
    context: &str,
) -> Result<(), String> {
    let executable = invocation.executable.clone();

    #[cfg(test)]
    {
        let _ = (package, progress);
        let output = invocation.into_command().output().map_err(|e| {
            format!(
                "Failed to spawn rockc for {} using {}: {}",
                context,
                executable.display(),
                e
            )
        })?;

        if !output.status.success() {
            return Err(format!(
                "rockc failed for {} with status {}; stdout:\n{}\nstderr:\n{}",
                context,
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        return Ok(());
    }

    #[cfg(not(test))]
    {
        let status =
            crate::progress::run_command(&mut invocation.into_command(), package, progress)
                .map_err(|e| {
                    format!(
                        "Failed to spawn rockc for {} using {}: {}",
                        context,
                        executable.display(),
                        e
                    )
                })?;

        if !status.success() {
            return Err(format!(
                "rockc failed for {} with status {}",
                context, status
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests;

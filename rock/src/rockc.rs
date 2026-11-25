use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

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

    process::dev_target_binary_from_current_exe("rockc")
}

/// Arguments every package compilation shares: std/prelude selection, lint
/// overrides and the compilation target.
fn common_package_args(mut invocation: RockcInvocation, package: &Package) -> RockcInvocation {
    if package.manifest.crate_.no_std || package.manifest.crate_.name == STD_CRATE_NAME {
        invocation = invocation.arg("--no-std");
    }
    if package.manifest.crate_.no_std {
        invocation = invocation.arg("--no-prelude");
    }
    for arg in &package.lint_args {
        invocation = invocation.arg(arg);
    }
    if let Some(triple) = &package.target_triple {
        invocation = invocation.arg("--target").arg(triple.clone());
    }
    invocation
}

pub(crate) fn build_dependency_artifact_invocation(
    executable: PathBuf,
    package: &Package,
    dependency_artifacts: &[ExternArtifact],
) -> RockcInvocation {
    // Std is built as the sysroot packages it, whatever the consumer's
    // profile: optimized, and overflow-checked because its integer operators
    // decide checking for every program.
    let is_std = package.manifest.crate_.name == STD_CRATE_NAME;
    let profile_flag = if is_std {
        crate::package::BuildProfile::Release.flag()
    } else {
        package.profile.flag()
    };
    let mut invocation = RockcInvocation::new(executable).arg(profile_flag);
    if is_std {
        invocation = invocation.arg("--overflow-checks").arg("true");
    }
    let mut invocation = invocation
        .arg("--crate-name")
        .arg(package.manifest.crate_.name.clone())
        .arg("--entry-file")
        .arg(package.entry_file())
        .arg("--output-dir")
        .arg(package.object_dir())
        .arg("--crate-type")
        .arg("lib")
        .arg("--no-link")
        .arg("--emit-object")
        .arg(package.object_path())
        .arg("--emit-artifact")
        .arg(package.artifact_path());

    invocation = common_package_args(invocation, package);

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

    invocation = common_package_args(invocation, package);

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
    let spawn_error = |e: std::io::Error| {
        format!(
            "Failed to spawn rockc for {} using {}: {}",
            context,
            executable.display(),
            e
        )
    };
    let mut command = invocation.into_command();

    if crate::progress::captures_tool_output() {
        let output = command.output().map_err(spawn_error)?;
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

    let status =
        crate::progress::run_command(&mut command, package, progress).map_err(spawn_error)?;
    if !status.success() {
        return Err(format!(
            "rockc failed for {} with status {}",
            context, status
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests;

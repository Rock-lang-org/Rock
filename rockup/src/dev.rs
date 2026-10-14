use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::{fsutil::copy_directory_recursive, layout::host_target_triple};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RockcInvocation {
    pub(crate) executable: PathBuf,
    pub(crate) args: Vec<OsString>,
}

impl RockcInvocation {
    pub(crate) fn args_as_strings(&self) -> Vec<String> {
        self.args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    fn into_command(self) -> Command {
        let mut command = Command::new(self.executable);
        command.args(self.args);
        command
    }
}

fn resolve_rockc_path(override_path: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = override_path {
        return Ok(path.to_path_buf());
    }
    if let Some(path) = std::env::var_os("ROCKC") {
        return Ok(PathBuf::from(path));
    }
    rock_shared::process::dev_target_binary_from_current_exe("rockc")
}

pub(crate) fn build_std_package_invocation(
    executable: PathBuf,
    std_root: &Path,
    layout: &crate::layout::ToolchainLayout,
) -> Result<RockcInvocation, String> {
    let manifest = rock_shared::manifest::load_manifest(&std_root.join("rock.toml"))?;
    if manifest.crate_.name != rock_shared::sysroot::STD_CRATE_NAME {
        return Err(format!(
            "Expected std crate at {}, found '{}' instead",
            std_root.display(),
            manifest.crate_.name
        ));
    }

    Ok(RockcInvocation {
        executable,
        args: vec![
            "--crate-name".into(),
            rock_shared::sysroot::STD_CRATE_NAME.into(),
            "--entry-file".into(),
            std_root.join(&manifest.lib.path).into_os_string(),
            "--output-dir".into(),
            layout.target_component_dir.clone().into_os_string(),
            "--no-std".into(),
            "--no-prelude".into(),
            "--no-link".into(),
            "--emit-object".into(),
            layout.std_object.clone().into_os_string(),
            "--emit-artifact".into(),
            layout.std_artifact.clone().into_os_string(),
        ],
    })
}

pub(crate) fn package_dev_std(
    std_root: &Path,
    sysroot_root: &Path,
    target_triple: Option<String>,
    copy_source: bool,
    rockc: Option<&Path>,
) -> Result<PathBuf, String> {
    let std_root = std_root.canonicalize().map_err(|e| {
        format!(
            "Failed to resolve std source directory {}: {}",
            std_root.display(),
            e
        )
    })?;
    let manifest = rock_shared::manifest::load_manifest(&std_root.join("rock.toml"))?;
    if manifest.crate_.name != rock_shared::sysroot::STD_CRATE_NAME {
        return Err(format!(
            "Expected std crate at {}, found '{}' instead",
            std_root.display(),
            manifest.crate_.name
        ));
    }
    fs::create_dir_all(sysroot_root).map_err(|e| {
        format!(
            "Failed to create dev sysroot root {}: {}",
            sysroot_root.display(),
            e
        )
    })?;
    let sysroot_root = sysroot_root.canonicalize().map_err(|e| {
        format!(
            "Failed to resolve dev sysroot root {}: {}",
            sysroot_root.display(),
            e
        )
    })?;

    let layout = crate::layout::ToolchainLayout::new_for_target(
        sysroot_root,
        target_triple.unwrap_or_else(host_target_triple),
    );
    fs::create_dir_all(&layout.target_component_dir).map_err(|e| {
        format!(
            "Failed to create dev sysroot target libdir {}: {}",
            layout.target_component_dir.display(),
            e
        )
    })?;

    let executable = resolve_rockc_path(rockc)?;
    let invocation = build_std_package_invocation(executable, &std_root, &layout)?;
    let status = invocation
        .into_command()
        .status()
        .map_err(|e| format!("Failed to spawn rockc for dev std packaging: {}", e))?;
    if !status.success() {
        return Err(format!(
            "rockc failed for dev std packaging with status {}",
            status
        ));
    }

    write_sysroot_metadata(&layout, &manifest.crate_.version)?;

    if copy_source {
        install_std_source_component(&std_root, &layout.sysroot)?;
    }

    Ok(layout.target_component_dir)
}

fn write_sysroot_metadata(
    layout: &crate::layout::ToolchainLayout,
    std_version: &str,
) -> Result<(), String> {
    let manifest = [
        "{\n".to_string(),
        format!(
            "  \"toolchain_version\": \"dev-{}\",\n",
            env!("CARGO_PKG_VERSION")
        ),
        format!("  \"target_triple\": \"{}\",\n", layout.target_triple),
        "  \"std\": {\n".to_string(),
        format!("    \"crate_version\": \"{}\",\n", std_version),
        format!(
            "    \"artifact\": \"{}\",\n",
            rock_shared::sysroot::STD_ARTIFACT_FILE_NAME
        ),
        format!(
            "    \"object\": \"{}\",\n",
            rock_shared::sysroot::STD_OBJECT_FILE_NAME
        ),
        format!(
            "    \"artifact_format_version\": {}\n",
            rock_shared::sysroot::PRODUCT_ARTIFACT_FORMAT_VERSION
        ),
        "  }\n".to_string(),
        "}\n".to_string(),
    ]
    .concat();
    fs::write(&layout.manifest_path, manifest).map_err(|e| {
        format!(
            "Failed to write sysroot metadata {}: {}",
            layout.manifest_path.display(),
            e
        )
    })?;

    let components = [
        "{\n".to_string(),
        "  \"components\": {\n".to_string(),
        "    \"std\": {\n".to_string(),
        "      \"present\": true,\n".to_string(),
        format!(
            "      \"artifact\": \"{}\",\n",
            rock_shared::sysroot::STD_ARTIFACT_FILE_NAME
        ),
        format!(
            "      \"object\": \"{}\"\n",
            rock_shared::sysroot::STD_OBJECT_FILE_NAME
        ),
        "    }\n".to_string(),
        "  }\n".to_string(),
        "}\n".to_string(),
    ]
    .concat();
    fs::write(&layout.components_path, components).map_err(|e| {
        format!(
            "Failed to write sysroot component manifest {}: {}",
            layout.components_path.display(),
            e
        )
    })?;

    Ok(())
}

fn install_std_source_component(std_root: &Path, sysroot_root: &Path) -> Result<(), String> {
    let destination = sysroot_root
        .join("src")
        .join(rock_shared::sysroot::STD_CRATE_NAME);
    if destination.exists() {
        fs::remove_dir_all(&destination).map_err(|e| {
            format!(
                "Failed to replace std source component {}: {}",
                destination.display(),
                e
            )
        })?;
    }

    copy_directory_recursive(std_root, &destination)
}

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use rock_shared::{
    fs::{collect_package_source_inputs, file_modified},
    sysroot::{self, SysrootLayout, SysrootResolution, SysrootSource},
};

use crate::package::Package;

pub(crate) use rock_shared::sysroot::STD_CRATE_NAME;

fn workspace_std_root() -> Result<Option<PathBuf>, String> {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("Failed to resolve workspace root for bundled std")?;
    let std_root = workspace_root.join(STD_CRATE_NAME);
    if !std_root.exists() {
        return Ok(None);
    }

    std_root.canonicalize().map(Some).map_err(|e| {
        format!(
            "Failed to resolve bundled std at {}: {}",
            std_root.display(),
            e
        )
    })
}

fn current_sysroot_resolution() -> Result<SysrootResolution, String> {
    sysroot::resolve_sysroot(None).map_err(|e| format!("Failed to resolve sysroot: {}", e))
}

fn sysroot_std_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn package_has_direct_std_dependency(package: &Package) -> bool {
    package
        .manifest
        .dependencies
        .as_ref()
        .is_some_and(|dependencies| dependencies.contains_key(STD_CRATE_NAME))
}

pub(crate) fn package_requires_sysroot_std(package: &Package) -> bool {
    package.manifest.crate_.name != STD_CRATE_NAME
        && !package.manifest.crate_.no_std
        && !package_has_direct_std_dependency(package)
}

pub(crate) fn ensure_sysroot_std_available() -> Result<SysrootLayout, String> {
    let _guard = sysroot_std_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let resolution = current_sysroot_resolution()?;
    let layout = resolution.clone().into_layout();

    if resolution.is_explicit() {
        if sysroot_std_is_fresh(&layout, None)? {
            return Ok(layout);
        }

        return Err(format!(
            "Explicit sysroot {} does not contain a valid bundled std in {}",
            layout.sysroot.display(),
            layout.target_libdir.display()
        ));
    }

    let workspace_std_root = if resolution.source == SysrootSource::CurrentDirTarget {
        workspace_std_root()?
    } else {
        None
    };

    if sysroot_std_is_fresh(&layout, workspace_std_root.as_deref())? {
        return Ok(layout);
    }

    if !can_auto_rebuild_std(&resolution, workspace_std_root.as_deref()) {
        return Err(format!(
            "Bundled std is missing from sysroot {} and no workspace std source is available",
            layout.target_libdir.display()
        ));
    };

    let std_root = workspace_std_root.expect("checked by can_auto_rebuild_std");
    package_sysroot_std_with_rockup(&std_root, &layout)?;
    Ok(layout)
}

fn can_auto_rebuild_std(resolution: &SysrootResolution, workspace_std_root: Option<&Path>) -> bool {
    matches!(resolution.source, SysrootSource::CurrentDirTarget)
        && workspace_std_root
            .map(|root| root.join("rock.toml").exists())
            .unwrap_or(false)
}

fn sysroot_std_is_fresh(
    layout: &SysrootLayout,
    workspace_std_root: Option<&Path>,
) -> Result<bool, String> {
    if !layout.std_artifact.exists()
        || !layout.std_object.exists()
        || !layout.manifest_path.exists()
        || !layout.components_path.exists()
    {
        return Ok(false);
    }

    if !sysroot_std_artifact_format_is_current(&layout.manifest_path)? {
        return Ok(false);
    }

    if let Some(std_root) = workspace_std_root {
        let artifact_modified = file_modified(&layout.std_artifact)?;
        let object_modified = file_modified(&layout.std_object)?;
        for source_path in collect_package_source_inputs(std_root, crate::package::BUILD_DIR)? {
            let source_modified = file_modified(&source_path)?;
            if source_modified > artifact_modified || source_modified > object_modified {
                return Ok(false);
            }
        }
    }

    Ok(true)
}

fn sysroot_std_artifact_format_is_current(manifest_path: &Path) -> Result<bool, String> {
    let manifest = fs::read_to_string(manifest_path).map_err(|e| {
        format!(
            "Failed to read sysroot manifest {}: {}",
            manifest_path.display(),
            e
        )
    })?;
    let Some((_, after_key)) = manifest.split_once("\"artifact_format_version\"") else {
        return Ok(false);
    };
    let Some((_, after_colon)) = after_key.split_once(':') else {
        return Ok(false);
    };
    let value = after_colon.trim_start();
    let digit_count = value
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .map(char::len_utf8)
        .sum();
    if digit_count == 0 {
        return Ok(false);
    }

    let digits = &value[..digit_count];
    let remainder = value[digit_count..].trim_start();
    if !matches!(remainder.chars().next(), Some(',') | Some('}') | Some(']')) {
        return Ok(false);
    }

    let Ok(version) = digits.parse::<u32>() else {
        return Ok(false);
    };

    Ok(version == sysroot::PRODUCT_ARTIFACT_FORMAT_VERSION)
}

fn resolve_rockup_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("ROCKUP") {
        return Ok(PathBuf::from(path));
    }

    #[cfg(test)]
    crate::rockc::ensure_dev_binary_built("rockup")?;

    rock_shared::process::dev_target_binary_from_current_exe("rockup")
}

fn package_sysroot_std_with_rockup(std_root: &Path, layout: &SysrootLayout) -> Result<(), String> {
    let rockup = resolve_rockup_path()?;
    let rockc = crate::rockc::resolve_rockc_path()?;
    let mut command = std::process::Command::new(&rockup);
    command
        .args(["dev", "std", "package", "--path"])
        .arg(std_root)
        .arg("--sysroot")
        .arg(&layout.sysroot)
        .arg("--target")
        .arg(&layout.target_triple)
        .arg("--rockc")
        .arg(&rockc)
        .arg("--copy-source");

    let package = Package::load(std_root.to_path_buf())?;
    crate::progress::compiling(&package);

    #[cfg(test)]
    let status = command.output().map(|output| output.status).map_err(|e| {
        format!(
            "Failed to spawn rockup for dev sysroot std packaging: {}",
            e
        )
    })?;

    #[cfg(not(test))]
    let status = crate::progress::run_command(&mut command, &package, None).map_err(|e| {
        format!(
            "Failed to spawn rockup for dev sysroot std packaging: {}",
            e
        )
    })?;

    if !status.success() {
        return Err(format!(
            "rockup failed for dev sysroot std packaging with status {}",
            status
        ));
    }

    Ok(())
}

#[cfg(test)]
pub(crate) mod tests;

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use crate::{
    home::RockupHome,
    layout::{target_component_dir, ToolchainLayout},
    selection::active_toolchain_name,
    toolchain::validate_toolchain_layout,
};

pub(crate) fn add_target_component(
    home: &RockupHome,
    triple: &str,
    toolchain_override: Option<&str>,
    source: &Path,
) -> Result<PathBuf, String> {
    let current_dir = env::current_dir().map_err(|e| {
        format!(
            "Failed to resolve current directory for target install: {}",
            e
        )
    })?;
    add_target_component_from_dir(home, triple, toolchain_override, source, &current_dir)
}

pub(crate) fn add_target_component_from_dir(
    home: &RockupHome,
    triple: &str,
    toolchain_override: Option<&str>,
    source: &Path,
    current_dir: &Path,
) -> Result<PathBuf, String> {
    crate::home::validate_name(triple)?;
    let toolchain_name = match toolchain_override {
        Some(name) if !name.trim().is_empty() => name.to_string(),
        _ => active_toolchain_name(
            home,
            crate::selection::current_env_toolchain(),
            Some(current_dir),
        )?,
    };
    let toolchain_root = home.toolchain_dir(&toolchain_name)?;
    validate_toolchain_layout(&ToolchainLayout::new(toolchain_root.clone()))?;

    let source_dir = detect_target_component_source(source, triple)?;
    let destination_dir = target_component_dir(&toolchain_root, triple);
    install_target_component_dir(&source_dir, &destination_dir)?;
    validate_target_component_dir(&destination_dir)?;

    Ok(destination_dir)
}

pub(crate) fn detect_target_component_source(path: &Path, triple: &str) -> Result<PathBuf, String> {
    let path = path.canonicalize().map_err(|e| {
        format!(
            "Failed to resolve target component source {}: {}",
            path.display(),
            e
        )
    })?;

    if validate_target_component_dir(&path).is_ok() {
        return Ok(path);
    }

    let nested = target_component_dir(&path, triple);
    if validate_target_component_dir(&nested).is_ok() {
        return Ok(nested);
    }

    Err(format!(
        "{} does not contain a target component for {}",
        path.display(),
        triple
    ))
}

pub(crate) fn validate_target_component_dir(path: &Path) -> Result<(), String> {
    validate_target_component_paths(
        &path.join(crate::constants::STD_ARTIFACT_NAME),
        &path.join(crate::constants::STD_OBJECT_NAME),
        &path.join(crate::constants::TOOLCHAIN_MANIFEST_NAME),
        &path.join(crate::constants::COMPONENTS_MANIFEST_NAME),
        path,
    )
}

pub(crate) fn validate_target_component_paths(
    std_artifact: &Path,
    std_object: &Path,
    manifest_path: &Path,
    components_path: &Path,
    root: &Path,
) -> Result<(), String> {
    let required_paths = [
        (std_artifact, "std artifact"),
        (std_object, "std object"),
        (manifest_path, "toolchain manifest"),
        (components_path, "components manifest"),
    ];

    for (path, label) in required_paths {
        if !path.is_file() {
            return Err(format!(
                "Missing {} in target component at {}",
                label,
                root.display()
            ));
        }
    }

    Ok(())
}

pub(crate) fn install_target_component_dir(
    source_dir: &Path,
    destination_dir: &Path,
) -> Result<(), String> {
    fs::create_dir_all(destination_dir).map_err(|e| {
        format!(
            "Failed to create target component directory {}: {}",
            destination_dir.display(),
            e
        )
    })?;

    for file_name in [
        crate::constants::STD_ARTIFACT_NAME,
        crate::constants::STD_OBJECT_NAME,
        crate::constants::TOOLCHAIN_MANIFEST_NAME,
        crate::constants::COMPONENTS_MANIFEST_NAME,
    ] {
        crate::fsutil::copy_file_with_permissions(
            source_dir.join(file_name),
            destination_dir.join(file_name),
        )?;
    }

    Ok(())
}

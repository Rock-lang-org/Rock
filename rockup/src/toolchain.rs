use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
};

use crate::{
    constants::{
        BIN_DIR, COMPONENTS_MANIFEST_NAME, LIB_DIR, ROCKC_BIN_NAME, ROCKUP_TOOLCHAIN_ENV,
        ROCK_BIN_NAME, ROCK_LSP_BIN_NAME, STD_ARTIFACT_NAME, STD_OBJECT_NAME,
        TOOLCHAIN_MANIFEST_NAME,
    },
    fsutil::copy_directory_recursive,
    home::{installed_toolchain_names, RockupHome},
    layout::{cargo_build_dir_layout, CargoBuildDirLayout, ToolchainLayout},
    selection::{current_env_toolchain, default_toolchain_name, resolve_active_toolchain},
    shell::{ensure_shims, toolchain_prefixed_path},
    target::validate_target_component_paths,
};

#[derive(Debug)]
pub(crate) struct ToolchainListEntry {
    pub(crate) name: String,
    pub(crate) is_active: bool,
}

#[derive(Debug)]
pub(crate) enum InstallSourceKind {
    ToolchainRoot,
    CargoBuildDir,
}

#[derive(Debug)]
pub(crate) struct InstallSource {
    pub(crate) root: PathBuf,
    pub(crate) kind: InstallSourceKind,
}

pub(crate) fn install_toolchain(
    home: &RockupHome,
    name: &str,
    source: &Path,
) -> Result<PathBuf, String> {
    let destination = home.toolchain_dir(name)?;
    let should_set_default =
        !home.default_toolchain_file().exists() && installed_toolchain_names(home)?.is_empty();
    let source = source.canonicalize().map_err(|e| {
        format!(
            "Failed to resolve source toolchain directory {}: {}",
            source.display(),
            e
        )
    })?;
    let install_source = detect_install_source(&source)?;

    if destination.exists() {
        return Err(format!(
            "Toolchain '{}' already exists at {}",
            name,
            destination.display()
        ));
    }

    fs::create_dir_all(home.toolchains_dir()).map_err(|e| {
        format!(
            "Failed to create toolchains directory {}: {}",
            home.toolchains_dir().display(),
            e
        )
    })?;

    match install_source.kind {
        InstallSourceKind::ToolchainRoot => {
            copy_directory_recursive(&install_source.root, &destination)?
        }
        InstallSourceKind::CargoBuildDir => {
            install_from_cargo_build_dir(&install_source.root, &destination)?
        }
    }

    validate_toolchain_layout(&ToolchainLayout::new(destination.clone()))?;
    ensure_shims(home)?;
    if should_set_default {
        set_default_toolchain(home, name)?;
    }

    Ok(destination)
}

pub(crate) fn remove_toolchain(home: &RockupHome, name: &str) -> Result<PathBuf, String> {
    let toolchain_dir = home.toolchain_dir(name)?;
    if !toolchain_dir.exists() {
        return Err(format!(
            "Toolchain '{}' is not installed at {}",
            name,
            toolchain_dir.display()
        ));
    }

    fs::remove_dir_all(&toolchain_dir).map_err(|e| {
        format!(
            "Failed to remove toolchain '{}' at {}: {}",
            name,
            toolchain_dir.display(),
            e
        )
    })?;

    if default_toolchain_name(home).as_deref() == Ok(name) {
        let remaining = installed_toolchain_names(home)?;
        if let Some(next_default) = remaining.first() {
            set_default_toolchain(home, next_default)?;
        } else if home.default_toolchain_file().exists() {
            fs::remove_file(home.default_toolchain_file()).map_err(|e| {
                format!(
                    "Failed to clear default toolchain file {}: {}",
                    home.default_toolchain_file().display(),
                    e
                )
            })?;
        }
    }

    Ok(toolchain_dir)
}

pub(crate) fn list_toolchains(home: &RockupHome) -> Result<Vec<ToolchainListEntry>, String> {
    let current_dir = env::current_dir().ok();
    let active_name =
        resolve_active_toolchain(home, current_env_toolchain(), current_dir.as_deref())
            .ok()
            .map(|toolchain| toolchain.name);
    let names = installed_toolchain_names(home)?;

    Ok(names
        .into_iter()
        .map(|name| ToolchainListEntry {
            is_active: active_name.as_deref() == Some(name.as_str()),
            name,
        })
        .collect())
}

pub(crate) fn set_default_toolchain(home: &RockupHome, name: &str) -> Result<(), String> {
    validate_toolchain_layout(&ToolchainLayout::new(home.toolchain_dir(name)?))?;
    fs::create_dir_all(&home.root).map_err(|e| {
        format!(
            "Failed to create rockup home directory {}: {}",
            home.root.display(),
            e
        )
    })?;
    fs::write(home.default_toolchain_file(), format!("{}\n", name)).map_err(|e| {
        format!(
            "Failed to write default toolchain file {}: {}",
            home.default_toolchain_file().display(),
            e
        )
    })
}

pub(crate) fn run_toolchain_command(
    home: &RockupHome,
    toolchain_name: &str,
    command: &[String],
) -> Result<ExitStatus, String> {
    if command.is_empty() {
        return Err("Missing command to run".to_string());
    }

    let layout = ToolchainLayout::new(home.toolchain_dir(toolchain_name)?);
    validate_toolchain_layout(&layout)?;

    let mut child = Command::new(&command[0]);
    child.args(&command[1..]);
    child.env(ROCKUP_TOOLCHAIN_ENV, toolchain_name);
    child.env("PATH", toolchain_prefixed_path(&layout.bin_dir)?);

    child
        .status()
        .map_err(|e| format!("Failed to run '{}': {}", command[0], e))
}

pub(crate) fn proxy_toolchain_command(
    home: &RockupHome,
    binary: &str,
    args: &[String],
) -> Result<ExitStatus, String> {
    let current_dir = env::current_dir().map_err(|e| {
        format!(
            "Failed to resolve current directory for toolchain selection: {}",
            e
        )
    })?;
    proxy_toolchain_command_from_dir(home, binary, args, &current_dir)
}

pub(crate) fn proxy_toolchain_command_from_dir(
    home: &RockupHome,
    binary: &str,
    args: &[String],
    current_dir: &Path,
) -> Result<ExitStatus, String> {
    let toolchain = resolve_active_toolchain(home, current_env_toolchain(), Some(current_dir))?;
    let layout = ToolchainLayout::new(home.toolchain_dir(&toolchain.name)?);
    validate_toolchain_layout(&layout)?;

    let binary_path = match binary {
        ROCK_BIN_NAME => layout.rock_bin,
        ROCKC_BIN_NAME => layout.rockc_bin,
        ROCK_LSP_BIN_NAME => layout.rock_lsp_bin,
        _ => return Err(format!("Unsupported rockup shim target '{}'", binary)),
    };

    let mut child = Command::new(&binary_path);
    child.args(args);
    child.env(ROCKUP_TOOLCHAIN_ENV, &toolchain.name);

    child.status().map_err(|e| {
        format!(
            "Failed to run toolchain binary {}: {}",
            binary_path.display(),
            e
        )
    })
}

pub(crate) fn detect_install_source(path: &Path) -> Result<InstallSource, String> {
    if !path.is_dir() {
        return Err(format!(
            "Toolchain source {} must be a directory",
            path.display()
        ));
    }

    let direct_layout = ToolchainLayout::new(path.to_path_buf());
    if validate_toolchain_layout(&direct_layout).is_ok() {
        return Ok(InstallSource {
            root: path.to_path_buf(),
            kind: InstallSourceKind::ToolchainRoot,
        });
    }

    let cargo_layout = cargo_build_dir_layout(path.to_path_buf());
    if validate_cargo_build_dir(&cargo_layout).is_ok() {
        return Ok(InstallSource {
            root: path.to_path_buf(),
            kind: InstallSourceKind::CargoBuildDir,
        });
    }

    Err(format!(
        "{} is not a valid toolchain root or cargo build directory",
        path.display()
    ))
}

pub(crate) fn validate_toolchain_layout(layout: &ToolchainLayout) -> Result<(), String> {
    let required_paths = [
        (&layout.rock_bin, "rock binary"),
        (&layout.rockc_bin, "rockc binary"),
        (&layout.rock_lsp_bin, "rock-lsp binary"),
    ];

    for (path, label) in required_paths {
        if !path.is_file() {
            return Err(format!(
                "Missing {} in toolchain layout at {}",
                label,
                path.display()
            ));
        }
    }

    validate_target_component_paths(
        &layout.std_artifact,
        &layout.std_object,
        &layout.manifest_path,
        &layout.components_path,
        &layout.target_component_dir,
    )
}

fn install_from_cargo_build_dir(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination.join(BIN_DIR)).map_err(|e| {
        format!(
            "Failed to create destination bin directory {}: {}",
            destination.join(BIN_DIR).display(),
            e
        )
    })?;

    crate::fsutil::copy_file_with_permissions(
        source.join(ROCK_BIN_NAME),
        destination.join(BIN_DIR).join(ROCK_BIN_NAME),
    )?;
    crate::fsutil::copy_file_with_permissions(
        source.join(ROCKC_BIN_NAME),
        destination.join(BIN_DIR).join(ROCKC_BIN_NAME),
    )?;
    crate::fsutil::copy_file_with_permissions(
        source.join(ROCK_LSP_BIN_NAME),
        destination.join(BIN_DIR).join(ROCK_LSP_BIN_NAME),
    )?;

    for optional_dir in [LIB_DIR, "share", "src"] {
        let source_dir = source.join(optional_dir);
        if source_dir.exists() {
            copy_directory_recursive(&source_dir, &destination.join(optional_dir))?;
        }
    }

    Ok(())
}

fn validate_cargo_build_dir(layout: &CargoBuildDirLayout) -> Result<(), String> {
    let required_paths = [
        (&layout.rock_bin, "rock binary"),
        (&layout.rockc_bin, "rockc binary"),
        (&layout.rock_lsp_bin, "rock-lsp binary"),
        (
            &layout.target_libdir.join(STD_ARTIFACT_NAME),
            "std artifact",
        ),
        (
            &layout.target_libdir.join(STD_OBJECT_NAME),
            "std object",
        ),
        (
            &layout.target_libdir.join(TOOLCHAIN_MANIFEST_NAME),
            "toolchain manifest",
        ),
        (
            &layout.target_libdir.join(COMPONENTS_MANIFEST_NAME),
            "components manifest",
        ),
    ];

    for (path, label) in required_paths {
        if !path.exists() {
            return Err(format!(
                "Missing {} in cargo build directory layout at {}",
                label,
                path.display()
            ));
        }
    }

    Ok(())
}

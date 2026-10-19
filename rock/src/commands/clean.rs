use std::{fs, io::ErrorKind, path::Path};

use crate::package::{Package, BUILD_DIR};

pub(crate) fn clean_project(project_root: &Path) -> Result<bool, String> {
    let package = Package::load(project_root.to_path_buf())?;
    let build_dir = package.root_dir.join(BUILD_DIR);
    let metadata = match fs::symlink_metadata(&build_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("Failed to inspect {}: {}", build_dir.display(), error)),
    };

    // Never resolve a build symlink into another project's cache or source tree.
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "Refusing to clean {}: expected a directory, not a symlink or file",
            build_dir.display(),
        ));
    }

    fs::remove_dir_all(&build_dir)
        .map_err(|error| format!("Failed to remove {}: {}", build_dir.display(), error))?;
    Ok(true)
}

use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

pub fn file_modified(path: &Path) -> Result<SystemTime, String> {
    fs::metadata(path)
        .map_err(|e| format!("Failed to read metadata for {}: {}", path.display(), e))?
        .modified()
        .map_err(|e| format!("Failed to read modified time for {}: {}", path.display(), e))
}

pub fn collect_package_source_inputs(
    crate_root: &Path,
    build_dir_name: &str,
) -> Result<Vec<PathBuf>, String> {
    let mut inputs = vec![crate_root.join("rock.toml")];
    collect_rock_sources(crate_root, crate_root, build_dir_name, &mut inputs)?;
    inputs.sort();
    inputs.dedup();
    Ok(inputs)
}

fn collect_rock_sources(
    root: &Path,
    current: &Path,
    build_dir_name: &str,
    inputs: &mut Vec<PathBuf>,
) -> Result<(), String> {
    for entry in fs::read_dir(current).map_err(|e| {
        format!(
            "Failed to read package directory {} while checking artifact cache: {}",
            current.display(),
            e
        )
    })? {
        let entry = entry.map_err(|e| {
            format!(
                "Failed to read package directory entry in {} while checking artifact cache: {}",
                current.display(),
                e
            )
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|e| {
            format!(
                "Failed to read file type for {} while checking artifact cache: {}",
                path.display(),
                e
            )
        })?;

        if file_type.is_dir() {
            if path == root.join(build_dir_name) {
                continue;
            }
            collect_rock_sources(root, &path, build_dir_name, inputs)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rk") {
            inputs.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

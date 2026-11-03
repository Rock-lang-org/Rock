use std::path::{Path, PathBuf};

pub fn dev_target_binary_from_current_exe(binary_name: &str) -> Result<PathBuf, String> {
    let current_exe = std::env::current_exe()
        .map_err(|e| format!("Failed to resolve current executable path: {}", e))?;
    let invoked_exe = std::env::args_os().next().map(PathBuf::from);
    dev_target_binary_from_exes(&current_exe, invoked_exe.as_deref(), binary_name)
}

fn dev_target_binary_from_exes(
    current_exe: &Path,
    invoked_exe: Option<&Path>,
    binary_name: &str,
) -> Result<PathBuf, String> {
    let current_exe_candidate = dev_target_binary_from_exe(current_exe, binary_name)?;
    if current_exe_candidate.exists() {
        return Ok(current_exe_candidate);
    }

    // Sandboxed launchers can expose current_exe() through a private path while
    // preserving Cargo's original executable path in argv[0].
    if let Some(invoked_exe) = invoked_exe {
        let invoked_exe_candidate = dev_target_binary_from_exe(invoked_exe, binary_name)?;
        if invoked_exe_candidate.exists() {
            return Ok(invoked_exe_candidate);
        }
    }

    Ok(current_exe_candidate)
}

pub fn dev_target_binary_from_exe(
    current_exe: &Path,
    binary_name: &str,
) -> Result<PathBuf, String> {
    // Cargo's artifact-dir layout places test executables in
    // <profile>/build/<crate>/<hash>/out rather than <profile>/deps.
    let artifact_profile = current_exe
        .parent()
        .filter(|dir| dir.ends_with("out"))
        .and_then(|dir| dir.parent())
        .and_then(|dir| dir.parent())
        .and_then(|dir| dir.parent())
        .filter(|dir| dir.ends_with("build"))
        .and_then(|dir| dir.parent());
    let profile_dir = if let Some(profile_dir) = artifact_profile {
        profile_dir.to_path_buf()
    } else if current_exe
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        == Some("deps")
    {
        current_exe
            .parent()
            .and_then(|deps| deps.parent())
            .ok_or_else(|| {
                format!(
                    "Failed to resolve Cargo profile directory from {}",
                    current_exe.display()
                )
            })?
            .to_path_buf()
    } else {
        current_exe
            .parent()
            .ok_or_else(|| {
                format!(
                    "Failed to resolve executable directory from {}",
                    current_exe.display()
                )
            })?
            .to_path_buf()
    };

    let executable_name = if cfg!(windows) {
        format!("{}.exe", binary_name)
    } else {
        binary_name.to_string()
    };
    Ok(profile_dir.join(executable_name))
}

#[cfg(test)]
mod tests;

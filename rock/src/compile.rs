use std::path::{Path, PathBuf};

/// The executable rockc writes for `entry_file`; `exe_suffix` comes from the
/// compilation target, not the host running `rock`.
pub(crate) fn output_executable_path(
    output_dir: &Path,
    entry_file: &Path,
    exe_suffix: &str,
) -> PathBuf {
    let module_name = entry_file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("main");

    output_dir.join(format!("{}{}", module_name, exe_suffix))
}

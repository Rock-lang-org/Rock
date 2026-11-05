use std::process::{Command as ProcessCommand, ExitStatus};

use crate::{package::Package, rockc::resolve_rockc_path};

pub(crate) fn expand_project() -> Result<(), String> {
    let package = Package::load(
        std::env::current_dir().map_err(|e| format!("Failed to read current directory: {}", e))?,
    )?;
    let rockc = resolve_rockc_path()?;
    let mut command = ProcessCommand::new(&rockc);
    command
        .arg("--entry-file")
        .arg(package.entry_file())
        .arg("--expand");
    let status = run_expand_command(&mut command)?;

    if !status.success() {
        return Err(format!(
            "rockc failed for macro expansion with status {}",
            status
        ));
    }

    Ok(())
}

fn run_expand_command(command: &mut ProcessCommand) -> Result<ExitStatus, String> {
    crate::progress::run_tool_status(command)
        .map_err(|e| format!("Failed to spawn rockc for macro expansion: {}", e))
}

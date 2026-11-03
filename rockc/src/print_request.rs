use rock_lib::diagnostic::Diagnostics;

use super::artifact_deps::print_artifact_deps;
use super::{diagnostics_from_message, Config, PrintValue};

pub(super) fn print_request_output(config: &Config) -> Result<Option<String>, Diagnostics> {
    let Some(print_value) = config.print else {
        return Ok(None);
    };

    let output = match print_value {
        PrintValue::ArtifactDeps => print_artifact_deps(config)?,
        PrintValue::Sysroot => resolved_sysroot_layout(config)?
            .sysroot
            .display()
            .to_string(),
        PrintValue::TargetLibdir => resolved_sysroot_layout(config)?
            .target_libdir
            .display()
            .to_string(),
        PrintValue::TargetTriple => rock_lib::sysroot::host_target_triple(),
    };

    Ok(Some(output))
}

fn resolved_sysroot_layout(
    config: &Config,
) -> Result<rock_lib::sysroot::SysrootLayout, Diagnostics> {
    rock_lib::sysroot::resolve_sysroot(config.sysroot.as_deref())
        .map(|resolution| resolution.into_layout())
        .map_err(diagnostics_from_message)
}

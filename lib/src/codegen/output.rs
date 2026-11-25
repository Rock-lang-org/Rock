//! Output and linking.
//!
//! Handles writing LLVM IR, object files, and linking executables for the
//! selected compilation target.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;

use inkwell::passes::PassBuilderOptions;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target as LlvmTarget, TargetMachine,
    TargetTriple,
};
use inkwell::OptimizationLevel;

use super::{CodeGen, CodegenError};
use crate::target::{Target, TargetOs};

/// Overrides the linker driver for every target (e.g. a cross linker).
const LINKER_ENV: &str = "ROCK_LINKER";

fn target_machine(
    target: &Target,
    opt_level: OptimizationLevel,
) -> Result<TargetMachine, CodegenError> {
    static LLVM_INIT: Once = Once::new();
    LLVM_INIT.call_once(|| {
        LlvmTarget::initialize_all(&InitializationConfig::default());
    });

    let triple = TargetTriple::create(&target.triple);
    let llvm_target = LlvmTarget::from_triple(&triple).map_err(|e| {
        CodegenError::toolchain(format!(
            "LLVM has no backend for target '{}': {}",
            target.triple, e
        ))
    })?;
    // Executables are position-independent on Darwin and Windows.
    let reloc = match target.os {
        TargetOs::Linux => RelocMode::Default,
        TargetOs::MacOs | TargetOs::Windows => RelocMode::PIC,
    };
    llvm_target
        .create_target_machine(&triple, "generic", "", opt_level, reloc, CodeModel::Default)
        .ok_or_else(|| {
            CodegenError::toolchain(format!(
                "Failed to create a target machine for '{}'",
                target.triple
            ))
        })
}

impl<'ctx> CodeGen<'ctx> {
    /// Stamp the module with the target triple and data layout, so emitted IR
    /// and layout-dependent constants agree with object emission.
    pub(crate) fn configure_target(&self, target: &Target) -> Result<(), CodegenError> {
        let machine = target_machine(target, OptimizationLevel::None)?;
        self.module.set_triple(&machine.get_triple());
        self.module
            .set_data_layout(&machine.get_target_data().get_data_layout());
        Ok(())
    }

    /// Write the LLVM IR to a file
    pub(crate) fn write_ir(&self, path: &Path) -> Result<(), CodegenError> {
        self.module
            .print_to_file(path)
            .map_err(|e| CodegenError::output(e.to_string(), path))
    }

    /// Get the LLVM IR as a string
    pub(crate) fn get_ir(&self) -> String {
        self.module.print_to_string().to_string()
    }

    /// Compile to an object file for `target`.
    pub(crate) fn write_object(
        &self,
        path: &Path,
        target: &Target,
        opt_level: OptimizationLevel,
    ) -> Result<(), CodegenError> {
        let machine = target_machine(target, opt_level)?;
        self.module.set_triple(&machine.get_triple());
        self.module
            .set_data_layout(&machine.get_target_data().get_data_layout());

        // Run optimization passes (skip at O0 — saves time without losing correctness)
        if opt_level != OptimizationLevel::None {
            let passes = match opt_level {
                OptimizationLevel::None => unreachable!(),
                OptimizationLevel::Less => "default<O1>",
                OptimizationLevel::Default => "default<O2>",
                OptimizationLevel::Aggressive => "default<O3>",
            };
            self.module
                .run_passes(passes, &machine, PassBuilderOptions::create())
                .map_err(|e| CodegenError::toolchain(e.to_string()))?;
        }

        machine
            .write_to_file(&self.module, FileType::Object, path)
            .map_err(|e| CodegenError::output(e.to_string(), path))
    }

    /// Compile to an executable by producing an object and linking
    ///
    /// # Arguments
    /// * `output_path` - Path to the output executable
    /// * `target` - Target the executable runs on
    /// * `opt_level` - Optimization level (0-3)
    /// * `crate_objects` - List of object files from external crates to link
    pub(crate) fn write_executable(
        &self,
        output_path: &Path,
        target: &Target,
        opt_level: OptimizationLevel,
        crate_objects: &[PathBuf],
    ) -> Result<(), CodegenError> {
        let link = linker_invocation(target, output_path)?;
        let obj_path = output_path.with_extension(target.object_extension());
        self.write_object(&obj_path, target, opt_level)?;

        let mut cmd = Command::new(&link.program);
        cmd.args(&link.leading)
            .arg(&obj_path)
            .args(crate_objects)
            .args(&link.trailing);

        let status = cmd.status().map_err(|e| {
            CodegenError::link(format!(
                "Failed to run linker {}: {}",
                link.program.to_string_lossy(),
                e
            ))
        })?;

        if !status.success() {
            return Err(CodegenError::link(format!(
                "Linking failed with status {status}"
            )));
        }

        let _ = std::fs::remove_file(&obj_path);

        Ok(())
    }
}

/// A linker driver and the arguments placed before and after object inputs;
/// libraries come last so `--as-needed` linkers still resolve them.
struct LinkerInvocation {
    program: OsString,
    leading: Vec<OsString>,
    trailing: Vec<OsString>,
}

/// Selects the linker for `target`. A foreign target without an explicit
/// `ROCK_LINKER` is an error rather than a silent host link.
fn linker_invocation(
    target: &Target,
    output_path: &Path,
) -> Result<LinkerInvocation, CodegenError> {
    let override_linker = std::env::var_os(LINKER_ENV);
    if override_linker.is_none() && !target.is_host() {
        return Err(CodegenError::toolchain(format!(
            "cannot link for target '{}' on host '{}': set {} to a linker driver for that \
             target, or pass --no-link to emit an object file",
            target.triple,
            Target::host().triple,
            LINKER_ENV
        )));
    }

    let os_args = |values: &[&str]| values.iter().map(OsString::from).collect::<Vec<_>>();
    let invocation = match target.os {
        TargetOs::Linux => LinkerInvocation {
            program: override_linker.unwrap_or_else(|| "cc".into()),
            leading: vec!["-o".into(), output_path.into(), "-no-pie".into()],
            trailing: os_args(&["-lm", "-pthread"]),
        },
        // libm and pthreads live in libSystem on Darwin.
        TargetOs::MacOs => LinkerInvocation {
            program: override_linker.unwrap_or_else(|| "cc".into()),
            leading: vec!["-o".into(), output_path.into()],
            trailing: Vec::new(),
        },
        // MSVC link.exe; LIB from the developer environment locates the CRT
        // and SDK import libraries.
        TargetOs::Windows => {
            let mut out = OsString::from("/OUT:");
            out.push(output_path);
            LinkerInvocation {
                program: override_linker.unwrap_or_else(|| "link.exe".into()),
                leading: vec!["/NOLOGO".into(), out, "/SUBSYSTEM:CONSOLE".into()],
                trailing: os_args(&["libcmt.lib", "ws2_32.lib", "kernel32.lib"]),
            }
        }
    };
    Ok(invocation)
}

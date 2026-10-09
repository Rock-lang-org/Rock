//! Output and linking.
//!
//! Handles writing LLVM IR, object files, and linking executables.

use std::path::{Path, PathBuf};
use std::sync::Once;

use inkwell::passes::PassBuilderOptions;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::OptimizationLevel;

use super::{CodeGen, CodegenError};

impl<'ctx> CodeGen<'ctx> {
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

    /// Compile to an object file
    pub(crate) fn write_object(
        &self,
        path: &Path,
        opt_level: OptimizationLevel,
    ) -> Result<(), CodegenError> {
        static LLVM_INIT: Once = Once::new();
        LLVM_INIT.call_once(|| {
            Target::initialize_all(&InitializationConfig::default());
        });

        let triple = TargetMachine::get_default_triple();
        let target =
            Target::from_triple(&triple).map_err(|e| CodegenError::toolchain(e.to_string()))?;
        let machine = target
            .create_target_machine(
                &triple,
                "generic",
                "",
                opt_level,
                RelocMode::Default,
                CodeModel::Default,
            )
            .ok_or_else(|| CodegenError::toolchain("Failed to create target machine"))?;

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
    /// * `opt_level` - Optimization level (0-3)
    /// * `crate_objects` - List of object files from external crates to link
    pub(crate) fn write_executable(
        &self,
        output_path: &Path,
        opt_level: OptimizationLevel,
        crate_objects: &[PathBuf],
    ) -> Result<(), CodegenError> {
        let obj_path = output_path.with_extension("o");
        self.write_object(&obj_path, opt_level)?;

        let mut cmd = std::process::Command::new("cc");
        cmd.arg(&obj_path);

        let runtime_files = if self.descriptor_runtime_required || self.descriptor_nominal_runtime_required || self.dictionary_runtime_required {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let directory = output_path.parent().unwrap_or_else(|| Path::new("."))
                .join(format!(".rock-descriptor-runtime-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
            std::fs::create_dir(&directory).map_err(|error| CodegenError::output(error.to_string(), &directory))?;
            let source = super::descriptor_runtime::write_sources(&directory)
                .map_err(|error| CodegenError::output(error.to_string(), &directory))?;
            let object = directory.join("runtime.o");
            let compiled = std::process::Command::new("cc")
                .args(["-std=c11", "-O2", "-fPIC", "-pthread", "-c"])
                .arg(&source).arg("-o").arg(&object).output()
                .map_err(|error| CodegenError::toolchain(format!("cannot compile descriptor runtime: {error}")))?;
            if !compiled.status.success() {
                return Err(CodegenError::toolchain(format!("descriptor runtime compilation failed: {}", String::from_utf8_lossy(&compiled.stderr))));
            }
            cmd.arg(&object);
            for support in self.runtime_support()? {
                let roots = match support {
                    crate::products::ProductRuntimeSupport::TupleDescriptorsV1 { roots, .. }
                    | crate::products::ProductRuntimeSupport::NominalDescriptorsV1 { roots, .. }
                    | crate::products::ProductRuntimeSupport::SelectedDictionariesV1 { roots, .. } => roots,
                };
                for root in roots { cmd.arg(format!("-Wl,-u,{}", root.symbol())); }
            }
            Some((directory, source, object))
        } else { None };

        for crate_obj in crate_objects {
            cmd.arg(crate_obj);
        }

        cmd.arg("-o")
            .arg(output_path)
            .arg("-no-pie")
            .arg("-lm")
            .arg("-pthread");

        let status = cmd
            .status()
            .map_err(|e| CodegenError::link(format!("Failed to run linker: {}", e)))?;

        if !status.success() {
            return Err(CodegenError::link(format!(
                "Linking failed with status {status}"
            )));
        }

        let _ = std::fs::remove_file(&obj_path);
        if let Some((directory, source, object)) = runtime_files {
            let _ = std::fs::remove_file(source);
            let _ = std::fs::remove_file(object);
            let _ = std::fs::remove_file(directory.join("rock_descriptor_runtime.h"));
            let _ = std::fs::remove_dir(directory);
        }

        Ok(())
    }
}

//! Compilation pipeline: configuration, frontend (source to finalized HIR)
//! and backend (monomorphization, MIR, code generation, products).

mod backend;
mod config;
mod frontend;
#[cfg(test)]
mod tests;
pub(crate) mod timing;

pub use config::{Config, CrateKind, DebugPrint, SourceProvider};

use crate::analysis;
use crate::ast::Program;
use crate::diagnostic::Diagnostics;
use crate::products::CompilerProducts;

#[derive(Debug, Clone)]
pub struct CompileOutput {
    pub ast: Program,
    pub products: Option<CompilerProducts>,
    /// Lint warnings of a successful compilation.
    pub warnings: Diagnostics,
}

pub fn compile(config: &Config) -> Result<Program, Diagnostics> {
    backend::compile_impl(config, false, true).map(|output| output.ast)
}

/// Like [`compile`], but also returns the warnings of a successful build.
pub fn compile_with_warnings(config: &Config) -> Result<CompileOutput, Diagnostics> {
    backend::compile_impl(config, false, true)
}

pub fn compile_with_products(config: &Config) -> Result<CompileOutput, Diagnostics> {
    backend::compile_impl(config, true, true)
}

/// Runs every check a build runs (types, ownership, LLVM lowering) without
/// writing an object, executable or IR file.
pub fn check(config: &Config) -> Result<CompileOutput, Diagnostics> {
    backend::compile_impl(config, false, false)
}

/// Run the source frontend through finalized HIR without MIR or code generation.
pub fn analyze(config: &Config) -> Result<analysis::Analysis, Diagnostics> {
    frontend::compile_frontend(config).map(|frontend| frontend.analysis)
}

pub(crate) fn module_name_from_entry(entry_file: &std::path::Path) -> String {
    entry_file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("main")
        .to_string()
}

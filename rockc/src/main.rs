use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use rock_lib::{diagnostic::Diagnostics, DebugPrint};

mod artifact_deps;
mod artifact_emit;
mod artifact_validation;
mod print_request;
mod source_commands;
use artifact_emit::normalize_product_object_path;
#[cfg(test)]
use artifact_validation::resolve_artifact_object_path;
use artifact_validation::validate_artifact;
use print_request::print_request_output;
use source_commands::{expand_config, format_config};
#[cfg(test)]
use source_commands::{load_program_for_source_command, load_program_with_source_database};

fn main() {
    if let Err(e) = run() {
        e.report();

        std::process::exit(1);
    }
}

fn run() -> Result<(), Diagnostics> {
    run_config(Config::parse())
}

fn run_config(config: Config) -> Result<(), Diagnostics> {
    if let Some(output) = print_request_output(&config)? {
        println!("{}", output);
        return Ok(());
    }

    if config.format {
        return format_config(config);
    }

    if config.expand {
        return expand_config(config);
    }

    if let Some(path) = &config.validate_artifact {
        return validate_artifact(path);
    }

    let emit_artifact = config.emit_artifact.clone();
    let compiler_config = config.into_compiler_config()?;
    if emit_artifact.is_some() && !compiler_config.no_link {
        return Err(diagnostics_from_message(
            "--emit-artifact requires --no-link so the product artifact records an object file",
        ));
    }

    if let Some(path) = emit_artifact {
        let output = rock_lib::compile_with_products(&compiler_config)?;
        let mut products = output.products.ok_or_else(|| {
            diagnostics_from_message("Compiler did not produce product data for artifact emission")
        })?;
        normalize_product_object_path(&path, &mut products);
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|err| {
                diagnostics_from_message(format!(
                    "Failed to create artifact directory {}: {}",
                    parent.display(),
                    err
                ))
            })?;
        }
        products
            .write_artifact_to_path(&path)
            .map_err(diagnostics_from_message)?;
    } else {
        rock_lib::compile(&compiler_config)?;
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
enum PrintValue {
    ArtifactDeps,
    Sysroot,
    TargetLibdir,
    TargetTriple,
}

#[derive(Parser, Debug)]
#[command(version, about = "Rock compiler", long_about = None)]
pub struct Config {
    #[arg(long)]
    entry_file: Option<PathBuf>,
    #[arg(long, default_value = "build")]
    output_dir: PathBuf,
    #[arg(long, default_value = None)]
    debug_print: Option<String>,
    #[arg(long, value_enum)]
    print: Option<PrintValue>,
    /// Crate name for product identity and codegen symbol qualification
    #[arg(long)]
    crate_name: Option<String>,
    /// Build without optimization (default)
    #[arg(long, conflicts_with = "release")]
    debug: bool,
    /// Build with full optimization
    #[arg(long, conflicts_with = "debug")]
    release: bool,
    /// Override integer overflow checks (true/false), independently of optimization.
    #[arg(long, action = clap::ArgAction::Set)]
    overflow_checks: Option<bool>,
    /// Emit LLVM IR file
    #[arg(long)]
    emit_llvm: bool,
    /// Don't link, just produce object file
    #[arg(long)]
    no_link: bool,
    /// Emit product-backed compiler artifact to this path
    #[arg(long)]
    emit_artifact: Option<PathBuf>,
    /// Emit object file to this exact path
    #[arg(long)]
    emit_object: Option<PathBuf>,
    /// Don't inject std prelude
    #[arg(long)]
    no_prelude: bool,
    /// Don't auto-load bundled std from the sysroot
    #[arg(long)]
    no_std: bool,
    /// Override the compiler sysroot
    #[arg(long)]
    sysroot: Option<PathBuf>,
    /// External product artifacts in name=path format
    #[arg(long)]
    extern_artifact: Vec<String>,
    /// Format parsed modules in place and print them
    #[arg(long)]
    format: bool,
    /// Expand macros, print modules, and stop before lowering/compilation
    #[arg(long)]
    expand: bool,
    /// Validate a product artifact can be loaded and its object path exists
    #[arg(long)]
    validate_artifact: Option<PathBuf>,
}

impl Config {
    fn into_compiler_config(self) -> Result<rock_lib::Config, Diagnostics> {
        let config = self;
        let entry_file = config.entry_file.ok_or_else(|| {
            diagnostics_from_message("Missing required --entry-file unless --print is used")
        })?;

        let extern_artifacts = config
            .extern_artifact
            .iter()
            .map(|s| parse_name_path(s, "extern-artifact"))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rock_lib::Config {
            entry_file,
            output_dir: config.output_dir,
            debug_print: config
                .debug_print
                .map(|s| s.split(',').map(DebugPrint::from).collect())
                .unwrap_or_default(),
            meta_files: Vec::new(),
            extern_artifacts,
            source_providers: Vec::new(),
            current_crate_name: config.crate_name,
            opt_level: if config.release { 3 } else { 0 },
            overflow_checks: config.overflow_checks,
            emit_llvm: config.emit_llvm,
            no_link: config.no_link,
            emit_object: config.emit_object,
            no_prelude: config.no_prelude,
            no_std: config.no_std,
            sysroot: config.sysroot,
        })
    }
}

fn diagnostics_from_message(message: impl Into<String>) -> Diagnostics {
    let mut diagnostics = Diagnostics::default();
    diagnostics.push(rock_lib::diagnostic::Diagnostic::for_toolchain(
        message.into(),
    ));
    diagnostics
}

fn parse_name_path(input: &str, flag: &str) -> Result<(String, PathBuf), Diagnostics> {
    let Some((name, path)) = input.split_once('=') else {
        return Err(diagnostics_from_message(format!(
            "Invalid --{} format: '{}'. Expected name=path",
            flag, input
        )));
    };
    if name.is_empty() || path.is_empty() {
        return Err(diagnostics_from_message(format!(
            "Invalid --{} format: '{}'. Expected name=path",
            flag, input
        )));
    }

    Ok((name.to_string(), PathBuf::from(path)))
}

#[cfg(test)]
mod tests;

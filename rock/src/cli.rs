use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::{
    build::{build_project, run_project},
    commands::{clean_project, expand_project, format_project, new_project},
    package::BuildProfile,
    progress,
};

pub fn run() -> Result<CliOutcome, String> {
    let config = Config::parse();

    match config.command {
        Command::New { project_name } => {
            let path = new_project(&current_project_root()?, &project_name)?;
            println!("Created project '{}' at {}", project_name, path.display());
            println!("\n  cd {}\n  rock run", project_name);
            Ok(CliOutcome::Success)
        }
        Command::Format => format_project().map(|_| CliOutcome::Success),
        Command::Clean => {
            progress::cleaned(clean_project(&current_project_root()?)?);
            Ok(CliOutcome::Success)
        }
        Command::Build { profile } => {
            let products = build_project(&current_project_root()?, profile.selected(), config.no_proc_execution)?;
            for path in products.paths() {
                println!("{}", path.display());
            }
            Ok(CliOutcome::Success)
        }
        Command::Run { profile, args, bin } => run_project(&current_project_root()?, profile.selected(), &args, bin.as_deref(), config.no_proc_execution).map(CliOutcome::Exit),
        Command::Expand => expand_project(config.no_proc_execution).map(|_| CliOutcome::Success),
    }
}

pub enum CliOutcome {
    Success,
    Exit(i32),
}

fn current_project_root() -> Result<PathBuf, String> {
    std::env::current_dir().map_err(|e| format!("Failed to read current directory: {}", e))
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub(crate) struct Config {
    /// Never execute procedural macro workers while building or expanding
    #[arg(long, global = true)]
    pub(crate) no_proc_execution: bool,
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    /// Create a new hello-world project in a new directory
    New {
        project_name: String,
    },
    Format,
    /// Remove this project's build directory, including dependency artifacts
    Clean,
    /// Build the library and all binaries declared in rock.toml
    Build {
        #[command(flatten)]
        profile: ProfileArgs,
    },
    /// Build the crate and execute it if it is a binary
    Run {
        #[command(flatten)]
        profile: ProfileArgs,
        /// Select a binary target by name
        #[arg(long)]
        bin: Option<String>,
        #[arg(last = true)]
        args: Vec<String>,
    },
    Expand,
}

#[derive(Args, Debug)]
pub(crate) struct ProfileArgs {
    /// Build without optimization (default)
    #[arg(long, conflicts_with = "release")]
    debug: bool,
    /// Build with full optimization
    #[arg(long, conflicts_with = "debug")]
    release: bool,
}

impl ProfileArgs {
    pub(crate) fn selected(&self) -> BuildProfile {
        if self.release { BuildProfile::Release } else { BuildProfile::Debug }
    }
}

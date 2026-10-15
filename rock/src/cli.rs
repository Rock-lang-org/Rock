use std::{path::PathBuf, time::Instant};

use clap::{Args, Parser, Subcommand};

use crate::{
    artifact::{ensure_artifact, ArtifactBuildState},
    build::{build_project, run_project},
    commands::{clean_project, expand_project, format_project, new_project},
    package::{BuildProfile, Package},
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
            let executable = build_project(&current_project_root()?, profile.selected())?;
            println!("{}", executable.display());
            Ok(CliOutcome::Success)
        }
        Command::Run { profile, args } => run_project(&current_project_root()?, profile.selected(), &args).map(CliOutcome::Exit),
        Command::Expand => expand_project().map(|_| CliOutcome::Success),
        Command::Artifact { profile } => {
            let started = Instant::now();
            let mut package = Package::load(current_project_root()?)?;
            package.profile = profile.selected();
            let mut state = ArtifactBuildState::for_package(&package)?;
            let artifact = ensure_artifact(&package.root_dir, &mut state)?;
            progress::finished(profile.selected(), started.elapsed(), &state.progress);
            println!("{}", artifact.display());
            Ok(CliOutcome::Success)
        }
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
    Build {
        #[command(flatten)]
        profile: ProfileArgs,
    },
    Run {
        #[command(flatten)]
        profile: ProfileArgs,
        #[arg(last = true)]
        args: Vec<String>,
    },
    Expand,
    Artifact {
        #[command(flatten)]
        profile: ProfileArgs,
    },
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

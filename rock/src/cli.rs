use std::path::PathBuf;

use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand};

use crate::{
    build::{build_project, run_project},
    commands::{clean_project, expand_project, format_project, new_project},
    package::BuildProfile,
    progress,
};

pub fn run() -> Result<CliOutcome, String> {
    let matches = Config::command().get_matches();
    let config = Config::from_arg_matches(&matches).unwrap_or_else(|error| error.exit());
    let lint_args = matches
        .subcommand()
        .filter(|(name, _)| matches!(*name, "build" | "run"))
        .map(|(_, sub)| ordered_lint_args(sub))
        .unwrap_or_default();
    progress::set_capture_tool_output(config.quiet);

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
        Command::Build { profile, .. } => {
            let products = build_project(&current_project_root()?, profile.selected(), lint_args)?;
            for path in products.paths() {
                println!("{}", path.display());
            }
            Ok(CliOutcome::Success)
        }
        Command::Run {
            profile, args, bin, ..
        } => run_project(
            &current_project_root()?,
            profile.selected(),
            lint_args,
            &args,
            bin.as_deref(),
        )
        .map(CliOutcome::Exit),
        Command::Expand => expand_project().map(|_| CliOutcome::Success),
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
    /// Hide the output of the compiler tools rock runs; a failing build step
    /// still prints it
    #[arg(long, short, global = true)]
    pub(crate) quiet: bool,
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
        #[command(flatten)]
        lints: LintArgs,
    },
    /// Build the crate and execute it if it is a binary
    Run {
        #[command(flatten)]
        profile: ProfileArgs,
        #[command(flatten)]
        lints: LintArgs,
        /// Select a binary target by name
        #[arg(long)]
        bin: Option<String>,
        #[arg(last = true)]
        args: Vec<String>,
    },
    Expand,
}

/// Lint level overrides for the project's own crate, forwarded to rockc.
#[derive(Args, Debug)]
pub(crate) struct LintArgs {
    /// Allow a lint or lint group, silencing its warnings
    ///
    /// Lints: `unused_imports`, `unused_variables`, `dead_code`; the `unused`
    /// group names all three. Flags apply in command-line order, later ones win.
    #[arg(short = 'A', long = "allow", value_name = "LINT")]
    allow: Vec<String>,
    /// Warn on a lint or lint group (the default for every lint)
    ///
    /// Lints: `unused_imports`, `unused_variables`, `dead_code`; the `unused`
    /// group names all three. Flags apply in command-line order, later ones win.
    #[arg(short = 'W', long = "warn", value_name = "LINT")]
    warn: Vec<String>,
    /// Deny a lint or lint group, turning its warnings into errors
    ///
    /// Lints: `unused_imports`, `unused_variables`, `dead_code`; the `unused`
    /// group names all three. Flags apply in command-line order, later ones win.
    #[arg(short = 'D', long = "deny", value_name = "LINT")]
    deny: Vec<String>,
}

/// `-A/-W/-D` occurrences as rockc arguments, in command-line order.
pub(crate) fn ordered_lint_args(matches: &clap::ArgMatches) -> Vec<String> {
    let mut ordered = Vec::new();
    for (id, flag) in [("allow", "-A"), ("warn", "-W"), ("deny", "-D")] {
        let (Some(indices), Some(values)) =
            (matches.indices_of(id), matches.get_many::<String>(id))
        else {
            continue;
        };
        ordered.extend(
            indices
                .zip(values)
                .map(|(index, name)| (index, flag, name.clone())),
        );
    }
    ordered.sort_by_key(|(index, _, _)| *index);
    ordered
        .into_iter()
        .flat_map(|(_, flag, name)| [flag.to_string(), name])
        .collect()
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
        if self.release {
            BuildProfile::Release
        } else {
            BuildProfile::Debug
        }
    }
}

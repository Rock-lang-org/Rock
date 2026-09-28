use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::{
    dev::package_dev_stdlib,
    fsutil::exit_code,
    home::{home_dir, RockupHome},
    shell::{
        ensure_shell_setup, ensure_shims, print_current_shell_activation_hint, render_env_script,
    },
    target::add_target_component,
    toolchain::{
        install_toolchain, list_toolchains, proxy_toolchain_command, remove_toolchain,
        run_toolchain_command, set_default_toolchain,
    },
};

pub(crate) fn run() -> Result<Exit, String> {
    let config = Config::parse();
    let home = RockupHome::resolve()?;

    match config.command {
        CommandConfig::Install { name, path } => {
            let installed_path = match path {
                Some(path) => {
                    let installed = install_toolchain(&home, &name, &path)?;
                    ensure_shell_setup(&home)?;
                    installed
                }
                None => crate::release::install_release(&home, &name, false)?,
            };
            print_current_shell_activation_hint()?;
            println!("{}", installed_path.display());
            Ok(Exit::Success)
        }
        CommandConfig::Remove { name } => {
            let removed_path = remove_toolchain(&home, &name)?;
            println!("{}", removed_path.display());
            Ok(Exit::Success)
        }
        CommandConfig::List => {
            let installed = list_toolchains(&home)?;
            println!("Installed toolchains (* active):");
            if installed.is_empty() {
                println!("  (none)");
            }
            for toolchain in &installed {
                let prefix = if toolchain.is_active { "*" } else { " " };
                println!("{} {}", prefix, toolchain.name);
            }
            println!("\nAvailable versions:");
            match crate::release::available_releases() {
                Ok(releases) => {
                    if releases.is_empty() {
                        println!("  (none)");
                    }
                    for release in releases {
                        let prerelease = if release.prerelease {
                            " (prerelease)"
                        } else {
                            ""
                        };
                        let installed = if installed.iter().any(|entry| entry.name == release.tag) {
                            " (installed)"
                        } else {
                            ""
                        };
                        println!("  {}{}{}", release.tag, prerelease, installed);
                    }
                    println!(
                        "\nInstall with `rockup install <version>` or `rockup install stable`."
                    );
                }
                Err(error) => {
                    println!("  (unavailable)");
                    eprintln!("Warning: could not list available versions: {}", error);
                }
            }
            Ok(Exit::Success)
        }
        CommandConfig::Target { command } => match command {
            TargetCommand::Add {
                triple,
                path,
                toolchain,
            } => {
                let installed_path =
                    add_target_component(&home, &triple, toolchain.as_deref(), &path)?;
                println!("{}", installed_path.display());
                Ok(Exit::Success)
            }
        },
        CommandConfig::Dev { command } => match command {
            DevCommand::Stdlib { command } => match command {
                DevStdlibCommand::Package {
                    path,
                    sysroot,
                    target,
                    copy_source,
                    rockc,
                } => {
                    let packaged =
                        package_dev_stdlib(&path, &sysroot, target, copy_source, rockc.as_deref())?;
                    println!("{}", packaged.display());
                    Ok(Exit::Success)
                }
            },
        },
        CommandConfig::Default { name } => {
            let name = crate::release::ensure_default_installed(&home, &name)?;
            set_default_toolchain(&home, &name)?;
            ensure_shims(&home)?;
            ensure_shell_setup(&home)?;
            print_current_shell_activation_hint()?;
            Ok(Exit::Success)
        }
        CommandConfig::Update { name } => {
            let path = crate::release::install_release(&home, &name, true)?;
            print_current_shell_activation_hint()?;
            println!("{}", path.display());
            Ok(Exit::Success)
        }
        CommandConfig::SelfCommand {
            command: SelfCommand::Install,
        } => {
            ensure_shims(&home)?;
            ensure_shell_setup(&home)?;
            print_current_shell_activation_hint()?;
            Ok(Exit::Success)
        }
        CommandConfig::SelfCommand {
            command: SelfCommand::Update,
        } => {
            crate::release::self_update()?;
            Ok(Exit::Success)
        }
        CommandConfig::Env => {
            print!("{}", render_env_script(&home, &home_dir()?));
            Ok(Exit::Success)
        }
        CommandConfig::Run { name, command } => {
            let status = run_toolchain_command(&home, &name, &command)?;
            Ok(Exit::Code(exit_code(status)))
        }
        CommandConfig::Proxy { binary, args } => {
            let status = proxy_toolchain_command(&home, &binary, &args)?;
            Ok(Exit::Code(exit_code(status)))
        }
    }
}

pub(crate) enum Exit {
    Success,
    Code(i32),
}

#[derive(Parser, Debug)]
#[command(version, about = "Rock toolchain manager", long_about = None)]
pub(crate) struct Config {
    #[command(subcommand)]
    pub(crate) command: CommandConfig,
}

#[derive(Subcommand, Debug)]
pub(crate) enum CommandConfig {
    /// Install the latest stable release, a specific version, or a local toolchain.
    Install {
        #[arg(default_value = "stable", help = "stable, vVERSION, or bare VERSION")]
        name: String,
        #[arg(long, help = "Use a local toolchain root or Cargo build directory")]
        path: Option<PathBuf>,
    },
    /// Remove an installed toolchain.
    Remove {
        name: String,
    },
    /// List installed toolchains (* active) and versions available to install.
    List,
    Target {
        #[command(subcommand)]
        command: TargetCommand,
    },
    Dev {
        #[command(subcommand)]
        command: DevCommand,
    },
    /// Select a toolchain, installing a missing stable or versioned release.
    Default {
        name: String,
    },
    /// Install or replace a GitHub release toolchain.
    Update {
        #[arg(default_value = "stable", help = "stable, vVERSION, or bare VERSION")]
        name: String,
    },
    #[command(name = "self")]
    SelfCommand {
        #[command(subcommand)]
        command: SelfCommand,
    },
    Env,
    Run {
        name: String,
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    #[command(hide = true, disable_help_flag = true)]
    Proxy {
        binary: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum SelfCommand {
    /// Install this manager and shell setup without downloading a toolchain.
    Install,
    /// Replace this executable with rockup from the latest stable GitHub release.
    Update,
}

#[derive(Subcommand, Debug)]
pub(crate) enum TargetCommand {
    Add {
        triple: String,
        #[arg(long)]
        path: PathBuf,
        #[arg(long)]
        toolchain: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum DevCommand {
    Stdlib {
        #[command(subcommand)]
        command: DevStdlibCommand,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum DevStdlibCommand {
    Package {
        #[arg(long)]
        path: PathBuf,
        #[arg(long)]
        sysroot: PathBuf,
        #[arg(long)]
        target: Option<String>,
        #[arg(long)]
        copy_source: bool,
        #[arg(long)]
        rockc: Option<PathBuf>,
    },
}

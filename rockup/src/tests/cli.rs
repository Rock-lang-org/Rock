use std::path::PathBuf;

use clap::Parser;

use crate::cli::{CommandConfig, Config, DevCommand, DevStdCommand, TargetCommand};

#[test]
fn test_proxy_forwards_help_to_toolchain_binary() {
    for flag in ["--help", "-h"] {
        let config = Config::try_parse_from(["rockup", "proxy", "rock-lsp", flag]).unwrap();
        match config.command {
            CommandConfig::Proxy { binary, args } => {
                assert_eq!(binary, "rock-lsp");
                assert_eq!(args, [flag]);
            }
            other => panic!("unexpected command: {:?}", other),
        }
    }
}

#[test]
fn test_install_and_update_default_to_stable() {
    for args in [vec!["rockup", "install"], vec!["rockup", "update"]] {
        match Config::try_parse_from(args).unwrap().command {
            CommandConfig::Install { name, path } => {
                assert_eq!(name, "stable");
                assert_eq!(path, None);
            }
            CommandConfig::Update { name } => assert_eq!(name, "stable"),
            other => panic!("unexpected command: {:?}", other),
        }
    }
}

#[test]
fn test_install_accepts_versions_and_local_paths() {
    for name in ["stable", "v1.2.3", "1.2.3"] {
        match Config::try_parse_from(["rockup", "install", name])
            .unwrap()
            .command
        {
            CommandConfig::Install { name: parsed, path } => {
                assert_eq!(parsed, name);
                assert_eq!(path, None);
            }
            other => panic!("unexpected command: {:?}", other),
        }
    }
    match Config::try_parse_from(["rockup", "install", "dev", "--path", "/tmp/toolchain"])
        .unwrap()
        .command
    {
        CommandConfig::Install { name, path } => {
            assert_eq!(name, "dev");
            assert_eq!(path, Some(PathBuf::from("/tmp/toolchain")));
        }
        other => panic!("unexpected command: {:?}", other),
    }
}

#[test]
fn test_cli_parses_list_command() {
    let config = Config::try_parse_from(["rockup", "list"]).unwrap();
    assert!(matches!(config.command, CommandConfig::List));
}

#[test]
fn test_cli_parses_remove_command() {
    let config = Config::try_parse_from(["rockup", "remove", "stable"]).unwrap();

    match config.command {
        CommandConfig::Remove { name } => {
            assert_eq!(name, "stable");
        }
        other => panic!("unexpected command: {:?}", other),
    }
}

#[test]
fn test_cli_parses_target_add_command() {
    let config = Config::try_parse_from([
        "rockup",
        "target",
        "add",
        "wasm32-unknown-unknown",
        "--path",
        "/tmp/component",
        "--toolchain",
        "stable",
    ])
    .unwrap();

    match config.command {
        CommandConfig::Target {
            command:
                TargetCommand::Add {
                    triple,
                    path,
                    toolchain,
                },
        } => {
            assert_eq!(triple, "wasm32-unknown-unknown");
            assert_eq!(path, PathBuf::from("/tmp/component"));
            assert_eq!(toolchain.as_deref(), Some("stable"));
        }
        other => panic!("unexpected command: {:?}", other),
    }
}

#[test]
fn test_cli_parses_env_command() {
    let config = Config::try_parse_from(["rockup", "env"]).unwrap();

    match config.command {
        CommandConfig::Env => {}
        other => panic!("unexpected command: {:?}", other),
    }
}

#[test]
fn test_cli_parses_dev_std_package_command() {
    let config = Config::try_parse_from([
        "rockup",
        "dev",
        "std",
        "package",
        "--path",
        "/tmp/std",
        "--sysroot",
        "/tmp/dev-sysroot",
        "--target",
        "wasm32-unknown-unknown",
        "--copy-source",
        "--rockc",
        "/tmp/rockc",
    ])
    .unwrap();

    match config.command {
        CommandConfig::Dev {
            command:
                DevCommand::Std {
                    command:
                        DevStdCommand::Package {
                            path,
                            sysroot,
                            target,
                            copy_source,
                            rockc,
                        },
                },
        } => {
            assert_eq!(path, PathBuf::from("/tmp/std"));
            assert_eq!(sysroot, PathBuf::from("/tmp/dev-sysroot"));
            assert_eq!(target.as_deref(), Some("wasm32-unknown-unknown"));
            assert!(copy_source);
            assert_eq!(rockc.as_deref(), Some(std::path::Path::new("/tmp/rockc")));
        }
        other => panic!("unexpected command: {:?}", other),
    }
}

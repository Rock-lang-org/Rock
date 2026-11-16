use rock_lib::lint::{Lint, LintLevel};
use rock_lib::CrateKind;

use super::Config;

fn compiler_config(args: &[&str]) -> Result<rock_lib::Config, rock_lib::diagnostic::Diagnostics> {
    let mut argv = vec!["rockc", "--entry-file", "main.rk"];
    argv.extend_from_slice(args);
    Config::parse_from_args(argv)
        .unwrap()
        .into_compiler_config()
}

#[test]
fn lint_flags_apply_in_command_line_order() {
    let config = compiler_config(&["-D", "unused", "-A", "dead_code", "--warn", "unused_imports"])
        .unwrap();
    assert_eq!(config.lint_levels.level(Lint::DeadCode), LintLevel::Allow);
    assert_eq!(config.lint_levels.level(Lint::UnusedImports), LintLevel::Warn);
    assert_eq!(config.lint_levels.level(Lint::UnusedVariables), LintLevel::Deny);

    let config = compiler_config(&["-A", "dead_code", "-D", "unused"]).unwrap();
    assert_eq!(config.lint_levels.level(Lint::DeadCode), LintLevel::Deny);
}

#[test]
fn unknown_lint_flag_is_rejected() {
    let error = compiler_config(&["-A", "not_a_lint"]).unwrap_err();
    assert!(error.contains("unknown lint: `not_a_lint`"), "{error}");
}

#[test]
fn crate_type_defaults_to_bin_and_accepts_lib() {
    assert_eq!(compiler_config(&[]).unwrap().crate_kind, CrateKind::Bin);
    assert_eq!(
        compiler_config(&["--crate-type", "lib"]).unwrap().crate_kind,
        CrateKind::Lib
    );
}

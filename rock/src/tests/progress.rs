use std::{fs, path::Path, process::Command};

use crate::rockc::{ensure_dev_binary_built, resolve_rockc_path};

use super::support::{run_command_output, temp_test_dir, write_package_with_options};

fn run_rock(root: &Path, command: &str, profile: &str) -> std::process::Output {
    ensure_dev_binary_built("rock").unwrap();
    let rock = rock_shared::process::dev_target_binary_from_current_exe("rock").unwrap();
    run_command_output(
        Command::new(rock)
            .current_dir(root)
            .args([command, profile])
            .env("ROCKC", resolve_rockc_path().unwrap())
            .env("NO_COLOR", "1"),
    )
}

fn compiling_crates(stderr: &str) -> Vec<&str> {
    stderr.lines().filter_map(|line| {
        line.trim_start().strip_prefix("◆ compile ")?.split_whitespace().next()
    }).collect()
}

fn write_progress_tree(root: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let leaf = root.join("leaf");
    let branch = root.join("branch");
    let middle = root.join("middle");
    let app = root.join("app");
    write_package_with_options(&leaf, "leaf", "lib.rk", &[], "< answer = -> 42\n", true);
    write_package_with_options(
        &branch, "branch", "lib.rk", &[("leaf", leaf.display().to_string())],
        "> leaf::answer\n< relay = -> answer!\n", true,
    );
    write_package_with_options(
        &middle, "middle", "lib.rk", &[("branch", branch.display().to_string())],
        "> branch::relay\n< forward = -> relay!\n", true,
    );
    write_package_with_options(
        &app, "app", "src/main.rk",
        &[("leaf", leaf.display().to_string()), ("middle", middle.display().to_string())],
        "> middle::forward\nmain = -> forward!\n", true,
    );
    (app, leaf)
}

#[test]
fn test_cli_progress_reports_rebuilds_in_dependency_order_on_stderr() {
    let root = temp_test_dir("cli_build_progress");
    let (app, leaf) = write_progress_tree(&root);

    for profile in ["--debug", "--release"] {
        let output = run_rock(&app, "build", profile);
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(output.status.success(), "{stderr}");
        assert_eq!(compiling_crates(&stderr), ["leaf", "branch", "middle", "app"]);
        assert!(stderr.contains("◆ compile leaf v0.1.0 · "), "{stderr}");
        let directory = profile.trim_start_matches("--");
        assert!(stderr.contains(&format!("✓ ready   {directory} · ")), "{stderr}");
        assert!(stderr.contains("4/4 crates · 100%"), "{stderr}");
        assert!(stderr.lines().any(|line| line.contains("✓ ready") && line.ends_with('s')), "{stderr}");
        assert!(!stderr.contains('\x1b'));
        assert_eq!(String::from_utf8(output.stdout).unwrap(), format!("{}\n", app.join("build").join(directory).join("app").display()));

        let cached = run_rock(&app, "build", profile);
        let stderr = String::from_utf8(cached.stderr).unwrap();
        assert!(cached.status.success(), "{stderr}");
        assert_eq!(compiling_crates(&stderr), ["app"]);
        assert!(stderr.contains("4/4 crates · 100%"), "{stderr}");
    }

    fs::write(leaf.join("lib.rk"), "< answer = -> 43\n").unwrap();
    let rebuilt = run_rock(&app, "build", "--debug");
    let stderr = String::from_utf8(rebuilt.stderr).unwrap();
    assert!(rebuilt.status.success(), "{stderr}");
    assert_eq!(compiling_crates(&stderr), ["leaf", "branch", "middle", "app"]);

    fs::write(leaf.join("lib.rk"), "< answer = -> missing_name\n").unwrap();
    let failed = run_rock(&app, "build", "--debug");
    let stderr = String::from_utf8(failed.stderr).unwrap();
    assert!(!failed.status.success());
    assert_eq!(compiling_crates(&stderr), ["leaf"]);
    assert!(!stderr.contains("✓ ready"), "{stderr}");
    assert!(!stderr.contains("Failed to spawn rockc"), "{stderr}");
    let stdout = String::from_utf8(failed.stdout).unwrap();
    assert!(!stdout.contains("◆ compile"));
    assert!(!stdout.contains("✓ ready"));

    let _ = fs::remove_dir_all(root);
}

#[cfg(target_os = "linux")]
#[test]
fn test_overall_terminal_progress_counts_shared_crates_cache_hits_and_failures() {
    let root = temp_test_dir("cli_overall_progress");
    let (app, _) = write_progress_tree(&root);
    ensure_dev_binary_built("rock").unwrap();
    let rock = rock_shared::process::dev_target_binary_from_current_exe("rock").unwrap();
    let rockc = resolve_rockc_path().unwrap();
    let run = || {
        run_command_output(
            Command::new("script")
                .current_dir(&app)
                .args(["--quiet", "--return", "--command", "\"$ROCK_TEST_BINARY\" build > /dev/null", "/dev/null"])
                .env("ROCK_TEST_BINARY", &rock)
                .env("ROCKC", &rockc)
                .env("NO_COLOR", "1")
                .env("TERM", "xterm"),
        )
    };
    let cold = run();
    let terminal = String::from_utf8(cold.stdout).unwrap();
    assert!(cold.status.success(), "{terminal}");
    for expected in ["0/4 · 0% · leaf", "1/4 · 25% · branch", "2/4 · 50% · middle", "3/4 · 75% · app"] {
        assert!(terminal.contains(expected), "{terminal}");
    }
    assert!(terminal.contains("4/4 crates · 100%"), "{terminal}");

    let warm = run();
    let terminal = String::from_utf8(warm.stdout).unwrap();
    assert!(warm.status.success(), "{terminal}");
    assert!(terminal.contains("3/4 · 75% · app"), "{terminal}");
    assert!(terminal.contains("4/4 crates · 100%"), "{terminal}");
    assert!(!terminal.contains("0/4 ·"), "{terminal}");
    assert!(!terminal.contains("◆ compile leaf"), "{terminal}");

    fs::write(root.join("middle/lib.rk"), "< forward = -> missing_progress_probe\n").unwrap();
    let failed = run();
    let terminal = String::from_utf8(failed.stdout).unwrap();
    assert!(!failed.status.success(), "{terminal}");
    assert!(terminal.contains("2/4 · 50% · middle"), "{terminal}");
    assert!(!terminal.contains("3/4 ·"), "{terminal}");
    assert!(!terminal.contains("100%"), "{terminal}");
    assert!(!terminal.contains("✓ ready"), "{terminal}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn test_cli_progress_for_library_cache_and_run_keeps_stdout_clean() {
    let root = temp_test_dir("cli_artifact_run_progress");
    write_package_with_options(&root, "app", "src/lib.rk", &[], "< answer = -> 42\n", true);

    for expected in [vec!["app"], vec![]] {
        let output = run_rock(&root, "build", "--debug");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(output.status.success(), "{stderr}");
        assert_eq!(compiling_crates(&stderr), expected);
        assert!(stderr.contains("✓ ready   debug · unoptimized · "));
        assert!(stderr.contains("1/1 crates · 100%"), "{stderr}");
        assert_eq!(String::from_utf8(output.stdout).unwrap(), format!("{}\n", root.join("build/debug/artifacts/app-0.1.0.rkca").display()));
    }

    let output = run_rock(&root, "run", "--debug");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(output.status.success(), "{stderr}");
    assert!(compiling_crates(&stderr).is_empty());
    assert!(stderr.contains("✓ ready   debug · unoptimized · "));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "Crate 'app' is a library; there is no binary to execute.\n");

    let binary = root.join("binary");
    write_package_with_options(&binary, "app", "src/main.rk", &[], "main = !-> ()\n", true);
    let output = run_rock(&binary, "run", "--debug");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(output.status.success(), "{stderr}");
    assert_eq!(compiling_crates(&stderr), ["app"]);
    assert!(stderr.contains("✓ ready   debug · unoptimized · "));
    assert!(output.stdout.is_empty());

    let _ = fs::remove_dir_all(root);
}

#[cfg(target_os = "linux")]
#[test]
fn test_cli_live_progress_in_a_terminal_preserves_output_and_clears_on_failure() {
    use std::os::unix::fs::PermissionsExt;

    let root = temp_test_dir("cli_live_progress");
    write_package_with_options(&root, "app", "src/main.rk", &[], "main = !-> ()\n", true);
    let compiler = root.join("mock-rockc");
    fs::write(&compiler, concat!(
        "#!/bin/sh\n",
        "while [ $# -gt 0 ]; do if [ \"$1\" = --output-dir ]; then shift; output=$1; fi; shift; done\n",
        ": > \"$output/main\"\n",
        "printf 'compiler-stdout\\n'\n",
        "sleep 0.1\n",
        "printf 'warning: ' >&2\n",
        "sleep 0.25\n",
        "printf 'visible-warning\\n' >&2\n",
        "sleep 0.25\n",
        "exit \"$ROCK_TEST_EXIT\"\n",
    )).unwrap();
    fs::set_permissions(&compiler, fs::Permissions::from_mode(0o755)).unwrap();
    ensure_dev_binary_built("rock").unwrap();
    let rock = rock_shared::process::dev_target_binary_from_current_exe("rock").unwrap();
    let stdout_path = root.join("stdout.txt");

    // util-linux script supplies a real PTY for stderr, while stdout stays a file.
    for (exit, no_color, term) in [(0, false, "xterm"), (3, false, "xterm"), (0, true, "xterm"), (0, false, "dumb")] {
        let mut command = Command::new("script");
        command.current_dir(&root)
            .args(["--quiet", "--return", "--command", "\"$ROCK_TEST_BINARY\" build > \"$ROCK_TEST_STDOUT\"", "/dev/null"])
            .env("ROCK_TEST_BINARY", &rock)
            .env("ROCK_TEST_STDOUT", &stdout_path)
            .env("ROCK_TEST_EXIT", exit.to_string())
            .env("ROCKC", &compiler)
            .env("TERM", term)
            .env_remove("NO_COLOR");
        if no_color {
            command.env("NO_COLOR", "1");
        }
        let output = run_command_output(&mut command);
        let terminal = String::from_utf8(output.stdout).unwrap();
        assert_eq!(output.status.success(), exit == 0, "{terminal}");
        assert!(terminal.contains("warning: visible-warning"), "{terminal}");
        if term == "dumb" {
            assert!(!terminal.contains('\x1b'), "{terminal}");
        } else {
            assert!(terminal.matches("\r\x1b[2K\x1b[?7l").count() >= 3, "{terminal}");
            assert!(terminal.contains("[◆─────────]"), "{terminal}");
            assert!(terminal.contains("◇ [◆─────────]"), "{terminal}");
            assert!(terminal.contains("0/1 · 0% · "), "{terminal}");
            if no_color {
                assert!(!terminal.contains("38;5;"), "{terminal}");
            } else {
                assert!(terminal.contains("38;5;80m"), "{terminal}");
                assert!(terminal.contains("38;5;147m"), "{terminal}");
            }
        }
        let stdout = fs::read_to_string(&stdout_path).unwrap();
        assert!(stdout.starts_with("compiler-stdout\n"), "{stdout}");
        assert!(!stdout.contains('\x1b'));
        if exit == 0 {
            assert!(terminal.contains("✓ ready"), "{terminal}");
            assert!(terminal.contains("1/1 crates · 100%"), "{terminal}");
            assert!(stdout.ends_with(&format!("{}\n", root.join("build/debug/app").display())));
        } else {
            assert!(!terminal.contains("✓ ready"), "{terminal}");
            assert!(terminal.contains("\r\x1b[2KError: rockc failed"), "{terminal}");
            assert_eq!(stdout, "compiler-stdout\n");
        }
    }
    let _ = fs::remove_dir_all(root);
}

#[cfg(target_os = "linux")]
#[test]
fn test_cli_live_progress_preserves_real_compiler_diagnostic_colors() {
    let root = temp_test_dir("cli_diagnostic_colors");
    write_package_with_options(
        &root, "app", "src/main.rk", &[], "main = -> missing_color_probe\n", true,
    );
    ensure_dev_binary_built("rock").unwrap();
    let rock = rock_shared::process::dev_target_binary_from_current_exe("rock").unwrap();
    let rockc = resolve_rockc_path().unwrap();
    let stdout_path = root.join("stdout.txt");

    for (no_color, clicolor, redirected, term) in [
        (false, "1", false, "xterm"),
        (true, "1", false, "xterm"),
        (false, "0", false, "xterm"),
        (false, "1", true, "xterm"),
        (false, "1", false, "dumb"),
    ] {
        let shell_command = if redirected {
            "\"$ROCK_TEST_BINARY\" build > \"$ROCK_TEST_STDOUT\""
        } else {
            "\"$ROCK_TEST_BINARY\" build"
        };
        let mut command = Command::new("script");
        command.current_dir(&root)
            .args(["--quiet", "--return", "--command", shell_command, "/dev/null"])
            .env("ROCK_TEST_BINARY", &rock)
            .env("ROCK_TEST_STDOUT", &stdout_path)
            .env("ROCKC", &rockc)
            .env("TERM", term)
            .env("CLICOLOR", clicolor)
            .env_remove("CLICOLOR_FORCE")
            .env_remove("NO_COLOR");
        if no_color {
            command.env("NO_COLOR", "1");
        }
        let output = run_command_output(&mut command);
        let terminal = String::from_utf8(output.stdout).unwrap();
        assert!(!output.status.success(), "{terminal}");
        let diagnostic = if redirected { fs::read_to_string(&stdout_path).unwrap() } else { terminal };
        assert!(diagnostic.contains("missing_color_probe"), "{diagnostic}");
        // Red belongs to the compiler's diagnostic, not the teal/lavender progress UI.
        let colored = diagnostic.contains("\x1b[31m") || diagnostic.contains("\x1b[38;5;9m");
        assert_eq!(colored, !no_color && clicolor != "0" && !redirected && term != "dumb", "{diagnostic}");
        if redirected {
            assert!(!diagnostic.contains('\x1b'), "{diagnostic}");
        }
    }
    let _ = fs::remove_dir_all(root);
}

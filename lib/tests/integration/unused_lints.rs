//! `unused_*` / `dead_code` lints: warnings on success, levels from config.

use std::sync::atomic::{AtomicU64, Ordering};

use rock_lib::diagnostic::{DiagnosticSeverity, Diagnostics};
use rock_lib::lint::LintLevel;

use crate::support::{test_config, test_temp_dir, TestDirCleanup};

static LINT_TEST_COUNTER: AtomicU64 = AtomicU64::new(1_000_000);

/// Compiles `source` (no link) with `levels` overrides and returns the
/// diagnostics: `Ok(warnings)` on success, `Err(errors)` on failure.
fn lint(source: &str, levels: &[(&str, LintLevel)]) -> Result<Diagnostics, Diagnostics> {
    let dir = test_temp_dir(LINT_TEST_COUNTER.fetch_add(1, Ordering::SeqCst));
    std::fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    let source_path = dir.join("test.rk");
    std::fs::write(&source_path, source).unwrap();
    let mut config = test_config(source_path, dir.clone());
    config.no_link = true;
    for (name, level) in levels {
        config.lint_levels.set(name, *level).unwrap();
    }
    rock_lib::compile_with_warnings(&config).map(|output| output.warnings)
}

fn messages(diagnostics: &Diagnostics) -> Vec<String> {
    diagnostics.0.iter().map(|d| d.message.clone()).collect()
}

#[test]
fn test_unused_variables_warn_for_locals_params_closures_and_patterns() {
    let warnings = lint(
        r#"
first = x, y -> x

main = !->
    t = 5
    used = 6
    f = (z -> 1)
    m = match used
        n => 2
    (first used, (f m)).println!
"#,
        &[],
    )
    .unwrap();
    assert_eq!(
        messages(&warnings),
        [
            "unused variable: `y`",
            "unused variable: `t`",
            "unused variable: `z`",
            "unused variable: `n`",
        ]
    );
    assert!(warnings
        .0
        .iter()
        .all(|warning| warning.severity == DiagnosticSeverity::Warning));
}

#[test]
fn test_underscore_prefix_and_reads_silence_unused_variables() {
    let warnings = lint(
        r#"
main = !->
    _skipped = 1
    read = 2
    captured = 3
    f = (-> captured)
    (read + (f!)).println!
"#,
        &[],
    )
    .unwrap();
    assert_eq!(messages(&warnings), Vec::<String>::new());
}

#[test]
fn test_assigned_but_never_read_local_is_unused() {
    let warnings = lint(
        r#"
main = !->
    w = 3
    w = 4
"#,
        &[],
    )
    .unwrap();
    assert_eq!(messages(&warnings), ["unused variable: `w`"]);
}

#[test]
fn test_allowed_lint_is_silent_and_denied_lint_fails_the_build() {
    let source = "main = !->\n    t = 5\n";
    assert!(lint(source, &[("unused", LintLevel::Allow)]).unwrap().0.is_empty());
    let errors = lint(source, &[("unused_variables", LintLevel::Deny)]).unwrap_err();
    assert_eq!(messages(&errors), ["unused variable: `t`"]);
    assert_eq!(errors.0[0].severity, DiagnosticSeverity::Error);
}

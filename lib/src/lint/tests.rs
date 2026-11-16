use super::*;
use crate::lexer::Span;

fn span() -> Span {
    Span {
        file_path: "main.rk".into(),
        start: 0,
        end: 1,
    }
}

#[test]
fn unused_lints_warn_by_default() {
    let levels = LintLevels::default();
    for lint in Lint::ALL {
        assert_eq!(levels.level(lint), LintLevel::Warn);
    }
}

#[test]
fn group_override_applies_to_every_member_and_later_overrides_win() {
    let mut levels = LintLevels::default();
    levels.set("unused", LintLevel::Allow).unwrap();
    levels.set("dead_code", LintLevel::Deny).unwrap();
    assert_eq!(levels.level(Lint::UnusedImports), LintLevel::Allow);
    assert_eq!(levels.level(Lint::UnusedVariables), LintLevel::Allow);
    assert_eq!(levels.level(Lint::DeadCode), LintLevel::Deny);
}

#[test]
fn unknown_lint_name_is_rejected() {
    let mut levels = LintLevels::default();
    assert!(levels.set("unused_everything", LintLevel::Deny).is_err());
}

#[test]
fn sink_drops_allowed_warns_and_denies_by_level() {
    let mut levels = LintLevels::default();
    levels.set("unused_imports", LintLevel::Allow).unwrap();
    levels.set("dead_code", LintLevel::Deny).unwrap();
    let mut sink = LintSink::new(&levels);
    let finding = || Diagnostic::new("unused".to_string(), span());
    sink.emit(Lint::UnusedImports, finding());
    sink.emit(Lint::UnusedVariables, finding());
    sink.emit(Lint::DeadCode, finding());
    let diagnostics = sink.finish();
    let severities = diagnostics
        .0
        .iter()
        .map(|diagnostic| diagnostic.severity)
        .collect::<Vec<_>>();
    assert_eq!(
        severities,
        vec![DiagnosticSeverity::Warning, DiagnosticSeverity::Error]
    );
    assert!(diagnostics
        .0
        .iter()
        .all(|diagnostic| diagnostic.code == Some(DiagnosticCode::Lint)));
}

#[test]
fn underscore_prefix_opts_out() {
    assert!(is_intentionally_unused("_x"));
    assert!(!is_intentionally_unused("x"));
}

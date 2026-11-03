use super::*;

use std::path::PathBuf;

use crate::lexer::Span;

#[test]
fn diagnostics_require_explicit_spans_and_deduplicate_messages() {
    let mut diagnostics = LowerDiagnosticSink::new();
    let span = Span {
        file_path: "main.rk".into(),
        start: 3,
        end: 9,
    };

    diagnostics.push("same message".to_string(), span.clone());
    diagnostics.push_once("same message".to_string(), span.clone());
    diagnostics.push_once("other message".to_string(), span);

    assert_eq!(diagnostics.errors().len(), 2);
    assert_eq!(diagnostics.errors()[0].message, "same message");
    assert_eq!(diagnostics.errors()[0].span().unwrap().start, 3);
    assert_eq!(diagnostics.errors()[1].message, "other message");
}

#[test]
fn diagnostics_push_with_explicit_span_preserves_operation_span() {
    let mut diagnostics = LowerDiagnosticSink::new();
    diagnostics.push_with_span(
        "explicit".to_string(),
        Span {
            file_path: "child.rk".into(),
            start: 10,
            end: 14,
        },
    );

    assert_eq!(
        diagnostics.errors()[0].span().unwrap().file_path,
        PathBuf::from("child.rk")
    );
    assert_eq!(diagnostics.errors()[0].span().unwrap().start, 10);
}

#[test]
fn diagnostic_sink_finishes_to_result_without_losing_span_or_deduplication() {
    let mut sink = LowerDiagnosticSink::new();
    let span = Span {
        file_path: "main.rk".into(),
        start: 3,
        end: 9,
    };

    sink.push("same message".to_string(), span.clone());
    sink.push_once("same message".to_string(), span.clone());
    sink.push_once("other message".to_string(), span);

    let diagnostics = sink.finish();

    assert_eq!(diagnostics.errors().len(), 2);
    assert_eq!(diagnostics.errors()[0].message, "same message");
    assert_eq!(diagnostics.errors()[0].span().unwrap().start, 3);
    assert_eq!(diagnostics.errors()[1].message, "other message");
}

#[test]
fn diagnostic_sink_assigns_stable_type_and_selection_codes() {
    let mut diagnostics = LowerDiagnosticSink::new();
    let span = Span {
        file_path: "main.rk".into(),
        start: 1,
        end: 2,
    };

    diagnostics.push_type_with_span("type mismatch".to_string(), span.clone());
    diagnostics.push_selection_with_span(
        "Call to unsafe function 'danger' requires an unsafe block".to_string(),
        span,
    );

    assert_eq!(diagnostics.errors()[0].code, DiagnosticCode::Type);
    assert_eq!(diagnostics.errors()[1].code, DiagnosticCode::Selection);
    assert!(diagnostics.errors()[1].message.contains("unsafe function"));
}

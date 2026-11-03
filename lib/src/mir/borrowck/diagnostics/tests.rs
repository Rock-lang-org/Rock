use super::borrow_conflict;
use crate::diagnostic::{DiagnosticCode, DiagnosticLocation};
use crate::lexer::Span;
use crate::mir::{Local, Place};

#[test]
fn borrow_conflict_uses_use_and_loan_origins_as_labels() {
    let use_span = Span::new("/virtual/main.rk".into(), 4, 7);
    let loan_span = Span::new("/virtual/main.rk".into(), 12, 15);
    let diagnostic = borrow_conflict(
        &Place {
            local: Local(1),
            projection: vec![],
        },
        Some(use_span.clone()),
        Some(loan_span.clone()),
    );

    assert_eq!(diagnostic.code, Some(DiagnosticCode::Borrow));
    assert_eq!(diagnostic.location, DiagnosticLocation::Source(use_span));
    assert_eq!(diagnostic.primary.as_ref().unwrap().span.start, 4);
    assert_eq!(diagnostic.secondary.len(), 2);
    assert_eq!(diagnostic.secondary[1].span.start, loan_span.start);
}

#[test]
fn borrow_conflict_without_origin_is_internal() {
    let diagnostic = borrow_conflict(
        &Place {
            local: Local(1),
            projection: vec![],
        },
        None,
        None,
    );

    assert_eq!(diagnostic.code, Some(DiagnosticCode::Internal));
    assert_eq!(diagnostic.location, DiagnosticLocation::Toolchain);
    assert!(diagnostic.primary.is_none());
}

use super::*;

#[test]
fn resolve_error_constructors_keep_source_and_non_source_origins_distinct() {
    let span = Span::test();
    let source = ResolveError::with_span("source".to_string(), span.clone());
    let non_source = ResolveError::non_source("toolchain".to_string());

    assert_eq!(source.kind(), ResolveErrorKind::Source);
    assert_eq!(source.code, DiagnosticCode::Resolve);
    assert_eq!(source.span(), Some(span));
    assert_eq!(non_source.kind(), ResolveErrorKind::NonSource);
    assert_eq!(non_source.code, DiagnosticCode::Internal);
    assert_eq!(non_source.span(), None);
}

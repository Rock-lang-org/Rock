use std::path::PathBuf;

#[test]
fn source_load_conversion_attaches_text_only_to_source_diagnostics() {
    let path = PathBuf::from("/virtual/loader.rk");
    let source = crate::source_loader::SourceFile {
        original_path: path.clone(),
        canonical_path: path.clone(),
        display_path: path.clone(),
        text: "source".to_string(),
        origin: crate::source_loader::SourceOrigin::Virtual,
    };
    let diagnostics = super::source_load_errors_to_diagnostics(vec![
        crate::source_loader::SourceLoadError::Parse {
            path,
            source: Some(source),
            error: crate::parser::ParseError::Fail,
        },
    ]);

    assert_eq!(diagnostics.0.len(), 1);
    assert!(matches!(
        &diagnostics.0[0].location,
        crate::diagnostic::DiagnosticLocation::Toolchain
    ));
    assert!(diagnostics.0[0].source.is_none());
}

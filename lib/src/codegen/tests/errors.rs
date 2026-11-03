use super::*;

#[test]
fn codegen_source_operation_keeps_exact_origin_and_code() {
    let span = crate::lexer::Span::new(std::path::PathBuf::from("/virtual/main.rk"), 4, 9);
    let diagnostic =
        CodegenError::with_span("operation failed".to_string(), span.clone()).into_diagnostic();

    assert_eq!(diagnostic.code, Some(DiagnosticCode::Codegen));
    assert_eq!(diagnostic.location, DiagnosticLocation::Source(span));
    assert!(diagnostic.primary.is_some());
}

#[test]
fn codegen_source_operation_hides_debug_identity_payloads() {
    let span = crate::lexer::Span::new(std::path::PathBuf::from("/virtual/main.rk"), 4, 9);
    let diagnostic = CodegenError::with_span(
        "MIR callable DefId { local: 1 } uses GenericParamId { index: 0 }".to_string(),
        span.clone(),
    )
    .into_diagnostic();

    assert_eq!(diagnostic.location, DiagnosticLocation::Source(span));
    assert_eq!(
        diagnostic.message,
        "Internal compiler error during MIR code generation"
    );
    assert!(diagnostic.primary.is_some());
}

#[test]
fn codegen_backend_contract_is_internal_and_non_source() {
    let diagnostic = CodegenError::backend_contract("invalid contract").into_diagnostic();

    assert_eq!(diagnostic.code, Some(DiagnosticCode::Internal));
    assert_eq!(diagnostic.location, DiagnosticLocation::Toolchain);
    assert!(diagnostic.primary.is_none());
}

#[test]
fn codegen_output_failure_is_file_classified() {
    let path = std::path::PathBuf::from("/tmp/main.o");
    let diagnostic = CodegenError::output("cannot write object", path.clone()).into_diagnostic();

    assert_eq!(diagnostic.code, Some(DiagnosticCode::Codegen));
    assert_eq!(diagnostic.location, DiagnosticLocation::File(path));
    assert!(diagnostic.primary.is_none());
}

#[test]
fn codegen_project_and_artifact_failures_keep_public_location_codes() {
    let project_path = std::path::PathBuf::from("/project/rock.toml");
    let artifact_path = std::path::PathBuf::from("/project/dep.rkca");

    let project = CodegenError::project("invalid project", project_path.clone()).into_diagnostic();
    let artifact =
        CodegenError::artifact("invalid artifact", artifact_path.clone()).into_diagnostic();

    assert_eq!(project.code, Some(DiagnosticCode::Project));
    assert_eq!(project.location, DiagnosticLocation::Project(project_path));
    assert_eq!(artifact.code, Some(DiagnosticCode::Artifact));
    assert_eq!(
        artifact.location,
        DiagnosticLocation::Artifact(artifact_path)
    );
}

#[test]
fn llvm_details_are_kept_as_notes_without_a_fabricated_source_span() {
    let diagnostic = CodegenError::new("LLVM rejected the module".to_string()).into_diagnostic();

    assert_eq!(diagnostic.code, Some(DiagnosticCode::Codegen));
    assert_eq!(diagnostic.location, DiagnosticLocation::Toolchain);
    assert!(diagnostic.primary.is_none());
    assert_eq!(diagnostic.notes, vec!["LLVM rejected the module"]);
}

#[test]
fn backend_errors_cannot_be_reclassified_as_source_operations() {
    let span = crate::lexer::Span::new(std::path::PathBuf::from("/virtual/main.rk"), 1, 2);
    let diagnostic = CodegenError::new("backend failed".to_string())
        .with_operation_span(Some(span))
        .into_diagnostic();

    assert_eq!(diagnostic.location, DiagnosticLocation::Toolchain);
    assert!(diagnostic.primary.is_none());
}

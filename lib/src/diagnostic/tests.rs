use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

use crate::{Config, SourceProvider};

#[test]
fn diagnostic_report_handles_more_than_three_labels() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let source_path = std::env::temp_dir().join(format!(
        "rock-diagnostic-many-labels-{}-{nonce}.rk",
        std::process::id()
    ));
    fs::write(&source_path, "main = -> 0\n").unwrap();
    let span = Span {
        file_path: source_path.clone(),
        start: 0,
        end: 4,
    };
    let diagnostic = Diagnostic::new("many labels".to_string(), span.clone()).with_labels(vec![
        ("first".to_string(), span.clone()),
        ("second".to_string(), span.clone()),
        ("third".to_string(), span.clone()),
        ("fourth".to_string(), span.clone()),
    ]);

    let result = std::panic::catch_unwind(|| diagnostic.report());

    let _ = fs::remove_file(source_path);
    assert!(result.is_ok());
}

#[test]
fn file_diagnostic_does_not_fabricate_a_source_range() {
    let path = PathBuf::from("/missing/main.rk");

    let diagnostic = Diagnostic::for_file("missing file".to_string(), path.clone());

    assert_eq!(diagnostic.location, DiagnosticLocation::File(path));
    assert!(diagnostic.primary.is_none());
    assert!(diagnostic.secondary.is_empty());
}

#[test]
#[should_panic(expected = "diagnostic locations require a path")]
fn source_diagnostic_rejects_an_empty_span() {
    let _ = Diagnostic::new(
        "invalid source diagnostic".to_string(),
        Span {
            file_path: PathBuf::new(),
            start: 0,
            end: 0,
        },
    );
}

#[test]
fn diagnostic_source_map_attaches_virtual_source_by_span_path() {
    let path = PathBuf::from("/virtual/main.rk");
    let text = "main = -> missing\n";
    let mut database = crate::source_loader::SourceDatabase::new();
    database.add_source_provider(&SourceProvider::Virtual {
        path: path.clone(),
        text: text.to_string(),
    });
    database
        .load_entry(path.clone(), &Config::default())
        .expect("virtual source should load");
    let sources = DiagnosticSourceMap::from_source_database(&database);
    let mut diagnostic = Diagnostic::new(
        "unknown value".to_string(),
        Span {
            file_path: path.clone(),
            start: 10,
            end: 17,
        },
    );

    sources.attach(&mut diagnostic);

    let source = diagnostic.source.expect("source should be attached");
    assert_eq!(source.display_path, path);
    assert_eq!(source.text, text);
    assert_eq!(source.origin, DiagnosticSourceOrigin::Virtual);
}

#[test]
fn diagnostic_source_map_attaches_sources_for_cross_file_labels() {
    let primary_path = PathBuf::from("/virtual/main.rk");
    let related_path = PathBuf::from("/virtual/dep.rk");
    let mut database = crate::source_loader::SourceDatabase::new();
    for (path, text) in [
        (primary_path.clone(), "main = -> dep\n"),
        (related_path.clone(), "dep = -> 0\n"),
    ] {
        database.add_source_provider(&SourceProvider::Virtual {
            path: path.clone(),
            text: text.to_string(),
        });
        database
            .load_entry(path, &Config::default())
            .expect("virtual source should load");
    }
    let sources = DiagnosticSourceMap::from_source_database(&database);
    let mut diagnostic = Diagnostic::new(
        "cross-file error".to_string(),
        Span {
            file_path: primary_path,
            start: 10,
            end: 13,
        },
    )
    .with_label(
        "defined here".to_string(),
        Span {
            file_path: related_path.clone(),
            start: 0,
            end: 3,
        },
    );

    sources.attach(&mut diagnostic);

    let report_result = std::panic::catch_unwind(|| diagnostic.report());
    let source = diagnostic.source.expect("source should be attached");
    let related = source
        .related
        .get(&related_path)
        .expect("related source should be attached");
    assert_eq!(related.display_path, related_path);
    assert_eq!(related.text, "dep = -> 0\n");
    assert!(report_result.is_ok());
}

#[test]
fn diagnostic_source_map_preserves_artifact_source_origin() {
    let path = PathBuf::from("/artifact/dep/lib.rk");
    let artifact_path = PathBuf::from("/artifact/dep.rkca");
    let text = "answer = -> 42\n";
    let mut database = crate::source_loader::SourceDatabase::new();
    database.add_source_provider(&SourceProvider::Artifact {
        path: path.clone(),
        artifact_path: artifact_path.clone(),
        text: text.to_string(),
    });
    database
        .load_entry(path.clone(), &Config::default())
        .expect("artifact source should load");
    let sources = DiagnosticSourceMap::from_source_database(&database);
    let mut diagnostic = Diagnostic::new(
        "artifact source error".to_string(),
        Span {
            file_path: path,
            start: 0,
            end: 6,
        },
    );

    sources.attach(&mut diagnostic);

    let source = diagnostic.source.expect("source should be attached");
    assert_eq!(source.text, text);
    assert_eq!(
        source.origin,
        DiagnosticSourceOrigin::Artifact { artifact_path }
    );
}

#[test]
fn unexpected_eof_diagnostic_points_to_actual_source_end() {
    let path = PathBuf::from("/virtual/main.rk");
    let text = "lang sized\n";
    let error = crate::parser::parse_source(path.clone(), text, &Config::default())
        .expect_err("incomplete language item should fail");
    let diagnostic = Diagnostic::from(error);

    let DiagnosticLocation::Source(span) = diagnostic.location else {
        panic!("EOF diagnostic should have a source location");
    };
    assert_eq!(span.file_path, path);
    assert_eq!(span.start, text.len());
    assert_eq!(span.end, text.len());
    assert!(diagnostic.primary.is_some());
}

#[test]
fn structured_diagnostic_dto_preserves_contract_fields() {
    let span = Span::new(PathBuf::from("/virtual/main.rk"), 2, 5);
    let diagnostic = Diagnostic::new("bad expression".to_string(), span.clone())
        .with_code(DiagnosticCode::Parser)
        .with_severity(DiagnosticSeverity::Warning)
        .with_label("related expression".to_string(), span.clone())
        .with_note("parser context")
        .with_help("remove the extra token");

    assert_eq!(diagnostic.validate(), Ok(()));
    let dto = diagnostic
        .to_dto()
        .expect("valid diagnostic should convert");
    assert_eq!(dto.code.as_deref(), Some("parser"));
    assert_eq!(dto.severity, DiagnosticSeverity::Warning);
    assert_eq!(dto.primary.as_ref().unwrap().span.start, 2);
    assert_eq!(dto.secondary[0].message, "related expression");
    assert_eq!(dto.notes, vec!["parser context"]);
    assert_eq!(dto.help, vec!["remove the extra token"]);
    assert_eq!(dto.location, DiagnosticLocation::Source(span));
}

#[test]
fn structured_diagnostic_dto_serializes_without_losing_source_contract() {
    let span = Span::new(PathBuf::from("/virtual/main.rk"), 2, 5);
    let dto = Diagnostic::new("bad expression".to_string(), span.clone())
        .with_code(DiagnosticCode::Parser)
        .with_note("parser context")
        .with_help("remove the extra token")
        .to_dto()
        .expect("valid diagnostic should convert");

    let encoded = bincode::serialize(&dto).expect("diagnostic DTO should serialize");
    let decoded: DiagnosticDto =
        bincode::deserialize(&encoded).expect("diagnostic DTO should deserialize");

    assert_eq!(decoded, dto);
    assert_eq!(decoded.location, DiagnosticLocation::Source(span));
}

#[test]
fn parser_control_flow_escape_is_internal_and_non_source() {
    for error in [
        ParseError::Fail,
        ParseError::ShortCircuit,
        ParseError::AssertFailed,
    ] {
        let diagnostic = Diagnostic::from(error);
        assert_eq!(diagnostic.code, Some(DiagnosticCode::Internal));
        assert_eq!(diagnostic.location, DiagnosticLocation::Toolchain);
        assert!(diagnostic.primary.is_none());
    }
}

#[test]
fn parse_context_preserves_wrapped_source_variant_contracts() {
    let path = PathBuf::from("/virtual/parse.rk");
    let macro_span = Span::new(path.clone(), 1, 2);
    let invocation_span = Span::new(path.clone(), 3, 4);
    let argument_span = Span::new(path.clone(), 5, 6);
    let token = crate::lexer::Token {
        token_type: crate::lexer::TokenType::Ident("value".to_string()),
        span: invocation_span.clone(),
    };
    let cases = [
        (
            ParseError::HardError("hard failure".to_string(), macro_span.clone()),
            DiagnosticCode::Parser,
            DiagnosticLocation::Source(macro_span.clone()),
        ),
        (
            ParseError::Lexer(LexerError::UnknownToken('!', invocation_span.clone())),
            DiagnosticCode::Lexer,
            DiagnosticLocation::Source(invocation_span.clone()),
        ),
        (
            ParseError::MacroNoCorrespondance {
                macro_name: macro_span.clone(),
                invoc_name: invocation_span.clone(),
                invoc_arg: Some(argument_span.clone()),
            },
            DiagnosticCode::Macro,
            DiagnosticLocation::Source(macro_span.clone()),
        ),
        (
            ParseError::UnexpectedToken("identifier".to_string(), token),
            DiagnosticCode::Parser,
            DiagnosticLocation::Source(invocation_span),
        ),
    ];

    for (error, code, location) in cases {
        let diagnostic = Diagnostic::from(error.with_context("outer").with_context("inner"));
        assert_eq!(diagnostic.code, Some(code));
        assert_eq!(diagnostic.location, location);
        assert!(diagnostic.message.contains("outer"));
        assert!(diagnostic.message.contains("inner"));
    }
}

#[test]
fn unexpected_indent_preserves_offending_token_span() {
    let span = Span::new(PathBuf::from("/virtual/main.rk"), 11, 12);
    let diagnostic = Diagnostic::from(ParseError::UnexpectedIndent(3, span.clone()));
    assert_eq!(diagnostic.code, Some(DiagnosticCode::Parser));
    assert_eq!(
        diagnostic.location,
        DiagnosticLocation::Source(span.clone())
    );
    assert_eq!(diagnostic.primary.unwrap().span.start, span.start);
}

#[test]
fn resolve_error_conversion_preserves_family_and_source_span() {
    let span = Span::new(PathBuf::from("/virtual/main.rk"), 5, 8);
    let error = crate::lower::ResolveError::with_span_code(
        "type mismatch".to_string(),
        span.clone(),
        DiagnosticCode::Type,
    );

    let diagnostic = Diagnostic::from_resolve_error(&error);

    assert_eq!(diagnostic.code, Some(DiagnosticCode::Type));
    assert_eq!(diagnostic.location, DiagnosticLocation::Source(span));
    assert!(!diagnostic.message.contains("TypeVarId"));
}

#[test]
fn resolve_error_conversion_accepts_only_validated_origin_states() {
    let source = crate::lower::ResolveError::with_span(
        "source".to_string(),
        Span::new(PathBuf::from("/virtual/main.rk"), 1, 2),
    );
    let non_source = crate::lower::ResolveError::non_source("internal".to_string());

    assert!(Diagnostic::from_resolve_error(&source).validate().is_ok());
    assert!(Diagnostic::from_resolve_error(&non_source)
        .validate()
        .is_ok());
}

#[test]
fn attached_source_validation_rejects_non_boundary_ranges() {
    let path = PathBuf::from("/virtual/main.rk");
    let diagnostic = Diagnostic::new("bad range".to_string(), Span::new(path.clone(), 1, 2))
        .with_source(DiagnosticSource {
            display_path: path,
            text: "é".to_string(),
            origin: DiagnosticSourceOrigin::Virtual,
            related: BTreeMap::new(),
        });

    assert_eq!(
        diagnostic.validate(),
        Err(DiagnosticValidationError::InvalidLocation)
    );
    assert!(diagnostic.to_dto().is_err());
}

#[test]
fn attached_source_validation_rejects_invalid_secondary_ranges() {
    let path = PathBuf::from("/virtual/main.rk");
    let diagnostic = Diagnostic::new("bad label".to_string(), Span::new(path.clone(), 0, 1))
        .with_label("invalid".to_string(), Span::new(path.clone(), 0, 4))
        .with_source(DiagnosticSource {
            display_path: path,
            text: "abc".to_string(),
            origin: DiagnosticSourceOrigin::Virtual,
            related: BTreeMap::new(),
        });

    assert_eq!(
        diagnostic.validate(),
        Err(DiagnosticValidationError::InvalidLabel)
    );
    assert!(diagnostic.to_dto().is_err());
}

#[test]
fn report_rejects_invalid_attached_ranges_without_panicking() {
    let path = PathBuf::from("/virtual/main.rk");
    let diagnostic = Diagnostic::new(
        "bad report range".to_string(),
        Span::new(path.clone(), 0, 4),
    )
    .with_source(DiagnosticSource {
        display_path: path,
        text: "ok".to_string(),
        origin: DiagnosticSourceOrigin::Virtual,
        related: BTreeMap::new(),
    });

    let result = std::panic::catch_unwind(|| diagnostic.report());
    assert!(result.is_ok());
}

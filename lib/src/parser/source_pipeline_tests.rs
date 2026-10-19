use crate::lexer::{LexerError, TokenType};
use crate::lexer::lossless::{LosslessStream, ScanLimits};
use crate::parser::{parse_source, ParseError};
use crate::Config;

#[test]
fn macro_a0_10_source_rejects_nul_instead_of_forging_eof() {
    let error = parse_source("/virtual/nul.rk".into(), "main = -> 42\0trailing", &Config::default()).unwrap_err();
    assert!(matches!(error, ParseError::Lexer(LexerError::UnknownToken('\0', span)) if span.start == 12 && span.end == 13));
}

const STORAGE: ScanLimits = ScanLimits { max_source_bytes: 1024 * 1024, max_runs: 65536 };

fn scan(text: &str) -> LosslessStream {
    LosslessStream::from_bytes("/virtual/generated.rk".into(), text.as_bytes().to_vec(), STORAGE).unwrap()
}

#[test]
fn macro_a0_10_eof_line_comments_are_bounded_and_preserve_ordinary_code() {
    for text in ["main = -> 42 // final", "main = ->\n  42\n   // ignored margin"] {
        let module = parse_source("/virtual/comments.rk".into(), text, &Config::default()).unwrap();
        assert_eq!(module.top_levels.len(), 1);
    }
    // Empty-module grammar is unchanged; an EOF comment must not hang its view.
    assert_eq!(parse_source("/virtual/comments.rk".into(), "// only", &Config::default()).is_ok(),
        parse_source("/virtual/comments.rk".into(), "", &Config::default()).is_ok());
}

#[test]
fn macro_a0_10_unterminated_block_comments_and_wide_indentation_are_located_errors() {
    let comment = "main = -> 42 /* broken";
    let error = parse_source("/virtual/comment.rk".into(), comment, &Config::default()).unwrap_err();
    assert!(matches!(error, ParseError::HardError(message, span)
        if message.contains("unterminated Rock block comment") && span.start == 13 && span.end == comment.len()));
    let text = format!("main = ->\n{}42\n", " ".repeat(256));
    let error = parse_source("/virtual/wide.rk".into(), &text, &Config::default()).unwrap_err();
    assert!(matches!(error, ParseError::HardError(message, span)
        if message.contains("indentation width 256") && span.start == 10 && span.end == 266));
}

#[test]
fn macro_a0_10_generated_and_ordinary_source_use_identical_spans_and_eof_rules() {
    let config = Config::default();
    for text in ["main = -> 42", "main = !->\r\n  42\r\n", "main = -> 42 // eof"] {
        let stream = scan(text);
    let generated = crate::macro_expansion::generated_parser::parse_generated_source(&stream, &config, None).unwrap();
        let source = parse_source(stream.start_span().file_path, text, &config).unwrap();
        assert_eq!(format!("{generated:?}"), format!("{source:?}"));
    }
    let original = scan("prefix main = -> 42 suffix");
    let bounded = original.slice(7..19, STORAGE).unwrap();
    let module = crate::macro_expansion::generated_parser::parse_generated_source(&bounded, &config, None).unwrap();
    let crate::ast::TopLevel::FunctionDecl(function) = &module.top_levels[0] else { panic!("expected function") };
    assert_eq!(function.name.span.start..function.name.span.end, 7..11);
    let broken = scan("main = -> \"bad");
    let diagnostics = crate::macro_expansion::generated_parser::parse_generated_source(&broken, &config, None).unwrap_err();
    assert!(diagnostics.0.iter().any(|diagnostic| {
        matches!(&diagnostic.location, crate::diagnostic::DiagnosticLocation::Source(span)
            if span.start == 10 && span.end == broken.len_bytes())
    }));
}

#[test]
fn macro_a0_10_capture_removes_speculative_diagnostics_only_inside_its_physical_range() {
    use crate::parser::{get_best_error, reset_best_error, track_error, ParseCtx};
    use crate::parser::source_cursor::{SourceInput, SourceLimits};
    use crate::lexer::Span;

    let stream = scan("%dsl\n   odd\nnext\n");
    let input = SourceInput::new(&stream, true, SourceLimits { max_entries: 128, max_branches: 4 }).unwrap();
    let config = Config::default();
    let checkpoint = ParseCtx::from_source(&input, &config).advance(3).unwrap();
    reset_best_error();
    track_error(&checkpoint.seek().unwrap_err());
    assert!(matches!(get_best_error(ParseError::Fail), ParseError::UnexpectedIndent(3, _)));
    let (resumed, _) = checkpoint.capture_to(12, STORAGE).unwrap();
    assert!(matches!(get_best_error(ParseError::Fail), ParseError::Fail));
    assert_eq!(resumed.seek_nth(1).unwrap().token_type, TokenType::Ident("next".into()));
    let outside = ParseError::Lexer(LexerError::UnknownToken('?', Span::new("/other.rk".into(), 0, 1)));
    track_error(&outside);
    checkpoint.capture_to(12, STORAGE).unwrap();
    assert_eq!(format!("{:?}", get_best_error(ParseError::Fail)), format!("{outside:?}"));
    reset_best_error();
}

#[test]
fn macro_a0_10_debug_token_observation_does_not_request_opaque_input() {
    use crate::parser::source_cursor::{SourceInput, SourceLimits};

    let stream = scan("%dsl\n    \"broken");
    let input = SourceInput::new(&stream, true, SourceLimits { max_entries: 128, max_branches: 4 }).unwrap();
    let checkpoint = input.cursor().advance(3).unwrap();
    let before = format!("{:?}", input.tokens_debug());
    assert!(before.contains("MacroInvoc"));
    assert!(!before.contains("broken"));
    let (resumed, _) = checkpoint.capture_to(stream.len_bytes(), STORAGE).unwrap();
    assert_eq!(format!("{:?}", input.tokens_debug()), before);
    assert_eq!(resumed.peek(0).unwrap().unwrap().token.token_type, TokenType::Indent(0));
}

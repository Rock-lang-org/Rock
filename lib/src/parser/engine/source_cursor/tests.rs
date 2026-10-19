use super::{SourceError, SourceInput, SourceLimits};
use crate::lexer::interpretation::RockError;
use crate::lexer::lossless::{LosslessStream, ScanLimits};
use crate::lexer::TokenType;
use crate::parser::{ParseCtx, ParseError, Parser};
use crate::Config;

const STORAGE: ScanLimits = ScanLimits { max_source_bytes: 1024 * 1024, max_runs: 65536 };
const CACHE: SourceLimits = SourceLimits { max_entries: 65536, max_branches: 128 };

fn scan(text: &str) -> LosslessStream {
    LosslessStream::from_bytes("/virtual/macro-a0-09.rk".into(), text.as_bytes().to_vec(), STORAGE).unwrap()
}

#[test]
fn macro_a0_09_capture_resumes_after_unterminated_quotes_without_lexing_the_body() {
    let text = "%dsl\n   \"<p>🦀\nnext = ->\n  42\n";
    let stream = scan(text);
    let input = SourceInput::new(&stream, true, CACHE).unwrap();
    let mut cursor = input.cursor();
    while cursor.peek(0).unwrap().unwrap().token.token_type != TokenType::Eol {
        cursor = cursor.advance(1).unwrap();
    }
    cursor = cursor.advance(1).unwrap();
    let end = text.find("next").unwrap();
    let (resumed, captured) = cursor.capture_to(end, STORAGE).unwrap();
    assert_eq!(captured.runs().iter().map(|run| run.text()).collect::<String>(), &text[5..end]);
    assert_eq!(resumed.peek(0).unwrap().unwrap().token.token_type, TokenType::Indent(0));
    let code = resumed.advance(1).unwrap();
    assert_eq!(code.peek(0).unwrap().unwrap().token.token_type, TokenType::Ident("next".into()));
    assert_eq!(code.indent_step().unwrap(), 4);
    let body = code.advance(4).unwrap();
    assert_eq!(body.indent_step().unwrap(), 2);
}

#[test]
fn macro_a0_09_repeated_successful_and_failing_peeks_are_cached() {
    let stream = scan("x /* unterminated");
    let input = SourceInput::new(&stream, false, CACHE).unwrap();
    let cursor = input.cursor();
    for _ in 0..128 {
        assert_eq!(cursor.peek(1).unwrap().unwrap().token.token_type, TokenType::Ident("x".into()));
    }
    assert_eq!(input.read_count(), 2);
    for _ in 0..128 {
        assert!(cursor.peek(2).is_err());
    }
    assert_eq!(input.read_count(), 3);
    assert_eq!(cursor.peek(1).unwrap().unwrap().token.span.start, 0);
}

#[test]
fn macro_a0_09_capture_discards_speculative_errors_and_indentation_choices() {
    for body in ["   odd\n", "    /* unterminated\n", "    \"unterminated\n", "    🦀 ([}\n", "    \0\n"] {
        let text = format!("%dsl\n{body}next = ->\n  42\n");
        let stream = scan(&text);
        let input = SourceInput::new(&stream, true, CACHE).unwrap();
        let config = Config::default();
        let original = ParseCtx::from_source(&input, &config);
        let checkpoint = original.advance(3).unwrap();
        assert!(checkpoint.seek_nth(100).is_err());
        assert_eq!(original.indent_step().unwrap(), 4);
        assert!(original.invalid_indent.is_none());
        let (resumed, captured) = checkpoint.capture_to(text.find("next").unwrap(), STORAGE).unwrap();
        assert_eq!(captured.runs().iter().map(|run| run.text()).collect::<String>(), body);
        assert!(resumed.invalid_indent.is_none());
        assert_eq!(resumed.seek_nth(1).unwrap().token_type, TokenType::Ident("next".into()));
        let body = resumed.advance(5).unwrap();
        assert_eq!(body.indent_step().unwrap(), 2);
        assert_eq!(body.seek().unwrap().token_type, TokenType::Indent(2));
        assert_eq!(original.seek_nth(1).unwrap().token_type, TokenType::MacroInvoc("dsl".into()));
        assert!(checkpoint.seek_nth(100).is_err());
    }
}

#[test]
fn macro_a0_09_successful_lookahead_cannot_swallow_the_resume_location() {
    let text = "%dsl\n    \"body\nnext = -> 42\n\"\n";
    let stream = scan(text);
    let input = SourceInput::new(&stream, true, CACHE).unwrap();
    let checkpoint = input.cursor().advance(3).unwrap();
    assert!(matches!(checkpoint.peek(1).unwrap().unwrap().token.token_type, TokenType::String(_)));
    let (resumed, _) = checkpoint.capture_to(text.find("next").unwrap(), STORAGE).unwrap();
    assert_eq!(resumed.peek(1).unwrap().unwrap().token.token_type, TokenType::Ident("next".into()));
    assert!(!resumed.same_remaining(&checkpoint));
    assert!(checkpoint.same_remaining(&checkpoint.advance(0).unwrap()));
    assert_eq!(checkpoint.offset().unwrap(), 5);
}

#[test]
fn macro_a0_09_previously_observed_rock_indent_survives_opaque_bodies() {
    let text = "main = ->\n  x\n%dsl\n     odd\nnext = ->\n  42\n";
    let stream = scan(text);
    let input = SourceInput::new(&stream, true, CACHE).unwrap();
    let mut cursor = input.cursor();
    while cursor.offset().unwrap() < text.find("     odd").unwrap() {
        cursor = cursor.advance(1).unwrap();
    }
    assert!(cursor.peek(0).is_err());
    let (resumed, _) = cursor.capture_to(text.find("next").unwrap(), STORAGE).unwrap();
    assert_eq!(resumed.indent_step().unwrap(), 2);
}

#[test]
fn macro_a0_09_prefixes_bound_lookahead_without_touching_later_invalid_rock() {
    let stream = scan("x \"unterminated");
    let input = SourceInput::new(&stream, false, CACHE).unwrap();
    let cursor = input.cursor();
    let bounded = cursor.prefix(2).unwrap();
    assert_eq!(input.read_count(), 2);
    assert_eq!(bounded.eof_span().unwrap().start, 1);
    for index in [2, 3, usize::MAX] {
        assert!(bounded.peek(index).unwrap().is_none());
    }
    assert_eq!(input.read_count(), 2);
    let empty = bounded.advance(2).unwrap();
    assert_eq!(empty.offset().unwrap(), 1);
    assert!(empty.peek(0).unwrap().is_none());
    assert!(matches!(empty.advance(1), Err(SourceError::UnexpectedEOF(span)) if span.start == 1));
    assert!(cursor.peek(2).is_err());
    assert!(bounded.peek(2).unwrap().is_none());
    assert!(cursor.peek(usize::MAX).is_err());
    assert!(!cursor.same_remaining(&bounded));
}

#[test]
fn macro_a0_09_capture_requires_checked_utf8_and_physical_line_boundaries() {
    let text = "%dsl\r\n    🦀\r\nnext\n";
    let stream = scan(text);
    let input = SourceInput::new(&stream, true, CACHE).unwrap();
    let cursor = input.cursor().advance(3).unwrap();
    assert_eq!(cursor.offset().unwrap(), 6);
    for end in [7, text.find("next").unwrap() - 1] {
        assert!(matches!(cursor.capture_to(end, STORAGE), Err(SourceError::Rock(RockError::InvalidCaptureBoundary { .. }))));
        assert_eq!(cursor.offset().unwrap(), 6);
    }
    assert!(matches!(cursor.capture_to(11, STORAGE), Err(SourceError::Rock(RockError::Preservation(_)))));
    for end in [0, text.len() + 1, usize::MAX] {
        assert!(cursor.capture_to(end, STORAGE).is_err());
    }
    let bounded = cursor.prefix(0).unwrap();
    assert!(bounded.capture_to(text.find("next").unwrap(), STORAGE).is_err());
    let (resumed, captured) = cursor.capture_to(text.find("next").unwrap(), STORAGE).unwrap();
    assert_eq!(captured.runs().iter().map(|run| run.text()).collect::<String>(), "    🦀\r\n");
    assert_eq!(resumed.peek(1).unwrap().unwrap().token.token_type, TokenType::Ident("next".into()));
    assert!(cursor.advance(1).unwrap().capture_to(text.find("next").unwrap(), STORAGE).is_err());
}

#[test]
fn macro_a0_09_eof_capture_retains_real_source_extents_and_terminal_origins() {
    use crate::lexer::interpretation::layout::LayoutOrigin;

    let original = scan("prefix %dsl\n   \"🦀");
    let stream = original.slice(7..original.len_bytes(), STORAGE).unwrap();
    let input = SourceInput::new(&stream, true, CACHE).unwrap();
    let checkpoint = input.cursor().advance(3).unwrap();
    let (resumed, body) = checkpoint.capture_to(stream.len_bytes(), STORAGE).unwrap();
    assert_eq!(body.runs().iter().map(|run| run.text()).collect::<String>(), "   \"🦀");
    let eof = original.end_span();
    for (index, kind) in [TokenType::Indent(0), TokenType::Eol, TokenType::Eof].into_iter().enumerate() {
        let token = resumed.peek(index).unwrap().unwrap();
        assert_eq!(token.token.token_type, kind);
        assert_eq!(token.token.span, eof);
        assert_eq!(token.origin, if index == 2 { LayoutOrigin::EndOfInput } else { LayoutOrigin::Synthetic });
    }
    assert!(resumed.peek(3).unwrap().is_none());
    assert_eq!(resumed.eof_span().unwrap(), eof);
    assert_eq!(body.end_span(), eof);
}

#[test]
fn macro_a0_09_cache_and_branch_budgets_fail_without_repeating_lexical_work() {
    let stream = scan("x y z");
    let input = SourceInput::new(&stream, false, SourceLimits { max_entries: 2, ..CACHE }).unwrap();
    for _ in 0..128 {
        assert!(matches!(input.cursor().peek(1000), Err(SourceError::LimitExceeded { resource: "entries", .. })));
    }
    assert_eq!(input.read_count(), 2);
    assert_eq!(input.cursor().peek(1).unwrap().unwrap().token.token_type, TokenType::Ident("x".into()));
    let input = SourceInput::new(&stream, false, SourceLimits { max_branches: 1, ..CACHE }).unwrap();
    assert!(matches!(input.cursor().capture_to(0, STORAGE), Err(SourceError::LimitExceeded { resource: "branches", .. })));
    assert_eq!(input.read_count(), 0);
    assert_eq!(input.cursor().offset().unwrap(), 0);
    assert!(matches!(SourceInput::new(&stream, true, SourceLimits { max_branches: 0, ..CACHE }), Err(SourceError::LimitExceeded { .. })));
}

#[test]
fn macro_a0_09_capture_preserves_segment_origins_and_explicit_splice_boundaries() {
    use crate::lexer::lossless::JoinBoundary;

    let header = scan("%dsl\n");
    let body = LosslessStream::from_bytes("/virtual/body.rk".into(), b"   \"broken\n".to_vec(), STORAGE).unwrap();
    let next = LosslessStream::from_bytes("/virtual/next.rk".into(), b"next\n".to_vec(), STORAGE).unwrap();
    let stream = header.join(&body, JoinBoundary::Separate, STORAGE).unwrap()
        .join(&next, JoinBoundary::Separate, STORAGE).unwrap();
    let input = SourceInput::new(&stream, true, CACHE).unwrap();
    let (resumed, captured) = input.cursor().advance(3).unwrap().capture_to(header.len_bytes() + body.len_bytes(), STORAGE).unwrap();
    assert_eq!(captured.start_span(), body.start_span());
    assert_eq!(captured.end_span(), body.end_span());
    assert_eq!(captured.runs()[0].boundary(), JoinBoundary::Separate);
    assert_eq!(resumed.peek(1).unwrap().unwrap().token.span.file_path, next.start_span().file_path);
    assert_eq!(resumed.peek(1).unwrap().unwrap().token.span.start, 0);
}

#[test]
fn macro_a0_09_lossless_backing_runs_existing_grammar_without_source_entrypoint_changes() {
    for text in [
        "main = -> 42\n", "main = !->\n    42\n", "main = !->\n  42\n",
        "main = -> [[1; 2]; 3]\n", "struct Pair\n    value: I64\n",
        "// lead\nmain = !->\n  // comment\n  42\n",
        "main = ->\n\n  42\n", "main = ->\r\n  42\r\n",
    ] {
        let stream = scan(text);
        let input = SourceInput::new(&stream, true, CACHE).unwrap();
        let config = Config::default();
        let context = ParseCtx::from_source(&input, &config);
        let expected = crate::parser::parse_source(stream.start_span().file_path.clone(), text, &config);
        crate::parser::reset_best_error();
        let actual = crate::parser::module_inline.process(context).map(|(_, mut module)| {
            module.filepath = Some(stream.start_span().file_path.clone());
            module
        });
        assert_eq!(format!("{actual:?}"), format!("{expected:?}"), "source: {text:?}");
    }
}

#[test]
fn macro_a0_09_parse_context_preserves_errors_at_nonempty_suffixes_and_bounds() {
    let stream = scan("x\0");
    let input = SourceInput::new(&stream, false, CACHE).unwrap();
    let config = Config::default();
    let original = ParseCtx::from_source(&input, &config);
    let bounded = original.prefix(2).unwrap();
    assert_eq!(bounded.eof_span().start, 1);
    assert!(bounded.advance(2).unwrap().is_empty());
    let failed = original.advance(2).unwrap();
    assert!(!failed.is_empty());
    assert_eq!(failed.len(), 1);
    assert!(matches!(failed.seek(), Err(ParseError::Lexer(_))));
    assert_eq!(original.seek_nth(1).unwrap().token_type, TokenType::Ident("x".into()));
    let restored = failed.resume_input_from(bounded);
    assert!(restored.same_remaining(&bounded));
    assert_eq!(restored.eof_span().start, 1);
    assert!(restored.seek_nth(2).is_err());
}

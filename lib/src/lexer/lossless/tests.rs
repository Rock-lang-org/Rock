use std::path::PathBuf;

use super::{LosslessError, LosslessStream, RunKind, ScanLimits};

const SOURCE_PATH: &str = "/virtual/macro-a0-02.rk";
const LIMITS: ScanLimits = ScanLimits {
    max_source_bytes: 1024 * 1024,
    max_runs: 64 * 1024,
};

fn scan(source: &str) -> LosslessStream {
    LosslessStream::from_bytes(PathBuf::from(SOURCE_PATH), source.as_bytes().to_vec(), LIMITS)
        .unwrap()
}

fn text(stream: &LosslessStream) -> String {
    stream.runs().iter().map(|run| run.text()).collect()
}

#[test]
fn macro_a0_02_runs_cover_original_utf8_bytes_and_layout() {
    let source = "a \té\r\n🦀\0";
    let stream = scan(source);
    assert_eq!(text(&stream), source);
    assert_eq!(stream.start_span().file_path, PathBuf::from(SOURCE_PATH));
    let expected = [
        (RunKind::Text, 0..1, "a"),
        (RunKind::Whitespace, 1..3, " \t"),
        (RunKind::Text, 3..5, "é"),
        (RunKind::LineEnding, 5..7, "\r\n"),
        (RunKind::Text, 7..12, "🦀\0"),
    ];
    assert_eq!(stream.runs().len(), expected.len());
    for (run, (kind, range, spelling)) in stream.runs().iter().zip(expected) {
        assert_eq!(run.kind(), kind);
        assert_eq!(run.range(), range);
        assert_eq!(run.text(), spelling);
        assert_eq!(run.span(), crate::lexer::Span::new(SOURCE_PATH.into(), range.start, range.end));
    }
    assert_eq!(stream.runs().iter().map(|run| run.text()).collect::<String>(), source);
}

#[test]
fn macro_a0_02_invalid_utf8_is_rejected_at_ingestion() {
    let result = LosslessStream::from_bytes(
        SOURCE_PATH.into(), vec![b'a', 0xff, b'b'], LIMITS,
    );
    assert!(matches!(
        result,
        Err(LosslessError::InvalidUtf8 { valid_up_to: 1, error_len: Some(1) }),
    ));
}

fn assert_coverage(source: &str) {
    let stream = scan(source);
    let mut end = 0;
    for run in stream.runs() {
        let range = run.range();
        assert_eq!(range.start, end, "gap/overlap in {source:?}");
        assert!(range.end > range.start, "empty run in {source:?}");
        assert!(source.is_char_boundary(range.start));
        assert!(source.is_char_boundary(range.end));
        assert_eq!(run.text(), &source[range.clone()]);
        assert_eq!(run.text().as_ptr(), run.source.text[range.clone()].as_ptr());
        assert!(std::sync::Arc::ptr_eq(&stream.anchor.source, &run.source));
        end = range.end;
    }
    assert_eq!(end, source.len());
    assert!(stream.runs().len() <= source.chars().count());
    assert_eq!(stream.runs().iter().map(|run| run.text()).collect::<String>(), source);
}

#[test]
fn macro_a0_02_arbitrary_dsl_and_incomplete_rock_are_preserved() {
    for source in [
        "", " ", "\t\t", "\r\n", "\0", "x\0y", "`backticks` 🦀",
        "\u{feff}é中e\u{301}\u{a0}\u{2028}\u{2029}",
        "\"unterminated", "'unterminated", "\"escaped\\\"", "\\",
        "// eof", "// comment\r\nnext", "/* unterminated",
        "/* outer /* nested */", "/* not closed\n  odd\nnext = 1\n",
        "([}", ")}]", "<div class=\"open>\nI'm arbitrary HTML\n",
        "%html\r\n    <p title=\"unterminated>\r\n   odd # 🦀\r\nnext = 1\r\n",
    ] {
        assert_coverage(source);
        let first = scan(source);
        let second = scan(source);
        let descriptors = |stream: &LosslessStream| {
            stream.runs().iter().map(|run| (run.kind(), run.range())).collect::<Vec<_>>()
        };
        assert_eq!(descriptors(&first), descriptors(&second));
    }
}

#[test]
fn macro_a0_02_quotes_and_comments_cannot_hide_physical_lines() {
    for source in [
        "\"open\r\n body\rnext\n",
        "/* open\r\n body\rnext\n",
        "// line\r\n body\rnext\n",
    ] {
        assert_coverage(source);
        let stream = scan(source);
        let endings = stream.runs().iter()
            .filter(|run| run.kind() == RunKind::LineEnding)
            .map(|run| run.text())
            .collect::<Vec<_>>();
        assert_eq!(endings, vec!["\r\n", "\r", "\n"]);
        assert!(stream.runs().iter().any(|run| run.text() == "next"));
    }
}

#[test]
fn macro_a0_02_each_physical_line_ending_remains_separate() {
    let stream = scan("\r\r\n\n\n\r");
    assert_eq!(
        stream.runs().iter().map(|run| run.text()).collect::<Vec<_>>(),
        vec!["\r", "\r\n", "\n", "\n", "\r"],
    );
    assert!(stream.runs().iter().all(|run| run.kind() == RunKind::LineEnding));
    assert_coverage(&text(&stream));
}

#[test]
fn macro_a0_02_empty_input_needs_no_runs_or_synthetic_eof() {
    let stream = LosslessStream::from_bytes(
        SOURCE_PATH.into(), Vec::new(), ScanLimits { max_source_bytes: 0, max_runs: 0 },
    ).unwrap();
    assert_eq!(text(&stream), "");
    assert!(stream.runs().is_empty());
}

#[test]
fn macro_a0_02_input_spelling_is_owned_without_copying() {
    let bytes = "é 🦀\0\r\n".as_bytes().to_vec();
    let original_pointer = bytes.as_ptr();
    let stream = LosslessStream::from_bytes(SOURCE_PATH.into(), bytes, LIMITS).unwrap();
    assert_eq!(stream.anchor.source.text.as_ptr(), original_pointer);
    let run = stream.runs()[0].clone();
    drop(stream);
    assert_eq!(run.text(), "é");
    assert_eq!(run.span(), crate::lexer::Span::new(SOURCE_PATH.into(), 0, 2));
}

#[test]
fn macro_a0_02_large_homogeneous_input_is_coalesced_not_per_character() {
    for (source, kind) in [
        ("é🦀\0`/*'([".repeat(4096), RunKind::Text),
        (" \t\u{a0}".repeat(4096), RunKind::Whitespace),
    ] {
        let stream = LosslessStream::from_bytes(
            SOURCE_PATH.into(), source.clone().into_bytes(),
            ScanLimits { max_source_bytes: source.len(), max_runs: 1 },
        ).unwrap();
        assert_eq!(stream.runs().len(), 1);
        assert_eq!(stream.runs()[0].kind(), kind);
        assert_eq!(stream.runs()[0].text(), source);
        assert_eq!(stream.runs()[0].range(), 0..source.len());
    }
}

#[test]
fn macro_a0_02_source_byte_budget_includes_trivia_and_multibyte_spelling() {
    for source in ["\0", "   ", "// comment", "🦀", "\r\n"] {
        let limits = ScanLimits { max_source_bytes: source.len(), max_runs: source.len() };
        assert!(LosslessStream::from_bytes(
            SOURCE_PATH.into(), source.as_bytes().to_vec(), limits,
        ).is_ok());
        let result = LosslessStream::from_bytes(
            SOURCE_PATH.into(), source.as_bytes().to_vec(),
            ScanLimits { max_source_bytes: source.len() - 1, ..limits },
        );
        assert!(matches!(
            result,
            Err(LosslessError::SourceTooLarge { bytes, limit })
                if bytes == source.len() && limit == source.len() - 1,
        ));
    }
}

#[test]
fn macro_a0_02_run_budget_fails_at_a_byte_boundary_without_partial_success() {
    for (source, limit, offset) in [("🦀 x", 0, 0), ("🦀 x", 1, 4), ("🦀 x", 2, 5)] {
        let result = LosslessStream::from_bytes(
            SOURCE_PATH.into(), source.as_bytes().to_vec(),
            ScanLimits { max_runs: limit, ..LIMITS },
        );
        assert!(matches!(
            result,
            Err(LosslessError::TooManyRuns { offset: actual, limit: actual_limit })
                if actual == offset && actual_limit == limit,
        ));
    }
    assert!(LosslessStream::from_bytes(
        SOURCE_PATH.into(), b"a b".to_vec(), ScanLimits { max_runs: 3, ..LIMITS },
    ).is_ok());
}

#[test]
fn macro_a0_02_empty_source_path_is_rejected_before_span_construction() {
    assert!(matches!(
        LosslessStream::from_bytes(PathBuf::new(), Vec::new(), LIMITS),
        Err(LosslessError::EmptySourcePath),
    ));
}

#[test]
fn macro_a0_02_malformed_utf8_is_never_lossily_replaced() {
    for (bytes, valid_up_to, error_len) in [
        (vec![0xc0, 0xaf], 0, Some(1)), // Overlong encoding.
        (vec![0xed, 0xa0, 0x80], 0, Some(1)), // Surrogate.
        (vec![0xf4, 0x90, 0x80, 0x80], 0, Some(1)), // Beyond Unicode.
        (vec![b'a', 0xe2, 0x82], 1, None), // Truncated scalar.
        (vec![0xc3, 0xa9, 0x80], 2, Some(1)), // Stray continuation after é.
    ] {
        assert!(matches!(
            LosslessStream::from_bytes(SOURCE_PATH.into(), bytes, LIMITS),
            Err(LosslessError::InvalidUtf8 { valid_up_to: actual, error_len: length })
                if actual == valid_up_to && length == error_len,
        ));
    }
}

#[test]
fn macro_a0_02_generated_short_inputs_always_progress_and_round_trip() {
    let alphabet = ["a", "é", "🦀", "\0", " ", "\t", "\r", "\n", "'", "\"", "/", "*", "[", "`"];
    for a in alphabet {
        for b in alphabet {
            for c in alphabet {
                assert_coverage(&format!("{a}{b}{c}"));
            }
        }
    }
}

use super::{PieceKind, RockError, RockView};
use crate::lexer::lossless::{JoinBoundary, LosslessStream, ScanLimits};
use crate::lexer::{Lexer, LexerError, TokenType};

const LIMITS: ScanLimits = ScanLimits {
    max_source_bytes: 1024 * 1024,
    max_runs: 64 * 1024,
};

fn scan(source: &str) -> LosslessStream {
    LosslessStream::from_bytes("/virtual/macro-a0-05.rk".into(), source.as_bytes().to_vec(), LIMITS)
        .unwrap()
}

#[test]
fn macro_a0_05_unicode_literals_keep_escaped_spelling_and_source_bytes() {
    let source = scan(r#""é\n\"🦀" '中' 7"#);
    let mut view = RockView::new(&source);
    let string = view.next_piece().unwrap().unwrap();
    assert_eq!(string.kind, PieceKind::Token(TokenType::String(r#"é\n\"🦀"#.into())));
    assert_eq!(string.range, 0..12);
    assert_eq!(string.span.start..string.span.end, 0..12);
    assert_eq!(view.next_piece().unwrap().unwrap().kind, PieceKind::Whitespace);
    let character = view.next_piece().unwrap().unwrap();
    assert_eq!(character.kind, PieceKind::Token(TokenType::Char("中".into())));
    assert_eq!(character.range, 13..18);
}

#[test]
fn macro_a0_05_unclosed_literal_cannot_read_past_its_supplied_slice() {
    let source = scan("prefix \"é\" suffix");
    let captured = source.slice(7..10, LIMITS).unwrap();
    let mut view = RockView::new(&captured);
    assert!(matches!(
        view.next_piece(),
        Err(RockError::Lexer(LexerError::UnknownToken('"', span)))
            if span.start == 7 && span.end == 10,
    ));
    assert_eq!(view.offset(), 0);
}

fn pieces(stream: &LosslessStream) -> Vec<(PieceKind, std::ops::Range<usize>, crate::lexer::Span)> {
    let mut view = RockView::new(stream);
    let mut pieces = Vec::new();
    while let Some(piece) = view.next_piece().unwrap() {
        assert!(piece.range.end > piece.range.start);
        pieces.push((piece.kind, piece.range, piece.span));
    }
    assert_eq!(view.offset(), stream.len_bytes());
    pieces
}

#[test]
fn macro_a0_05_literals_and_terminated_comments_match_existing_rock_tokens() {
    for spelling in [
        "\"é中🦀\" '🦀' '\\'' 42\n",
        r#""" '' "a\\" "a\"b" '\n' '\\' "unknown\q" tail"#,
        "\"first\nsecond\" tail\n",
        "a // line\n    // only\n    b /* block */\nc\n",
        "/* first\nsecond */ \"literal /* not a comment */\" tail\n",
        "// \"not a literal\n42 /* \"also text */\n",
    ] {
        let stream = scan(spelling);
        let expected = Lexer::new("/virtual/macro-a0-05.rk".into(), spelling).unwrap()
            .with_newline_at_end(false).collect().unwrap().into_iter()
            .filter(|token| !matches!(token.token_type, TokenType::Indent(_) | TokenType::Eol | TokenType::Eof))
            .map(|token| (token.token_type, token.span)).collect::<Vec<_>>();
        let actual = pieces(&stream).into_iter().filter_map(|(kind, _, span)| match kind {
            PieceKind::Token(token) => Some((token, span)),
            _ => None,
        }).collect::<Vec<_>>();
        assert_eq!(actual, expected, "source: {spelling:?}");
    }
}

#[test]
fn macro_a0_05_comments_remain_retrievable_with_exact_spelling_and_origins() {
    let spelling = "a // é🦀\r\n/* unmatched ([ \" \0 */b";
    let stream = scan(spelling);
    let classified = pieces(&stream);
    let comments = classified.iter().filter(|(kind, _, _)| matches!(
        kind, PieceKind::LineComment | PieceKind::BlockComment,
    )).collect::<Vec<_>>();
    assert_eq!(comments.len(), 2);
    for (kind, range, span) in comments {
        let slice = stream.slice(range.clone(), LIMITS).unwrap();
        let captured = slice.runs().iter().map(|run| run.text()).collect::<String>();
        assert_eq!(captured, spelling[span.start..span.end]);
        assert_eq!(captured, match kind {
            PieceKind::LineComment => "// é🦀\r",
            _ => "/* unmatched ([ \" \0 */",
        });
    }
    assert_eq!(classified.last().unwrap().0, PieceKind::Token(TokenType::Ident("b".into())));
}

#[test]
fn macro_a0_05_line_comments_terminate_at_eof_or_a_splice_without_hanging() {
    for spelling in ["//", "// no newline", "// 🦀\0\" /* ", "// first\r"] {
        let stream = scan(spelling);
        let classified = pieces(&stream);
        assert_eq!(classified.len(), 1);
        assert_eq!(classified[0].0, PieceKind::LineComment);
        assert_eq!(classified[0].1, 0..spelling.len());
    }
    let joined = scan("// line").join(&scan("next"), JoinBoundary::Separate, LIMITS).unwrap();
    let classified = pieces(&joined);
    assert_eq!(classified.len(), 2);
    assert_eq!(classified[0].0, PieceKind::LineComment);
    assert_eq!(classified[1].0, PieceKind::Token(TokenType::Ident("next".into())));
}

#[test]
fn macro_a0_05_unterminated_block_comment_is_a_bounded_structured_error() {
    for spelling in ["/*", "/* é🦀\0", "/* \"unterminated\n    odd\nnext = 1\n"] {
        let stream = scan(spelling);
        let mut view = RockView::new(&stream);
        for _ in 0..2 {
            assert!(matches!(
                view.next_piece(),
                Err(RockError::UnterminatedBlockComment(span)) if span.start == 0 && span.end == spelling.len(),
            ));
            assert_eq!(view.offset(), 0);
        }
        assert_eq!(stream.runs().iter().map(|run| run.text()).collect::<String>(), spelling);
    }
}

#[test]
fn macro_a0_05_block_comment_closes_at_the_first_terminator_without_nesting() {
    let stream = scan("/* outer /* inner */tail");
    let classified = pieces(&stream);
    assert_eq!(classified.len(), 2);
    assert_eq!(classified[0].0, PieceKind::BlockComment);
    assert_eq!(classified[0].1, 0..20);
    assert_eq!(classified[1].0, PieceKind::Token(TokenType::Ident("tail".into())));
}

#[test]
fn macro_a0_05_every_adjacent_utf8_cut_preserves_literal_and_comment_views() {
    for spelling in [
        "\"é\\\"🦀\" '中' tail", "/* 🦀 ([ \" */tail", "// é\r\nnext",
        "\"multiline\n  é\" tail", "\"\\\"\" '\\'' \"\\\\\" tail",
    ] {
        let source = scan(spelling);
        let expected = pieces(&source);
        for cut in (0..=spelling.len()).filter(|offset| spelling.is_char_boundary(*offset)) {
            let left = source.slice(0..cut, LIMITS).unwrap();
            let right = source.slice(cut..spelling.len(), LIMITS).unwrap();
            let joined = left.join(&right, JoinBoundary::SourceAdjacent, LIMITS).unwrap();
            assert_eq!(pieces(&joined), expected, "source: {spelling:?}, cut: {cut}");
        }
    }
}

#[test]
fn macro_a0_05_a_splice_cannot_supply_a_quote_or_comment_terminator() {
    for (left, right) in [("\"open", "\""), ("'open", "'"), ("/*open", "*/")] {
        let joined = scan(left).join(&scan(right), JoinBoundary::Separate, LIMITS).unwrap();
        let mut view = RockView::new(&joined);
        let error_span = match view.next_piece().unwrap_err() {
            RockError::Lexer(LexerError::UnknownToken(_, span)) => span,
            RockError::UnterminatedBlockComment(span) => span,
            other => panic!("unexpected error: {other:?}"),
        };
        assert_eq!(error_span.start..error_span.end, 0..left.len());
        assert_eq!(view.offset(), 0);
    }
}

#[test]
fn macro_a0_05_partial_comment_slice_cannot_read_the_original_closing_marker() {
    let original = scan("prefix /*é*/ tail");
    let slice = original.slice(7..11, LIMITS).unwrap();
    let mut view = RockView::new(&slice);
    assert!(matches!(
        view.next_piece(),
        Err(RockError::UnterminatedBlockComment(span)) if span.start == 7 && span.end == 11,
    ));
    assert_eq!(view.offset(), 0);
}

#[test]
fn macro_a0_05_escape_parity_and_nul_do_not_confuse_quote_or_eof_detection() {
    for (spelling, value) in [
        ("\"a\\\\\"", "a\\\\"),
        ("\"a\\\"b\"", "a\\\"b"),
        ("\"a\0b\"", "a\0b"),
    ] {
        let stream = scan(spelling);
        let classified = pieces(&stream);
        assert_eq!(classified.len(), 1);
        assert_eq!(classified[0].0, PieceKind::Token(TokenType::String(value.into())));
        assert_eq!(classified[0].1, 0..spelling.len());
    }
    for spelling in ["\"trailing\\", "\"escaped\\\"", "'\\'"] {
        let source = scan(spelling);
        assert!(RockView::new(&source).next_piece().is_err());
    }
}

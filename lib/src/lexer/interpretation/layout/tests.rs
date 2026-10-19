use super::{LayoutOrigin, LayoutView};
use crate::lexer::interpretation::RockError;
use crate::lexer::lossless::{JoinBoundary, LosslessStream, ScanLimits, SyntaxContext};
use crate::lexer::{Lexer, LexerError, TokenType};

const LIMITS: ScanLimits = ScanLimits {
    max_source_bytes: 1024 * 1024,
    max_runs: 64 * 1024,
};

fn scan(source: &str) -> LosslessStream {
    LosslessStream::from_bytes("/virtual/macro-a0-06.rk".into(), source.as_bytes().to_vec(), LIMITS)
        .unwrap()
}

#[test]
fn macro_a0_06_terminal_layout_is_synthetic_and_anchored_at_real_eof() {
    let stream = scan("x");
    let mut view = LayoutView::new(&stream, true);
    let mut tokens = Vec::new();
    while let Some(token) = view.next_token().unwrap() {
        tokens.push(token);
    }
    assert_eq!(tokens.iter().map(|token| token.token.token_type.clone()).collect::<Vec<_>>(), vec![
        TokenType::Indent(0), TokenType::Ident("x".into()), TokenType::Eol,
        TokenType::Indent(0), TokenType::Eol, TokenType::Eof,
    ]);
    for token in &tokens[2..5] {
        assert_eq!(token.origin, LayoutOrigin::Synthetic);
        assert_eq!(token.token.span.start..token.token.span.end, 1..1);
    }
    assert_eq!(tokens[5].origin, LayoutOrigin::EndOfInput);
}

#[test]
fn macro_a0_06_indentation_over_u8_is_rejected_without_wrapping() {
    let source = format!("{}x\n", " ".repeat(256));
    let stream = scan(&source);
    let mut view = LayoutView::new(&stream, false);
    assert!(matches!(
        view.next_token(),
        Err(RockError::IndentationTooWide { width: 256, span }) if span.start == 0 && span.end == 256,
    ));
}

fn tokens(stream: &LosslessStream, terminal: bool) -> Vec<super::LayoutToken> {
    let mut view = LayoutView::new(stream, terminal);
    let mut output = Vec::new();
    while let Some(token) = view.next_token().unwrap() {
        output.push(token);
    }
    assert_eq!(view.offset(), stream.len_bytes());
    assert!(view.next_token().unwrap().is_none());
    output
}

#[test]
fn macro_a0_06_ordinary_layout_comments_and_spans_match_characterized_rock() {
    for source in [
        "", "\n", " ", " \n", "x", "x\n", "x\r\n", "x\n \n    y\nz\n",
        "struct Pair\n    value: I64 = 42\n",
        "a + b\na +b\na ++ b\na ++b\na.b\na .b\na . b\n",
        "a // line\n    // only\n    b /* block */\nc\n",
        "// lead\nfoo\n", "/* lead */foo\n", "foo /*inline*/ //last\nbar\n",
        "// a\n// b\n// c\n", "foo\n    /*c*/bar\n",
        "foo\n  // odd\n    baz\n", "foo\n  /* multiline\nmore */ baz\n",
        "\"é\\\"🦀\" '中' 7", "\"multiline\nstring\"\nnext\n",
        "\tfoo\n\t\tbar\n", "a\r\n    b\r\n",
        "%m $x $(x)* ~I64Add -> !-> ~> =>\n",
    ] {
        let stream = scan(source);
        for terminal in [false, true] {
            let expected = Lexer::new("/virtual/macro-a0-06.rk".into(), source).unwrap()
                .with_newline_at_end(terminal).collect().unwrap();
            let actual = tokens(&stream, terminal);
            assert_eq!(actual.len(), expected.len(), "source: {source:?}, terminal: {terminal}");
            for (actual, expected) in actual.iter().zip(expected) {
                assert_eq!(actual.token.token_type, expected.token_type, "source: {source:?}, terminal: {terminal}");
                if actual.origin != LayoutOrigin::Synthetic {
                    assert_eq!(actual.token.span, expected.span, "source: {source:?}");
                } else {
                    assert_eq!(actual.token.span, stream.end_span());
                }
            }
        }
    }
}

#[test]
fn macro_a0_06_indentation_limits_apply_to_real_widths_not_blank_lines() {
    for width in [255, 256, 512, 4096] {
        let stream = scan(&format!("{}x\n", " ".repeat(width)));
        let mut view = LayoutView::new(&stream, false);
        if width == 255 {
            assert_eq!(view.next_token().unwrap().unwrap().token.token_type, TokenType::Indent(255));
        } else {
            for _ in 0..2 {
                assert!(matches!(
                    view.next_token(),
                    Err(RockError::IndentationTooWide { width: actual, span })
                        if actual == width && span.start == 0 && span.end == width,
                ));
                assert_eq!(view.offset(), 0);
            }
        }
    }
    let stream = scan(&format!("{}\n", " ".repeat(4096)));
    let first = tokens(&stream, false).remove(0);
    assert_eq!(first.token.token_type, TokenType::Indent(0));
    assert_eq!(first.origin, LayoutOrigin::Source(0..4096));
}

#[test]
fn macro_a0_06_comment_only_indentation_does_not_select_the_parser_step() {
    let source = "main = !->\n   // odd comment margin\n    x\n";
    let stream = scan(source);
    let tokens = tokens(&stream, true);
    let plain = tokens.iter().map(|token| token.token.clone()).collect::<Vec<_>>();
    let config = crate::Config::default();
    let context = crate::parser::ParseCtx::from(&plain, &config);
    assert_eq!(context.indent_step, 4);
    assert!(context.invalid_indent.is_none());
    let recovered = stream.runs().iter().map(|run| run.text()).collect::<String>();
    assert_eq!(recovered, source);
    assert!(recovered.contains("   // odd"));
}

#[test]
fn macro_a0_06_odd_code_indentation_remains_a_located_parser_error() {
    let stream = scan("main = !->\n   x\n");
    let plain = tokens(&stream, true).into_iter().map(|token| token.token).collect::<Vec<_>>();
    let config = crate::Config::default();
    let context = crate::parser::ParseCtx::from(&plain, &config);
    let (width, span) = context.invalid_indent.unwrap();
    assert_eq!(width, 3);
    assert_eq!(span.start..span.end, 11..14);
}

#[test]
fn macro_a0_06_nul_is_a_source_error_not_an_eof_control_marker() {
    let stream = scan("x\0y");
    let mut view = LayoutView::new(&stream, true);
    assert_eq!(view.next_token().unwrap().unwrap().token.token_type, TokenType::Indent(0));
    let name = view.next_token().unwrap().unwrap();
    assert_eq!(name.context, Some(SyntaxContext::ROOT));
    assert_eq!(name.origin, LayoutOrigin::Source(0..1));
    assert!(matches!(
        view.next_token(),
        Err(RockError::Lexer(LexerError::UnknownToken('\0', span))) if span.start == 1 && span.end == 2,
    ));
    assert_eq!(view.offset(), 1);
    assert_eq!(stream.len_bytes(), 3);
}

#[test]
fn macro_a0_06_eof_and_synthetic_tokens_use_slice_extents_not_file_length() {
    let stream = scan("prefix x suffix").slice(7..8, LIMITS).unwrap();
    let output = tokens(&stream, true);
    assert_eq!(output[1].origin, LayoutOrigin::Source(0..1));
    assert_eq!(output[1].token.span.start..output[1].token.span.end, 7..8);
    for token in &output[2..] {
        assert_eq!(token.token.span.start..token.token.span.end, 8..8);
    }
    let empty = stream.slice(1..1, LIMITS).unwrap();
    for token in tokens(&empty, true) {
        assert_ne!(token.origin, LayoutOrigin::Source(0..0));
        assert_eq!(token.token.span.start..token.token.span.end, 8..8);
    }
}

#[test]
fn macro_a0_06_adjacent_run_cuts_do_not_change_layout_or_trivia_semantics() {
    for source in ["a\r\n    b\n", "a // é\n  // odd\n    b\n", "\n \n\"é🦀\"\n", "a .b\n"] {
        let stream = scan(source);
        let describe = |stream: &LosslessStream| {
            tokens(stream, true).into_iter().map(|token| (token.token.token_type, token.token.span, token.origin, token.context))
                .collect::<Vec<_>>()
        };
        let expected = describe(&stream);
        for cut in (0..=source.len()).filter(|offset| source.is_char_boundary(*offset)) {
            let left = stream.slice(0..cut, LIMITS).unwrap();
            let right = stream.slice(cut..source.len(), LIMITS).unwrap();
            let joined = left.join(&right, JoinBoundary::SourceAdjacent, LIMITS).unwrap();
            assert_eq!(describe(&joined), expected, "source: {source:?}, cut: {cut}");
        }
    }
}

#[test]
fn macro_a0_06_layout_does_not_interpret_later_source_before_requested() {
    let stream = scan("main = !->\n    \"unterminated");
    let mut view = LayoutView::new(&stream, true);
    loop {
        let token = view.next_token().unwrap().unwrap();
        if token.token.token_type == TokenType::Eol {
            break;
        }
    }
    assert_eq!(view.offset(), 11);
    assert_eq!(view.next_token().unwrap().unwrap().token.token_type, TokenType::Indent(4));
    assert_eq!(view.offset(), 15);
    assert!(view.next_token().is_err());
}

#[test]
fn macro_a0_06_many_comment_lines_are_iterative_not_recursive() {
    let stream = scan(&format!("{}x\n", "// comment\n".repeat(8192)));
    let output = tokens(&stream, false);
    assert_eq!(output.len(), 4);
    assert_eq!(output[0].token.token_type, TokenType::Indent(0));
    assert_eq!(output[1].token.token_type, TokenType::Ident("x".into()));
}

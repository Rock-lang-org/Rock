use crate::lexer::lossless::{JoinBoundary, LosslessStream, ScanLimits, SyntaxContext};
use crate::lexer::{Lexer, LexerError, Span, TokenType};

use super::{PieceKind, RockError, RockView};

const LIMITS: ScanLimits = ScanLimits {
    max_source_bytes: 1024 * 1024,
    max_runs: 64 * 1024,
};

fn scan(source: &str) -> LosslessStream {
    LosslessStream::from_bytes("/virtual/macro-a0-04.rk".into(), source.as_bytes().to_vec(), LIMITS)
        .unwrap()
}

#[test]
fn macro_a0_04_classifies_ordinary_rock_without_changing_source() {
    let stream = scan("struct Pair = 42");
    let mut view = RockView::new(&stream);
    let mut tokens = Vec::new();
    while let Some(piece) = view.next_piece().unwrap() {
        if let PieceKind::Token(kind) = piece.kind {
            tokens.push((kind, piece.span.start..piece.span.end));
        }
    }
    assert_eq!(tokens, vec![
        (TokenType::Keyword("struct".into()), 0..6),
        (TokenType::Type("Pair".into()), 7..11),
        (TokenType::Equal, 12..13),
        (TokenType::Number("42".into()), 14..16),
    ]);
    assert_eq!(stream.runs().iter().map(|run| run.text()).collect::<String>(), "struct Pair = 42");
}

#[test]
fn macro_a0_04_unknown_unicode_is_preservable_but_not_ordinary_rock() {
    let stream = scan("🦀");
    let mut view = RockView::new(&stream);
    assert!(matches!(
        view.next_piece(),
        Err(RockError::Lexer(LexerError::UnknownToken('🦀', span))) if span.start == 0 && span.end == 4,
    ));
    assert_eq!(stream.runs()[0].text(), "🦀");
}

fn tokens(stream: &LosslessStream) -> Vec<(TokenType, Span, Option<SyntaxContext>)> {
    let mut view = RockView::new(stream);
    let mut tokens = Vec::new();
    while let Some(piece) = view.next_piece().unwrap() {
        assert!(piece.range.end > piece.range.start);
        if let PieceKind::Token(kind) = piece.kind {
            tokens.push((kind, piece.span, piece.context));
        }
    }
    assert_eq!(view.offset(), stream.len_bytes());
    assert!(view.next_piece().unwrap().is_none());
    tokens
}

#[test]
fn macro_a0_04_relevant_existing_token_and_span_contracts_match() {
    // Ordinary, terminated fixtures only; the legacy scanner has unbounded
    // comment paths and invalid byte arithmetic on Unicode identifiers.
    for spelling in [
        "struct enum trait impl if then else for in while loop macro true false return continue break infix mod extern match unsafe type mut where as do tail\n",
        "( ) [ ] { } , : :: .. ..= .x . x ^ ~ @ ? \\ _ _name __ _2 tail\n",
        "%m $x $(x)* ~I64Add -> !-> ~> => tail\n",
        "%Mixed $Type $_x ~lower ~I64_1 tail\n",
        "0 42 123.50 1..2 1..=2 1.foo 1. 1e3 0x12 tail\n",
        "a + b\na +b\na ++ b\na ++b\na.b\na .b\na . b\n",
        "&&x &x || x; y\na >> b\na >=b\n",
    ] {
        let stream = scan(spelling);
        let expected = Lexer::new("/virtual/macro-a0-04.rk".into(), spelling).unwrap()
            .with_newline_at_end(false).collect().unwrap();
        let expected = expected.into_iter().filter(|token| !matches!(
            token.token_type, TokenType::Indent(_) | TokenType::Eol | TokenType::Eof,
        )).map(|token| (token.token_type, token.span)).collect::<Vec<_>>();
        let actual = tokens(&stream).into_iter().map(|(kind, span, _)| (kind, span)).collect::<Vec<_>>();
        assert_eq!(actual, expected, "source: {spelling:?}");
    }
}

#[test]
fn macro_a0_04_every_utf8_cut_can_be_interpreted_across_adjacent_runs() {
    for spelling in [
        "struct Pair = 42\n", "a ++b .show!\n", "%m $x $(x)* ~I64Add -> !-> ~> =>\n",
        "café Éclair _中 %été $中 ~É1", "12.34..=56\n",
    ] {
        let source = scan(spelling);
        let expected = tokens(&source);
        for cut in (0..=spelling.len()).filter(|offset| spelling.is_char_boundary(*offset)) {
            let left = source.slice(0..cut, LIMITS).unwrap();
            let right = source.slice(cut..spelling.len(), LIMITS).unwrap();
            let joined = left.join(&right, JoinBoundary::SourceAdjacent, LIMITS).unwrap();
            assert_eq!(tokens(&joined), expected, "source: {spelling:?}, cut: {cut}");
        }
    }
}

#[test]
fn macro_a0_04_splices_do_not_glue_identifiers_numbers_or_special_markers() {
    for (left, right, expected) in [
        ("foo", "bar", vec![TokenType::Ident("foo".into()), TokenType::Ident("bar".into())]),
        ("-", ">", vec![TokenType::StuckOperator("-".into()), TokenType::StuckOperator(">".into())]),
        ("$", "x", vec![TokenType::StuckOperator("$".into()), TokenType::Ident("x".into())]),
        ("12", ".34", vec![TokenType::Number("12".into()), TokenType::Dot, TokenType::Number("34".into())]),
        (" ", ".x", vec![TokenType::Dot, TokenType::Ident("x".into())]),
    ] {
        let joined = scan(left).join(&scan(right), JoinBoundary::Separate, LIMITS).unwrap();
        let actual = tokens(&joined).into_iter().map(|(kind, _, _)| kind).collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }
}

#[test]
fn macro_a0_04_unicode_names_use_bytes_not_scalar_counts() {
    let spelling = "café Éclair _中 %été $中 ~É1";
    let stream = scan(spelling);
    let expected = [
        TokenType::Ident("café".into()), TokenType::Type("Éclair".into()),
        TokenType::Ident("_中".into()), TokenType::MacroInvoc("été".into()),
        TokenType::MacroVar("中".into()), TokenType::NativeOperator("É1".into()),
    ];
    let actual = tokens(&stream);
    assert_eq!(actual.len(), expected.len());
    for ((kind, span, context), expected) in actual.iter().zip(expected) {
        assert_eq!(*kind, expected);
        assert!(spelling.is_char_boundary(span.start));
        assert!(spelling.is_char_boundary(span.end));
        assert_eq!(*context, Some(SyntaxContext::ROOT));
    }
    assert_eq!(actual[0].1.start..actual[0].1.end, 0..5);
    assert_eq!(&spelling[actual[5].1.start..actual[5].1.end], "~É1");
}

#[test]
fn macro_a0_04_unknown_characters_are_located_and_do_not_advance_the_view() {
    for character in ['🦀', '`', '\0'] {
        let source = scan(&format!("prefix {character} tail"));
        let captured = source.slice(7..7 + character.len_utf8(), LIMITS).unwrap();
        let mut view = RockView::new(&captured);
        for _ in 0..2 {
            assert!(matches!(
                view.next_piece(),
                Err(RockError::Lexer(LexerError::UnknownToken(actual, span)))
                    if actual == character && span.start == 7 && span.end == 7 + character.len_utf8(),
            ));
            assert_eq!(view.offset(), 0);
        }
        assert_eq!(captured.runs()[0].text(), character.to_string());
    }
}

#[test]
fn macro_a0_04_unmatched_groups_are_independent_tokens_not_global_errors() {
    let source = scan("([})");
    assert_eq!(
        tokens(&source).into_iter().map(|(kind, _, _)| kind).collect::<Vec<_>>(),
        vec![TokenType::OpenParen, TokenType::OpenBracket, TokenType::CloseBrace, TokenType::CloseParen],
    );
}

#[test]
fn macro_a0_05_failed_delimited_views_leave_the_source_untouched() {
    for spelling in ["\"open", "'open", "/* open"] {
        let source = scan(spelling);
        let mut view = RockView::new(&source);
        assert!(view.next_piece().is_err());
        assert_eq!(view.offset(), 0);
        assert_eq!(source.runs().iter().map(|run| run.text()).collect::<String>(), spelling);
    }
}

#[test]
fn macro_a0_04_trivia_is_visible_without_synthesized_layout_or_eof() {
    let source = scan("\t x\r\n");
    let mut view = RockView::new(&source);
    let first = view.next_piece().unwrap().unwrap();
    assert_eq!(first.kind, PieceKind::Whitespace);
    assert_eq!(first.range, 0..2);
    assert_eq!(view.next_piece().unwrap().unwrap().kind, PieceKind::Token(TokenType::Ident("x".into())));
    let ending = view.next_piece().unwrap().unwrap();
    assert_eq!(ending.kind, PieceKind::LineEnding);
    assert_eq!(ending.range, 3..5);
    assert!(view.next_piece().unwrap().is_none());
}

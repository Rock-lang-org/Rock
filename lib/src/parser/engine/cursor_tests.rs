use super::{ParseCtx, ParseError};
use crate::lexer::{Span, Token, TokenType};
use crate::Config;

fn tokens() -> Vec<Token> {
    vec![
        Token { token_type: TokenType::Ident("x".into()), span: Span::new("/virtual/cursor.rk".into(), 0, 1) },
        Token { token_type: TokenType::Ident("x".into()), span: Span::new("/virtual/cursor.rk".into(), 2, 3) },
        Token { token_type: TokenType::Eof, span: Span::new("/virtual/cursor.rk".into(), 3, 3) },
    ]
}

#[test]
fn macro_a0_08_bounded_prefix_keeps_a_boundary_eof_and_cannot_see_past_it() {
    let tokens = tokens();
    let config = Config::default();
    let original = ParseCtx::from(&tokens, &config);
    let bounded = original.prefix(1).unwrap();
    assert_eq!(bounded.len(), 1);
    assert_eq!(bounded.eof_span().start..bounded.eof_span().end, 2..2);
    assert!(matches!(bounded.seek_nth(1), Err(ParseError::UnexpectedEOF(span)) if span.start == 2));
    let (exhausted, token) = bounded.consume().unwrap();
    assert_eq!(token.span.start..token.span.end, 0..1);
    assert!(exhausted.is_empty());
    assert!(matches!(exhausted.seek(), Err(ParseError::UnexpectedEOF(span)) if span.start == 2));
    assert_eq!(original.len(), 3);
}

#[test]
fn macro_a0_08_checkpoint_identity_does_not_use_spelling_equality() {
    let tokens = tokens();
    let separate_tokens = tokens.clone();
    let config = Config::default();
    let original = ParseCtx::from(&tokens, &config);
    let checkpoint = original;
    assert!(original.same_remaining(&checkpoint));
    assert!(!original.same_remaining(&original.advance(1).unwrap()));
    assert!(!original.same_remaining(&ParseCtx::from(&separate_tokens, &config)));
    assert!(original.same_remaining(&checkpoint.advance(0).unwrap()));
}

#[test]
fn macro_a0_08_lookahead_at_or_beyond_the_limit_returns_eof_not_a_panic() {
    let tokens = tokens();
    let config = Config::default();
    let original = ParseCtx::from(&tokens, &config);
    for index in [original.len(), original.len() + 1, usize::MAX] {
        assert!(matches!(original.seek_nth(index), Err(ParseError::UnexpectedEOF(span)) if span.start == 3));
    }
    assert_eq!(original.seek_nth(1).unwrap().span.start..original.seek_nth(1).unwrap().span.end, 2..3);
    assert_eq!(original.len(), 3);
}

#[test]
fn macro_a0_08_invalid_prefix_and_advance_counts_do_not_change_input() {
    let tokens = tokens();
    let config = Config::default();
    let original = ParseCtx::from(&tokens, &config);
    for count in [4, usize::MAX] {
        assert!(matches!(original.prefix(count), Err(ParseError::UnexpectedEOF(_))));
        assert!(matches!(original.advance(count), Err(ParseError::UnexpectedEOF(_))));
    }
    let empty_prefix = original.prefix(0).unwrap();
    assert!(empty_prefix.is_empty());
    assert_eq!(empty_prefix.eof_span().start, 0);
    let empty_suffix = original.advance(original.len()).unwrap();
    assert!(empty_suffix.is_empty());
    assert_eq!(empty_suffix.eof_span().start, 3);
    assert_eq!(original.seek().unwrap().span.start, 0);
}

#[test]
fn macro_a0_08_checkpoint_restore_keeps_subparse_flags_and_restores_extent() {
    let tokens = tokens();
    let config = Config::default();
    let original = ParseCtx::from(&tokens, &config);
    let mut bounded = original.prefix(1).unwrap();
    bounded.inside_argument_list = true;
    bounded.after_closing_paren = true;
    let (exhausted, _) = bounded.consume().unwrap();
    let restored = exhausted.resume_input_from(original.advance(1).unwrap());
    assert!(restored.inside_argument_list);
    assert!(restored.after_closing_paren);
    assert_eq!(restored.eof_span(), original.eof_span());
    assert_eq!(restored.len(), 2);
    assert_eq!(restored.seek().unwrap().span.start, 2);
    let rollback = original;
    assert_eq!(rollback.len(), 3);
    assert!(!rollback.inside_argument_list);
}

#[test]
fn macro_a0_08_cursor_operations_preserve_pending_indentation_errors() {
    let mut tokens = tokens();
    tokens.insert(1, Token {
        token_type: TokenType::Indent(3),
        span: Span::new("/virtual/cursor.rk".into(), 1, 4),
    });
    let config = Config::default();
    let context = ParseCtx::from(&tokens, &config);
    assert!(matches!(context.seek_nth(0), Err(ParseError::UnexpectedIndent(3, span)) if span.start == 1));
    assert!(matches!(context.consume(), Err(ParseError::UnexpectedIndent(3, _))));
    assert!(matches!(context.advance(1), Err(ParseError::UnexpectedIndent(3, _))));
}

#[test]
fn macro_a0_08_negative_lookahead_can_reject_a_parser_that_consumes_to_eof() {
    use super::{not, Parser};

    let tokens = tokens();
    let config = Config::default();
    let context = ParseCtx::from(&tokens, &config).prefix(1).unwrap();
    let mut parser = not(TokenType::Ident("x".into()));
    assert!(matches!(parser.process(context), Err(ParseError::UnexpectedEOF(span)) if span.start == 2));
    assert_eq!(context.len(), 1);
}

#[test]
fn macro_a0_08_nested_repeat_arrays_resume_the_outer_cursor_correctly() {
    use crate::ast::{Expression, LiteralKind, Operand, Statement, TopLevel, UnaryExpr};

    let config = Config::default();
    let module = crate::parser::parse_source(
        "/virtual/cursor-array.rk".into(), "main = -> [[1; 2]; 3]\n", &config,
    ).unwrap();
    let [TopLevel::FunctionDecl(function)] = module.top_levels.as_slice() else {
        panic!("expected one function");
    };
    let [Statement::Expression(Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)))] =
        function.lambda.body.statements.as_slice()
    else { panic!("expected an array expression") };
    let Operand::Literal(literal) = &primary.operand else { panic!("expected a literal") };
    let LiteralKind::ArrayRepeat { value, len: 3 } = &literal.kind else { panic!("expected outer repeat") };
    let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = value.as_ref() else {
        panic!("expected an inner array expression");
    };
    let Operand::Literal(literal) = &primary.operand else { panic!("expected inner literal") };
    assert!(matches!(literal.kind, LiteralKind::ArrayRepeat { len: 2, .. }));
}

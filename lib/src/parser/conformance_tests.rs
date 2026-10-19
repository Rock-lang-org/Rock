//! Current Rock syntax contracts for the lossless-frontend migration (A0.01).
//! Assert token kinds and byte spans separately: token equality ignores spans.

use std::ops::Range;
use std::path::PathBuf;

use crate::ast::{
    Expression, FunctionDecl, Ident, IdentOrNumber, IdentOrType, LambdaArrowKind,
    LiteralKind, Module, Operand, ParseType, PrimaryExpr, SecondaryExpr, Statement,
    TopLevel, UnaryExpr,
};
use crate::lexer::{Lexer, LexerError, Span, TokenType};
use crate::parser::{parse_source, ParseError};
use crate::Config;

const SOURCE_PATH: &str = "/virtual/macro-a0-01.rk";

fn assert_span(span: &Span, range: Range<usize>) {
    assert_eq!(
        span,
        &Span::new(PathBuf::from(SOURCE_PATH), range.start, range.end),
    );
}

fn assert_tokens(source: &str, expected: &[(TokenType, Range<usize>)]) {
    // Disable parser-added terminal layout to describe only source tokens and EOF.
    let tokens = Lexer::new(PathBuf::from(SOURCE_PATH), source)
        .unwrap()
        .with_newline_at_end(false)
        .collect()
        .unwrap();
    assert_eq!(
        tokens.len(), expected.len(),
        "source: {source:?}; tokens: {tokens:?}",
    );
    for (index, (token, (kind, range))) in tokens.iter().zip(expected).enumerate() {
        assert_eq!(&token.token_type, kind, "source: {source:?}; token {index}");
        assert_span(&token.span, range.clone());
    }
}

fn parse(source: &str) -> Module {
    parse_source(PathBuf::from(SOURCE_PATH), source, &Config::default()).unwrap()
}

fn function<'a>(module: &'a Module, name: &str) -> &'a FunctionDecl {
    match module.top_level_from_ident(name).unwrap() {
        TopLevel::FunctionDecl(function) => function,
        other => panic!("expected function {name}, got {other:?}"),
    }
}

fn primary(expression: &Expression) -> &PrimaryExpr {
    match expression {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) => primary,
        other => panic!("expected a primary expression, got {other:?}"),
    }
}

fn identifier(expression: &Expression) -> &Ident {
    let Operand::Ident(path) = &primary(expression).operand else {
        panic!("expected an identifier, got {expression:?}");
    };
    let [IdentOrType::Ident(ident)] = path.path.as_slice() else {
        panic!("expected one path segment, got {path:?}");
    };
    ident
}

fn number(expression: &Expression) -> u64 {
    match &primary(expression).operand {
        Operand::Literal(literal) => match literal.kind {
            LiteralKind::Number(value) => value,
            ref other => panic!("expected a number, got {other:?}"),
        },
        other => panic!("expected a literal, got {other:?}"),
    }
}

#[test]
fn macro_a0_01_declaration_tokens_have_exact_byte_spans() {
    assert_tokens("struct Pair\n    value: I64 = 42\n", &[
        (TokenType::Indent(0), 0..0),
        (TokenType::Keyword("struct".into()), 0..6),
        (TokenType::Type("Pair".into()), 7..11),
        (TokenType::Eol, 11..12),
        (TokenType::Indent(4), 12..16),
        (TokenType::Ident("value".into()), 16..21),
        (TokenType::Colon, 21..22),
        (TokenType::Type("I64".into()), 23..26),
        (TokenType::Equal, 27..28),
        (TokenType::Number("42".into()), 29..31),
        (TokenType::Eol, 31..32),
        (TokenType::Eof, 32..32),
    ]);
}

#[test]
fn macro_a0_01_operator_spacing_preserves_adjacency() {
    for (source, operator, right, eol) in [
        ("a + b\n", TokenType::Operator("+".into()), 4..5, 5..6),
        ("a +b\n", TokenType::StuckOperator("+".into()), 3..4, 4..5),
        ("a ++ b\n", TokenType::Operator("++".into()), 5..6, 6..7),
        ("a ++b\n", TokenType::StuckOperator("++".into()), 4..5, 5..6),
    ] {
        let operator_end = 2 + if source.contains("++") { 2 } else { 1 };
        assert_tokens(source, &[
            (TokenType::Indent(0), 0..0),
            (TokenType::Ident("a".into()), 0..1),
            (operator, 2..operator_end),
            (TokenType::Ident("b".into()), right),
            (TokenType::Eol, eol),
            (TokenType::Eof, source.len()..source.len()),
        ]);
    }
}

#[test]
fn macro_a0_01_dot_spacing_selects_the_lexical_form() {
    for (source, dot, range, right, eol) in [
        ("a.b\n", TokenType::Dot, 1..2, 2..3, 3..4),
        ("a .b\n", TokenType::SpacedDot, 1..3, 3..4, 4..5),
        ("a . b\n", TokenType::Operator(".".into()), 2..3, 4..5, 5..6),
    ] {
        assert_tokens(source, &[
            (TokenType::Indent(0), 0..0),
            (TokenType::Ident("a".into()), 0..1),
            (dot, range),
            (TokenType::Ident("b".into()), right),
            (TokenType::Eol, eol),
            (TokenType::Eof, source.len()..source.len()),
        ]);
    }
}

#[test]
fn macro_a0_01_unicode_literals_retain_spelling_and_byte_ranges() {
    assert_tokens(r#""é\n\"🦀" '中' 7"#, &[
        (TokenType::Indent(0), 0..0),
        (TokenType::String(r#"é\n\"🦀"#.into()), 0..12),
        (TokenType::Char("中".into()), 13..18),
        (TokenType::Number("7".into()), 19..20),
        (TokenType::Eof, 20..20),
    ]);
}

#[test]
fn macro_a0_01_comments_keep_code_line_boundaries() {
    // All comments terminate; unterminated-comment recovery is not a baseline contract.
    assert_tokens("a // line\n    // only\n    b /* block */\nc\n", &[
        (TokenType::Indent(0), 0..0),
        (TokenType::Ident("a".into()), 0..1),
        (TokenType::Eol, 9..10),
        (TokenType::Indent(4), 22..26),
        (TokenType::Ident("b".into()), 26..27),
        (TokenType::Eol, 39..40),
        (TokenType::Indent(0), 40..40),
        (TokenType::Ident("c".into()), 40..41),
        (TokenType::Eol, 41..42),
        (TokenType::Eof, 42..42),
    ]);
}

#[test]
fn macro_a0_01_blank_lines_and_crlf_preserve_layout_locations() {
    assert_tokens("a\n \n    b\nc\n", &[
        (TokenType::Indent(0), 0..0),
        (TokenType::Ident("a".into()), 0..1),
        (TokenType::Eol, 1..2),
        (TokenType::Indent(0), 2..3),
        (TokenType::Eol, 3..4),
        (TokenType::Indent(4), 4..8),
        (TokenType::Ident("b".into()), 8..9),
        (TokenType::Eol, 9..10),
        (TokenType::Indent(0), 10..10),
        (TokenType::Ident("c".into()), 10..11),
        (TokenType::Eol, 11..12),
        (TokenType::Eof, 12..12),
    ]);
    assert_tokens("a\r\n    b\r\n", &[
        (TokenType::Indent(0), 0..0),
        (TokenType::Ident("a".into()), 0..1),
        (TokenType::Eol, 2..3),
        (TokenType::Indent(4), 3..7),
        (TokenType::Ident("b".into()), 7..8),
        (TokenType::Eol, 9..10),
        (TokenType::Eof, 10..10),
    ]);
}

#[test]
fn macro_a0_01_macro_intrinsic_and_arrow_tokens_remain_distinct() {
    assert_tokens("%m $x $(x)* ~I64Add -> !-> ~> =>\n", &[
        (TokenType::Indent(0), 0..0),
        (TokenType::MacroInvoc("m".into()), 0..2),
        (TokenType::MacroVar("x".into()), 3..5),
        (TokenType::MacroRepeatOpen, 6..8),
        (TokenType::Ident("x".into()), 8..9),
        (TokenType::MacroRepeatClose, 9..11),
        (TokenType::NativeOperator("I64Add".into()), 12..19),
        (TokenType::Arrow, 20..22),
        (TokenType::UnitArrow, 23..26),
        (TokenType::CurriedArrow, 27..29),
        (TokenType::FatArrow, 30..32),
        (TokenType::Eol, 32..33),
        (TokenType::Eof, 33..33),
    ]);
}

#[test]
fn macro_a0_01_layout_comments_and_nested_calls_preserve_ast_meaning() {
    let source = "main = !->\n    // comment\n    text = \"é🦀\"\n \n    render text, (combine 2, 3)\n";
    let module = parse(source);
    assert_eq!(module.filepath, Some(PathBuf::from(SOURCE_PATH)));
    assert_eq!(module.top_levels.len(), 1);
    let function = function(&module, "main");
    assert_span(&function.name.span, 0..4);
    assert_eq!(function.lambda.arrow_kind, LambdaArrowKind::Unit);
    assert_span(&function.lambda.span, 7..10);
    let [Statement::Assignment(assignment), Statement::Expression(call)] =
        function.lambda.body.statements.as_slice()
    else {
        panic!("expected an assignment and a call, got {:?}", function.lambda.body);
    };
    let Operand::Literal(literal) = &primary(&assignment.rhs).operand else {
        panic!("expected the string literal");
    };
    assert_eq!(literal.kind, LiteralKind::String("é🦀".into()));
    let start = source.find('"').unwrap();
    assert_span(&literal.span, start..start + "\"é🦀\"".len());
    assert_eq!(identifier(call).name, "render");
    let [SecondaryExpr::Arguments(arguments)] = primary(call).secondaries.as_deref().unwrap()
    else {
        panic!("expected one argument list");
    };
    assert_eq!(arguments.len(), 2);
    assert_eq!(identifier(&arguments[0].arg).name, "text");
    let Operand::Expression(nested) = &primary(&arguments[1].arg).operand else {
        panic!("expected explicit grouping around the nested call");
    };
    assert_eq!(identifier(nested).name, "combine");
    let [SecondaryExpr::Arguments(arguments)] = primary(nested).secondaries.as_deref().unwrap()
    else {
        panic!("expected the nested argument list");
    };
    assert_eq!(
        arguments.iter().map(|argument| number(&argument.arg)).collect::<Vec<_>>(),
        vec![2, 3],
    );
}

#[test]
fn macro_a0_01_spaced_dot_applies_to_the_call_not_its_argument() {
    for (source, spaced) in [
        ("main = -> render 1 .show!\n", true),
        ("main = -> render 1.show!\n", false),
    ] {
        let module = parse(source);
        let [Statement::Expression(call)] = function(&module, "main").lambda.body.statements.as_slice()
        else {
            panic!("expected one call");
        };
        assert_eq!(identifier(call).name, "render");
        let secondaries = primary(call).secondaries.as_deref().unwrap();
        assert_eq!(secondaries.len(), if spaced { 3 } else { 1 });
        let SecondaryExpr::Arguments(arguments) = &secondaries[0] else {
            panic!("expected call arguments");
        };
        assert_eq!(arguments.len(), 1);
        assert_eq!(number(&arguments[0].arg), 1);
        let members = if spaced {
            assert!(primary(&arguments[0].arg)
                .secondaries.as_deref().unwrap_or(&[]).is_empty());
            &secondaries[1..]
        } else {
            primary(&arguments[0].arg).secondaries.as_deref().unwrap()
        };
        let [SecondaryExpr::Dot(IdentOrNumber::Ident(member)), SecondaryExpr::Arguments(arguments)] = members
        else {
            panic!("expected a zero-argument member call, got {members:?}");
        };
        assert_eq!(member.name, "show");
        assert!(arguments.is_empty());
        let start = source.find("show").unwrap();
        assert_span(&member.span, start..start + 4);
    }
}

#[test]
fn macro_a0_01_dynamic_operator_syntax_has_no_builtin_meaning() {
    let source = "infix 7 %%\nmain = -> 1 %% 2\n";
    let module = parse(source);
    assert_eq!(module.top_levels.len(), 2);
    assert!(matches!(
        &module.top_levels[0],
        TopLevel::InfixOperator(7, symbol) if symbol == "%%",
    ));
    let [Statement::Expression(Expression::BinopExpr(left, operator, right))] =
        function(&module, "main").lambda.body.statements.as_slice()
    else {
        panic!("expected a binary expression");
    };
    assert_eq!(number(&Expression::UnaryExpr(left.clone())), 1);
    assert_eq!(operator.value, "%%");
    let start = source.rfind("%%").unwrap();
    assert_span(&operator.span, start..start + 2);
    assert_eq!(number(right), 2);
}

#[test]
fn macro_a0_01_struct_fields_keep_order_types_and_name_spans() {
    let source = "struct Pair\n    left: I64\n    right: &I64\n";
    let module = parse(source);
    let [TopLevel::StructDecl(declaration)] = module.top_levels.as_slice() else {
        panic!("expected one struct declaration");
    };
    assert_eq!(declaration.name.name, "Pair");
    assert_span(&declaration.name.span, 7..11);
    assert_eq!(declaration.fields.len(), 2);
    for (field, name) in declaration.fields.iter().zip(["left", "right"]) {
        assert_eq!(field.name.name, name);
        let start = source.find(name).unwrap();
        assert_span(&field.name.span, start..start + name.len());
        assert!(field.default.is_none());
    }
    assert!(matches!(&declaration.fields[0].ty, ParseType::Type(ty) if ty.name == "I64"));
    let ParseType::Reference { is_mut: false, pointee } = &declaration.fields[1].ty else {
        panic!("expected an immutable reference type");
    };
    assert!(matches!(pointee.as_ref(), ParseType::Type(ty) if ty.name == "I64"));
}

#[test]
fn macro_a0_01_unterminated_literals_report_the_original_byte_range() {
    for (source, quote) in [
        ("value = \"é🦀", '"'),
        ("value = '中", '\''),
        ("value = \"escaped\\\"", '"'),
    ] {
        let error = parse_source(PathBuf::from(SOURCE_PATH), source, &Config::default())
            .unwrap_err();
        let ParseError::Lexer(LexerError::UnknownToken(actual, span)) = error else {
            panic!("expected a structured literal error, got {error:?}");
        };
        assert_eq!(actual, quote);
        assert_span(&span, 8..source.len());
    }
}

#[test]
fn macro_a0_01_unknown_punctuation_is_a_located_lexer_error() {
    let error = parse_source(PathBuf::from(SOURCE_PATH), "value = `\n", &Config::default())
        .unwrap_err();
    let ParseError::Lexer(LexerError::UnknownToken('`', span)) = error else {
        panic!("expected a structured unknown-token error, got {error:?}");
    };
    assert_span(&span, 8..9);
}

#[test]
fn macro_a0_01_odd_indentation_reports_the_indentation_not_the_body() {
    for (source, level) in [("main = !->\n x\n", 1), ("main = !->\n   x\n", 3)] {
        let error = parse_source(PathBuf::from(SOURCE_PATH), source, &Config::default())
            .unwrap_err();
        let ParseError::UnexpectedIndent(actual, span) = error else {
            panic!("expected a structured indentation error, got {error:?}");
        };
        assert_eq!(actual, level);
        assert_span(&span, 11..11 + usize::from(level));
        assert_eq!(&source[span.start..span.end], " ".repeat(usize::from(level)));
    }
}

#[test]
fn macro_a0_01_final_line_is_parsed_with_or_without_a_newline() {
    for source in ["main = !-> 42", "main = !-> 42\n", "main = !-> 42\r\n"] {
        let module = parse(source);
        assert_eq!(module.top_levels.len(), 1);
        let function = function(&module, "main");
        assert_eq!(function.lambda.arrow_kind, LambdaArrowKind::Unit);
        let [Statement::Expression(expression)] = function.lambda.body.statements.as_slice() else {
            panic!("expected one literal expression");
        };
        assert_eq!(number(expression), 42);
        let Operand::Literal(literal) = &primary(expression).operand else {
            panic!("expected a literal");
        };
        assert_span(&literal.span, 11..13);
    }
}

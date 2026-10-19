use crate::ast::{Literal, LiteralKind};
use crate::fmt::{FormatContext, FormatNode};
use crate::lexer::Span;
use crate::parser::{lex_test, literal, ParseCtx, Parser};
use crate::Config;

/// Helper function to parse a literal from input string
pub fn parse_literal(input: &str) -> Literal {
    let tokens = lex_test(input);
    let config = Config::default();

    let (rest, lit) = literal.process(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(rest.len(), 0);

    lit
}

fn formatted<T: FormatNode>(node: &T) -> String {
    let mut context = FormatContext::new();
    let mut output = String::new();
    node.fmt_with(&mut context, &mut output)
        .expect("formatting into a String should not fail");
    output
}

/// Compare parser nodes by their semantic formatter output, excluding source spans.
pub fn assert_formatted_eq<T: FormatNode>(actual: &T, expected: &T) {
    assert_eq!(formatted(actual), formatted(expected));
}

/// Compare literal kinds through the formatter, which is implemented on `Literal`.
pub fn assert_formatted_literal_kind_eq(actual: &LiteralKind, expected: &LiteralKind) {
    let actual = Literal {
        kind: actual.clone(),
        span: Span::test(),
    };
    let expected = Literal {
        kind: expected.clone(),
        span: Span::test(),
    };
    assert_formatted_eq(&actual, &expected);
}

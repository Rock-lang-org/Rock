use crate::ast::{Ident, Open};
use crate::lexer::TokenType;
use crate::parser::engine::*;

use super::{block, disallow_multiline_fn_call, expression, get_span, ident};

/// Recognize the complete contextual header before consuming `open`.
/// The source slice prevents its delimiter `as` from becoming a cast suffix.
pub fn parse_open(stream: Input) -> IResult<Open> {
    let (after_name, name) = ident(stream)?;
    if name.name != "open" {
        return Err(ParseError::ShortCircuit);
    }
    let mut depth = 0usize;
    let mut delimiter = None;
    for (index, token) in after_name.tokens.iter().enumerate() {
        match &token.token_type {
            TokenType::OpenParen | TokenType::OpenBracket | TokenType::OpenBrace => depth += 1,
            TokenType::CloseParen | TokenType::CloseBracket | TokenType::CloseBrace => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            TokenType::Eol | TokenType::Eof if depth == 0 => break,
            TokenType::Keyword(word) if word == "as" && depth == 0 => {
                if matches!(
                    after_name.tokens.get(index + 1).map(|t| &t.token_type),
                    Some(TokenType::Type(_))
                ) && matches!(
                    after_name.tokens.get(index + 2).map(|t| &t.token_type),
                    Some(TokenType::Coma)
                ) {
                    delimiter = Some(index);
                    break;
                }
            }
            _ => {}
        }
    }
    let Some(delimiter) = delimiter else {
        return Err(ParseError::ShortCircuit);
    };
    let source_input = Input {
        tokens: &after_name.tokens[..delimiter],
        ..after_name
    };
    let (remaining, source) = disallow_multiline_fn_call(expression).process(source_input)?;
    if !remaining.is_empty() {
        return Err(ParseError::HardError(
            "invalid opening source expression".into(),
            remaining.seek()?.span,
        ));
    }
    let header = Input {
        tokens: &after_name.tokens[delimiter..],
        ..after_name
    };
    let (header, _) = TokenType::Keyword("as".into()).process(header)?;
    let (header, span) = get_span(header)?;
    let (header, token) = header.consume()?;
    let TokenType::Type(witness_name) = token.token_type else {
        return Err(ParseError::HardError(
            "expected opening type binder".into(),
            span,
        ));
    };
    let witness = Ident {
        name: witness_name,
        span: token.span,
    };
    let (header, _) = TokenType::Coma.process(header)?;
    let (header, value) = ident(header)?;
    // Open, unlike ordinary single-line block expressions, requires indentation.
    let (_, _) = seek(TokenType::Eol).process(header)?;
    let (rest, body) = block(header)?;
    Ok((
        rest,
        Open {
            source,
            witness,
            value,
            body,
            span: name.span,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Expression, Operand, UnaryExpr};
    use crate::parser::lex_test;
    use crate::Config;

    #[test]
    fn opening_header_keeps_source_casts_inside_parentheses() {
        let tokens = lex_test("open (object as &Read) as Hidden, value\n    value");
        let config = Config::default();
        let (rest, open) = parse_open(Input::from(&tokens, &config)).unwrap();
        assert!(rest.is_empty());
        assert_eq!(open.witness.name, "Hidden");
        assert_eq!(open.value.name, "value");
        let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = open.source else {
            panic!("expected parenthesized source");
        };
        let Operand::Expression(source) = primary.operand else {
            panic!("expected parenthesized cast");
        };
        assert!(matches!(*source, Expression::CastExpr(..)));
        assert_eq!(open.span, tokens[0].span);
    }

    #[test]
    fn ordinary_open_identifier_is_not_reserved() {
        let tokens = lex_test("open object");
        let config = Config::default();
        assert!(parse_open(Input::from(&tokens, &config)).is_err());
        let (rest, _) = expression(Input::from(&tokens, &config)).unwrap();
        assert!(rest.is_empty());
    }
}

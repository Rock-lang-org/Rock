//! Target cfg blocks.
//!
//! ```text
//! cfg(target_family = "unix")
//!     mod posix
//!     < posix::read_some
//! ```
//!
//! A cfg block is non-scoping: its active items are spliced into the
//! enclosing item list in source order. Every branch is parsed and its
//! predicate validated whatever the target, but inactive items never reach
//! the AST, so their child files, imports, macros and operators are never
//! loaded, resolved or registered.

use crate::{
    ast::TopLevel,
    lexer::{Span, Token, TokenType},
    parser::{
        engine::*,
        items::{primitives::indent, top_level::top_level, utils::empty_lines},
    },
    target::{validate_cfg_pair, Target},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CfgPredicate {
    Pair { key: String, value: String },
    All(Vec<CfgPredicate>),
    Any(Vec<CfgPredicate>),
    Not(Box<CfgPredicate>),
}

impl CfgPredicate {
    pub fn eval(&self, target: &Target) -> bool {
        match self {
            CfgPredicate::Pair { key, value } => target.cfg_value(key) == Some(value.as_str()),
            CfgPredicate::All(items) => items.iter().all(|item| item.eval(target)),
            CfgPredicate::Any(items) => items.iter().any(|item| item.eval(target)),
            CfgPredicate::Not(item) => !item.eval(target),
        }
    }
}

/// One module item: either an ordinary top-level item or the active items of
/// a cfg block (possibly none).
pub fn module_item(stream: Input) -> IResult<Vec<TopLevel>> {
    // Not via `or`: that would track the ordinary item's error a second time
    // and change which diagnostic wins.
    match cfg_block(stream) {
        Err(ParseError::Fail) => top_level(stream).map(|(rest, item)| (rest, vec![item])),
        other => other,
    }
}

fn is_cfg_head(stream: Input) -> bool {
    let Ok((rest, token)) = stream.consume() else {
        return false;
    };
    if token.token_type != TokenType::Ident("cfg".to_string()) {
        return false;
    }
    matches!(rest.consume(), Ok((_, next)) if next.token_type == TokenType::OpenParen)
}

fn hard(message: impl Into<String>, span: Span) -> ParseError {
    ParseError::HardError(message.into(), span)
}

fn expect<'a>(stream: Input<'a>, expected: TokenType, what: &str) -> IResult<'a, Token> {
    let (rest, token) = stream.consume()?;
    if token.token_type == expected {
        Ok((rest, token))
    } else {
        let message = format!("expected {} in cfg predicate", what);
        Err(hard(message, token.span))
    }
}

fn cfg_block(stream: Input) -> IResult<Vec<TopLevel>> {
    // Anything that is not a cfg head belongs to the ordinary item parser;
    // `Fail` keeps its diagnostics from being outranked by this attempt.
    let Ok((stream, _)) = empty_lines(stream) else {
        return Err(ParseError::Fail);
    };
    let Ok((stream, _)) = indent(stream) else {
        return Err(ParseError::Fail);
    };
    if !is_cfg_head(stream) {
        return Err(ParseError::Fail);
    }
    let (stream, head) = stream.consume()?;
    let (stream, _) = expect(stream, TokenType::OpenParen, "'('")?;
    let (stream, predicate) = predicate(stream)?;
    let (stream, _) = expect(stream, TokenType::CloseParen, "')'")?;
    let (stream, _) = expect(stream, TokenType::Eol, "end of line after cfg(...)")?;

    let (stream, groups) = match indented(many(module_item)).process(stream) {
        Ok(result) => result,
        Err(err @ ParseError::HardError(..)) => return Err(err),
        Err(_) => (stream, Vec::new()),
    };
    if groups.is_empty() {
        return Err(hard(
            "a cfg block needs at least one indented module item",
            head.span,
        ));
    }
    let (stream, _) = empty_lines(stream)?;

    let items = if predicate.eval(&stream.config.target()) {
        groups.into_iter().flatten().collect()
    } else {
        Vec::new()
    };
    Ok((stream, items))
}

fn predicate(stream: Input) -> IResult<CfgPredicate> {
    let (stream, name) = stream.consume()?;
    let TokenType::Ident(key) = name.token_type else {
        return Err(hard(
            "expected a cfg key or all(...), any(...), not(...)",
            name.span,
        ));
    };

    let (after, next) = stream.consume()?;
    match next.token_type {
        TokenType::Equal => {
            let (after, value) = after.consume()?;
            let TokenType::String(value) = value.token_type else {
                return Err(hard("expected a string value in cfg predicate", value.span));
            };
            validate_cfg_pair(&key, &value).map_err(|message| hard(message, name.span))?;
            Ok((after, CfgPredicate::Pair { key, value }))
        }
        TokenType::OpenParen => {
            let (after, operands) = operand_list(after)?;
            let predicate = match key.as_str() {
                "all" | "any" if operands.is_empty() => {
                    return Err(hard(
                        format!("{}(...) needs at least one operand", key),
                        name.span,
                    ));
                }
                "all" => CfgPredicate::All(operands),
                "any" => CfgPredicate::Any(operands),
                "not" if operands.len() == 1 => {
                    CfgPredicate::Not(Box::new(operands.into_iter().next().expect("one operand")))
                }
                "not" => return Err(hard("not(...) takes exactly one operand", name.span)),
                _ => {
                    let message =
                        format!("unknown cfg combinator '{}'; expected all, any or not", key);
                    return Err(hard(message, name.span));
                }
            };
            Ok((after, predicate))
        }
        _ => Err(hard(
            format!("expected '= \"value\"' after cfg key '{}'", key),
            next.span,
        )),
    }
}

fn operand_list(mut stream: Input) -> IResult<Vec<CfgPredicate>> {
    let mut operands = Vec::new();
    loop {
        let (after, token) = stream.consume()?;
        if token.token_type == TokenType::CloseParen {
            return Ok((after, operands));
        }
        let (after, operand) = predicate(stream)?;
        operands.push(operand);
        let (rest, token) = after.consume()?;
        match token.token_type {
            TokenType::Coma => stream = rest,
            TokenType::CloseParen => return Ok((rest, operands)),
            _ => return Err(hard("expected ',' or ')' in cfg predicate", token.span)),
        }
    }
}

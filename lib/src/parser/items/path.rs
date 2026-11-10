use crate::parser::{
    engine::*, separated1, IdentOrType, IdentifierPath, Path, TokenType, TypePath,
};

use super::{get_span, ident, parse_type, parse_type_path_head};

/// `(Vec I64)::new`-style heads. Numbers stay expression arguments here: `(Maybe::Some 7)` is a
/// call, not a type application with a literal argument.
fn parenthesized_path_type(stream: Input) -> IResult<crate::ast::ParseType> {
    let (stream, _) = TokenType::OpenParen.process(stream)?;
    let mut inner_stream = stream;
    inner_stream.no_literal_type_args = true;
    inner_stream.literal_type_arg_after_comma = true;
    let (mut stream, ty) = parse_type(inner_stream)?;
    stream.no_literal_type_args = false;
    stream.literal_type_arg_after_comma = false;
    let (stream, _) = TokenType::CloseParen.process(stream)?;
    Ok((stream, ty))
}

fn path_type(stream: Input) -> IResult<crate::ast::ParseType> {
    (
        get_span,
        TokenType::OpenParen,
        get_span,
        TokenType::CloseParen,
    )
        .map(|(open_span, _, close_span, _)| {
            crate::ast::ParseType::Unit(crate::lexer::Span::new(
                open_span.file_path,
                open_span.start,
                close_span.end,
            ))
        })
        .or(parenthesized_path_type)
        .or(parse_type_path_head)
        .process(stream)
}

fn is_named_path_segment(segment: &IdentOrType) -> bool {
    matches!(
        segment,
        IdentOrType::Ident(_)
            | IdentOrType::Type(crate::ast::ParseType::Type(_))
            | IdentOrType::Type(crate::ast::ParseType::Application(_))
    )
}

pub fn path(stream: Input) -> IResult<Path> {
    ident_path
        .map(Path::Ident)
        .or(type_path.map(Path::Type))
        .process(stream)
}

pub fn ident_path(stream: Input) -> IResult<IdentifierPath> {
    let (stream, path) = separated1(ident_or_type, TokenType::DoubleColon)
        .map(|idents| IdentifierPath { path: idents })
        .process(stream)?;

    if path.path.len() > 1 && !path.path.iter().all(is_named_path_segment) {
        return Err(ParseError::Fail);
    }

    if let Some(last) = path.path.last() {
        if let IdentOrType::Ident(_) = last {
            return Ok((stream, path));
        } else {
            return Err(ParseError::Fail);
        }
    }

    Err(ParseError::Fail)
}

// the same, but finished with a type
pub fn type_path(stream: Input) -> IResult<TypePath> {
    let (stream, path) = separated1(ident_or_type, TokenType::DoubleColon)
        .map(|idents| TypePath { path: idents })
        .process(stream)?;

    if path.path.len() > 1 && !path.path.iter().all(is_named_path_segment) {
        return Err(ParseError::Fail);
    }

    if let Some(last) = path.path.last() {
        if let IdentOrType::Type(_) = last {
            return Ok((stream, path));
        } else {
            return Err(ParseError::Fail);
        }
    }

    Err(ParseError::Fail)
}

pub fn ident_or_type(stream: Input) -> IResult<IdentOrType> {
    ident
        .map(IdentOrType::Ident)
        .or(path_type.map(IdentOrType::Type))
        .process(stream)
}

use crate::ast::{DeriveAttachment, DeriveTarget, DerivedDecl, Ident, IdentOrType, IdentifierPath, InvocationId, NodeId, TopLevel};
use crate::lexer::TokenType;
use crate::parser::engine::*;

use super::macro_decl::required_macro_input;

pub(super) fn is_directive(stream: Input) -> Result<bool, ParseError> {
    Ok(matches!(stream.seek()?.token_type, TokenType::MacroInvoc(ref name) if name == "derive")
        && !matches!(stream.seek_nth(1)?.token_type, TokenType::DoubleColon))
}

pub(super) fn named_segment(stream: Input) -> IResult<Ident> {
    let (rest, token) = stream.consume()?;
    match token.token_type {
        TokenType::Ident(name) | TokenType::Type(name) => Ok((rest, Ident { name, context: token.context, span: token.span })),
        _ => Err(ParseError::UnexpectedToken("derive name or path segment".into(), token)),
    }
}

fn attachment(stream: Input) -> IResult<DeriveAttachment> {
    let (mut rest, name) = named_segment(stream)?;
    let mut span = name.span.clone();
    let mut path = vec![IdentOrType::Ident(name)];
    while matches!(rest.seek()?.token_type, TokenType::DoubleColon) {
        let (next, _) = rest.consume()?;
        let (next, name) = named_segment(next).map_err(|error| required_macro_input(error, "expected derive path segment after '::'", span.clone()))?;
        span.end = name.span.end;
        path.push(IdentOrType::Ident(name));
        rest = next;
    }
    let id = InvocationId::fresh().map_err(|message| ParseError::HardError(message.into(), span.clone()))?;
    Ok((rest, DeriveAttachment { id, parent: stream.expansion_parent, path: IdentifierPath { path }, span }))
}

pub(super) fn derived_decl(mut stream: Input) -> IResult<TopLevel> {
    let mut derives = Vec::new();
    let mut seen = std::collections::HashSet::new();
    loop {
        let (next, _) = super::indent(stream)?;
        let (mut next, directive) = next.consume()?;
        let directive_span = directive.span;
        loop {
            let (rest, item) = attachment(next).map_err(|error| required_macro_input(error, "%derive expects a nonempty comma-separated path list", directive_span.clone()))?;
            let key = crate::collect::resolver::binding_path(&item.path.path).unwrap_or_default();
            seen.try_reserve(1).map_err(|_| ParseError::HardError("cannot allocate derive path set".into(), item.span.clone()))?;
            if !seen.insert(key) {
                return Err(ParseError::HardError("duplicate derive path on one declaration".into(), item.span));
            }
            derives.try_reserve(1).map_err(|_| ParseError::HardError("cannot allocate derive attachments".into(), item.span.clone()))?;
            derives.push(item);
            next = rest;
            match next.seek()?.token_type {
                TokenType::Coma => {
                    next = next.consume()?.0;
                    if matches!(next.seek()?.token_type, TokenType::Eol) { break; }
                }
                TokenType::Eol => break,
                _ => return Err(ParseError::HardError("expected ',' or newline in %derive list".into(), next.seek()?.span)),
            }
        }
        stream = TokenType::Eol.process(next)?.0;
        stream = super::empty_lines(stream)?.0;
        let (target, _) = super::indent(stream).map_err(|error| required_macro_input(error, "%derive target must be in the same scope", directive_span.clone()))?;
        if is_directive(target)? { continue; }
        let mut head = target;
        if matches!(head.seek()?.token_type, TokenType::Operator(ref op) if op == "<") { head = head.consume()?.0; }
        match head.seek()?.token_type {
            TokenType::Keyword(ref keyword) if keyword == "struct" || keyword == "enum" => {}
            TokenType::Ident(ref name) if name == "lang" => {}
            _ => return Err(ParseError::HardError("%derive must be followed by a struct or enum without intervening items".into(), directive_span)),
        }
        // Parse the target independently so original input excludes directives.
        let (rest, item) = super::top_level(stream)?;
        let target = match item {
            TopLevel::StructDecl(node) => DeriveTarget::Struct(node),
            TopLevel::EnumDecl(node) => DeriveTarget::Enum(node),
            _ => return Err(ParseError::HardError("%derive supports only structs and enums".into(), directive_span)),
        };
        let id = NodeId::fresh().map_err(|message| ParseError::HardError(message.into(), directive_span))?;
        return Ok((rest, TopLevel::DerivedDecl(DerivedDecl { id, derives, target })));
    }
}

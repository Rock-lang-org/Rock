use crate::{
    lexer::{Token, TokenType},
    parser::{engine::*, Ident, MacroDecl, MacroEntry, MacroFragment, MacroInvoc, MacroLexeme},
};

use super::{consume_tokens_until, empty_lines, get_span, ident, indent, macro_invoc_token};

pub fn macro_decl(stream: Input) -> IResult<MacroDecl> {
    let (stream, (_, name, _, entries)) = (
        TokenType::Keyword("macro".to_string()),
        macro_name,
        TokenType::Eol,
        indented(many(macro_entry)),
    )
        .process(stream)?;
    let id = crate::ast::NodeId::fresh()
        .map_err(|message| ParseError::HardError(message.into(), name.span.clone()))?;
    Ok((stream, MacroDecl { id, name, entries }))
}

fn macro_name(stream: Input) -> IResult<crate::ast::Ident> {
    let (stream, name) = ident(stream)?;
    if name.name == "derive" {
        return Err(ParseError::HardError("'derive' is reserved for derive macro declarations".into(), name.span));
    }
    Ok((stream, name))
}

pub fn proc_macro_decl(stream: Input) -> IResult<crate::ast::ProcMacroDecl> {
    let (stream, _) = TokenType::Keyword("macro".into()).process(stream)?;
    if matches!(stream.seek()?.token_type, TokenType::Ident(ref name) if name == "derive") {
        let marker = stream.seek()?.span;
        let (next, _) = stream.consume()?;
        let (next, name) = super::derive::named_segment(next).map_err(|error| required_macro_input(error, "'derive' is reserved for derive macro declarations; expected a derive name", marker))?;
        let (next, _) = TokenType::Equal.process(next).map_err(|error| required_macro_input(error, "expected '=' in derive macro declaration", name.span.clone()))?;
        let (next, lambda) = super::lambda_decl(next)?;
        let (next, _) = TokenType::Eol.process(next)?;
        let id = crate::ast::NodeId::fresh().map_err(|message| ParseError::HardError(message.into(), name.span.clone()))?;
        let function = crate::ast::FunctionDecl { name, lambda, self_receiver: None, self_context: Default::default(), is_unsafe: false, exported: false };
        return Ok((next, crate::ast::ProcMacroDecl { id, kind: crate::macro_expansion::proc_macro::ProcMacroKind::DeriveLike, function }));
    }
    // Check the name before the shared ordinary function parser, so reserved
    // derive syntax receives its own diagnostic instead of an expected '='.
    macro_name(stream)?;
    let (stream, function) = super::function_decl(stream)?;
    let id = crate::ast::NodeId::fresh()
        .map_err(|message| ParseError::HardError(message.into(), function.name.span.clone()))?;
    Ok((stream, crate::ast::ProcMacroDecl { id, kind: crate::macro_expansion::proc_macro::ProcMacroKind::FunctionLike, function }))
}

pub fn macro_entry(stream: Input) -> IResult<MacroEntry> {
    let start = stream;
    let (stream, (_, _, defs, _, _, body)) = (
        empty_lines,
        indent,
        parse_macro_head_recursive,
        TokenType::FatArrow,
        TokenType::Eol,
        parse_macro_block_recursive,
    )
        .process(stream)?;
    let input = start.lossless_until(stream)?;
    Ok((stream, MacroEntry { input, defs, body }))
}

pub fn macro_invoc(stream: Input) -> IResult<MacroInvoc> {
    let header_indent = stream.header_indent()?;
    let (stream, (span, path)) = macro_path(stream)?;
    let (stream, args, literal_ranges, block) = if let Some((stream, captured)) = stream.capture_indented_block()? {
        let (stream, _) = TokenType::Eol.process(stream)?;
        (stream, captured.input, Vec::new(), Some(crate::ast::MacroBlockInput {
            header_indent: captured.header_indent, body_indent: captured.body_indent,
        }))
    } else {
        let start = stream;
        let (stream, _) = consume_tokens_until(TokenType::Indent(0)).process(stream)?;
        (stream, start.lossless_until(stream)?, start.literal_ranges_until(stream)?, None)
    };
    let id = crate::ast::InvocationId::fresh()
        .map_err(|message| ParseError::HardError(message.into(), span.clone()))?;
    Ok((stream, MacroInvoc {
            header_indent,
            id,
            parent: stream.expansion_parent,
            output: crate::ast::MacroOutputCategory::Items,
            path,
            span,
            args,
            arguments: Vec::new(),
            literal_ranges,
            block,
        }))
}

pub(super) fn macro_path(stream: Input) -> IResult<(crate::lexer::Span, crate::ast::IdentifierPath)> {
    let context = stream.seek()?.context;
    let (mut stream, (mut span, name)) = (get_span, macro_invoc_token).process(stream)?;
    let mut path = vec![crate::ast::IdentOrType::Ident(Ident { name, span: span.clone(), context })];
    while matches!(stream.seek().ok().map(|token| token.token_type), Some(TokenType::DoubleColon)) {
        let (next, _) = TokenType::DoubleColon.process(stream)?;
        let (next, part) = super::derive::named_segment(next).or_else(|_| ident(next))
            .map_err(|error| required_macro_input(error, "expected macro path segment after '::'", next.eof_span()))?;
        span.end = part.span.end;
        path.push(crate::ast::IdentOrType::Ident(part));
        stream = next;
    }
    if path.len() == 1 && matches!(&path[0], crate::ast::IdentOrType::Ident(ident) if ident.name == "derive") {
        return Err(ParseError::HardError("%derive must precede a struct or enum, not occur as an expression".into(), span));
    }
    Ok((stream, (span, crate::ast::IdentifierPath { path })))
}

pub fn macro_expression(stream: Input) -> IResult<MacroInvoc> {
    let header_indent = stream.header_indent()?;
    let (start, (span, path)) = macro_path(stream)?;
    if let Some((stream, captured)) = start.capture_indented_block()? {
        let id = crate::ast::InvocationId::fresh().map_err(|message| ParseError::HardError(message.into(), span.clone()))?;
        return Ok((stream, MacroInvoc {
            header_indent,
            id, parent: stream.expansion_parent, output: crate::ast::MacroOutputCategory::Expression,
            path, span, args: captured.input, arguments: Vec::new(),
            literal_ranges: Vec::new(),
            block: Some(crate::ast::MacroBlockInput {
                header_indent: captured.header_indent, body_indent: captured.body_indent,
            }),
        }));
    }
    if matches!(start.seek(), Ok(token) if token.token_type == TokenType::Eol) {
        return Err(ParseError::HardError("bare expression macro requires an indented input block; use '!' for no arguments".into(), span));
    }
    let (stream, parsed) = super::expression::call_arguments(start)
        .map_err(|error| required_macro_input(error, "expected ordinary macro call arguments or '!'", span.clone()))?;
    let args = if parsed.is_empty() { stream.lossless_until(stream)? } else { start.lossless_until(stream)? };
    let literal_ranges = if parsed.is_empty() { Vec::new() } else { start.literal_ranges_until(stream)? };
    let offset = start.source_offset()?.ok_or_else(|| ParseError::HardError("macro calls require retained source".into(), span.clone()))?;
    let arguments = parsed.into_iter().map(|argument| {
        let range = argument.range.ok_or_else(|| ParseError::HardError("macro argument has no retained source range".into(), span.clone()))?;
        let start = range.start.checked_sub(offset);
        let end = range.end.checked_sub(offset);
        match (start, end) {
            (Some(start), Some(end)) if start <= end && end <= args.len_bytes() => Ok(start..end),
            _ => Err(ParseError::HardError("invalid macro argument range".into(), span.clone())),
        }
    }).collect::<Result<Vec<_>, _>>()?;
    let id = crate::ast::InvocationId::fresh().map_err(|message| ParseError::HardError(message.into(), span.clone()))?;
    Ok((stream, MacroInvoc { header_indent, id, parent: stream.expansion_parent, output: crate::ast::MacroOutputCategory::Expression, path, span, args, arguments, literal_ranges, block: None }))
}

pub(super) fn required_macro_input(error: ParseError, message: &str, span: crate::lexer::Span) -> ParseError {
    match error.without_source_context() {
        error @ (ParseError::Lexer(_) | ParseError::UnexpectedIndent(_, _) | ParseError::HardError(_, _)) => error,
        _ => ParseError::HardError(message.into(), span),
    }
}

fn incomplete_macro_head(message: &str, token: &Token) -> ParseError {
    ParseError::HardError(
        format!("Incomplete macro head: {}", message),
        token.span.clone(),
    )
}

fn parse_macro_head_recursive<'a>(
    mut stream: Input<'a>,
) -> IResult<'a, Vec<MacroFragment>> {
    let mut defs = Vec::new();

    while !stream.is_empty() {
        let token = stream.seek()?;
        match &token.token_type {
            TokenType::MacroVar(name) => {
                let Ok(colon_token) = stream.seek_nth(1) else {
                    return Err(incomplete_macro_head(
                        "expected ':' after macro variable",
                        &token,
                    ));
                };
                if matches!(colon_token.token_type, TokenType::Eof | TokenType::Eol) {
                    return Err(incomplete_macro_head(
                        "expected ':' after macro variable",
                        &colon_token,
                    ));
                }
                if colon_token.token_type != TokenType::Colon {
                    return Err(ParseError::UnexpectedToken(
                        TokenType::Colon.discriminant().to_string(),
                        colon_token,
                        // vec![TokenType::Colon],
                    ));
                }
                let Ok(kind_token) = stream.seek_nth(2) else {
                    return Err(incomplete_macro_head("expected macro fragment kind", &token));
                };
                if matches!(kind_token.token_type, TokenType::Eof | TokenType::Eol) {
                    return Err(incomplete_macro_head(
                        "expected macro fragment kind",
                        &kind_token,
                    ));
                }

                if kind_token.token_type == TokenType::Ident("ident".to_string()) {
                    defs.push(MacroFragment::Ident(Ident {
                        context: token.context,
                        name: name.clone(),
                        span: token.span.clone(),
                    }));
                } else if kind_token.token_type == TokenType::Ident("expr".to_string()) {
                    defs.push(MacroFragment::Expr(Ident {
                        context: token.context,
                        name: name.clone(),
                        span: token.span.clone(),
                    }));
                } else if kind_token.token_type == TokenType::Ident("ty".to_string()) {
                    defs.push(MacroFragment::Type(Ident {
                        context: token.context,
                        name: name.clone(),
                        span: token.span.clone(),
                    }));
                } else {
                    return Err(ParseError::UnexpectedToken(
                        TokenType::Ident(String::new()).discriminant().to_string(),
                        kind_token,
                        /* vec![
                            TokenType::Ident("ident".to_string()),
                            TokenType::Ident("expr".to_string()),
                            TokenType::Ident("type".to_string()),
                        ], */
                    ));
                }

                stream = stream.advance(3)?;

                continue;
            }
            TokenType::MacroRepeatOpen => {
                let (remaining, inner_block) =
                    parse_macro_head_recursive(stream.advance(1)?)?;

                stream = remaining;
                defs.push(MacroFragment::Repetition(inner_block));
            }
            TokenType::MacroRepeatClose => {
                return Ok((stream.advance(1)?, defs));
            }
            TokenType::FatArrow => {
                return Ok((stream, defs));
            }
            _ => {
                let next = stream.advance(1)?;
                defs.push(MacroFragment::Token(MacroLexeme { token, input: stream.lossless_until(next)? }));
                stream = next;
            }
        }
    }

    Ok((stream, defs))
}

fn parse_macro_block_recursive<'a>(
    mut stream: Input<'a>,
) -> IResult<'a, Vec<MacroFragment>> {
    let mut block = Vec::new();

    while !stream.is_empty() {
        let token = stream.seek()?;
        match &token.token_type {
            TokenType::MacroVar(name) => {
                let next = stream.advance(1)?;
                let input = stream.lossless_until(next)?;
                let prefix = input.len_bytes().checked_sub(token.span.end - token.span.start)
                    .ok_or_else(|| ParseError::HardError("invalid macro variable extent".into(), token.span.clone()))?;
                if prefix > 0 && !block.is_empty() {
                    block.push(MacroFragment::Trivia(input.slice(0..prefix, super::super::SOURCE_LIMITS)
                        .map_err(|error| ParseError::HardError(format!("invalid macro trivia: {error:?}"), token.span.clone()))?));
                }
                let ident = MacroFragment::Ident(Ident {
                    context: token.context,
                    name: name.clone(),
                    span: token.span.clone(),
                });
                block.push(ident);
                stream = next;
            }
            TokenType::MacroRepeatOpen => {
                let (remaining, inner_block) =
                    parse_macro_block_recursive(stream.advance(1)?)?;

                stream = remaining;
                block.push(MacroFragment::Repetition(inner_block));
            }
            TokenType::MacroRepeatClose => {
                return Ok((stream.advance(1)?, block));
            }
            _ => {
                let mut token = token;

                // Rebase relative to this declaration's module, not a fixed
                // root-level margin; nested modules retain the same bytes/maps.
                if let TokenType::Indent(level) = token.token_type {
                    if usize::from(level) <= stream.indent_level {
                        // the definition is over
                        return Ok((stream, block));
                    }
                    let Some(base_indent) = stream.indent_level.checked_add(stream.indent_step()?)
                        .and_then(|width| u8::try_from(width).ok()) else {
                        return Err(ParseError::UnexpectedIndent(level, token.span.clone()));
                    };
                    let Some(adjusted_indent) = level.checked_sub(base_indent) else {
                        return Err(ParseError::UnexpectedIndent(level, token.span.clone()));
                    };
                    token.token_type = TokenType::Indent(adjusted_indent);
                }

                let next = stream.advance(1)?;
                let mut input = stream.lossless_until(next)?;
                if let TokenType::Indent(_) = token.token_type {
                    let base_indent = stream.indent_level.checked_add(stream.indent_step()?)
                        .ok_or_else(|| ParseError::UnexpectedIndent(255, token.span.clone()))?;
                    input = input.slice(base_indent..input.len_bytes(), super::super::SOURCE_LIMITS)
                        .map_err(|error| ParseError::HardError(format!("invalid macro margin: {error:?}"), token.span.clone()))?;
                }

                block.push(MacroFragment::Token(MacroLexeme { token, input }));
                stream = next;
            }
        }
    }

    Ok((stream, block))
}

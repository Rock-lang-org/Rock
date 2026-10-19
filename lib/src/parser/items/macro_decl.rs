use crate::{
    lexer::{Token, TokenType},
    parser::{engine::*, Ident, MacroDecl, MacroEntry, MacroFragment, MacroInvoc, MacroLexeme},
};

use super::{consume_tokens_until, empty_lines, get_span, ident, indent, macro_invoc_token};

pub fn macro_decl(stream: Input) -> IResult<MacroDecl> {
    (
        TokenType::Keyword("macro".to_string()),
        ident,
        TokenType::Eol,
        indented(many(macro_entry)),
    )
        .map(|(_, name, _, entries)| MacroDecl { name, entries })
        .process(stream)
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
    let (stream, (span, name)) = (get_span, macro_invoc_token).process(stream)?;
    let start = stream;
    let (stream, _) = consume_tokens_until(TokenType::Indent(0)).process(stream)?;
    let args = start.lossless_until(stream)?;
    let id = crate::ast::InvocationId::fresh()
        .map_err(|message| ParseError::HardError(message.into(), span.clone()))?;
    Ok((stream, MacroInvoc {
            id,
            parent: None,
            output: crate::ast::MacroOutputCategory::Items,
            name: Ident { name, span },
            args,
        }))
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
                        name: name.clone(),
                        span: token.span.clone(),
                    }));
                } else if kind_token.token_type == TokenType::Ident("expr".to_string()) {
                    defs.push(MacroFragment::Expr(Ident {
                        name: name.clone(),
                        span: token.span.clone(),
                    }));
                } else if kind_token.token_type == TokenType::Ident("ty".to_string()) {
                    defs.push(MacroFragment::Type(Ident {
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

                // fix the indentation for the parser
                if let TokenType::Indent(level) = token.token_type {
                    if level == 0 || level == stream.indent_step()? as u8 {
                        // the definition is over
                        return Ok((stream, block));
                    }
                    if level % 2 != 0 {
                        return Err(ParseError::UnexpectedIndent(level, token.span.clone()));
                    }
                    let Some(base_indent) = (stream.indent_step()? as u8).checked_mul(2) else {
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
                    let base_indent = stream.indent_step()?.checked_mul(2)
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

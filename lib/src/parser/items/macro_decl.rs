use crate::{
    lexer::{Token, TokenType},
    parser::{engine::*, Ident, MacroDecl, MacroEntry, MacroFragment, MacroInvoc},
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
    let (stream, (_, _, defs, _, _, mut body)) = (
        empty_lines,
        indent,
        parse_macro_head_recursive,
        TokenType::FatArrow,
        TokenType::Eol,
        parse_macro_block_recursive,
    )
        .process(stream)?;
    let boundary_span = stream
        .seek()
        .map(|token| token.span)
        .unwrap_or_else(|_| stream.eof_span());
    body.push(MacroFragment::Token(Token {
        token_type: TokenType::Eof,
        span: boundary_span,
    }));

    Ok((stream, MacroEntry { defs, body }))
}

pub fn macro_invoc(stream: Input) -> IResult<MacroInvoc> {
    (
        get_span,
        macro_invoc_token,
        consume_tokens_until(TokenType::Indent(0)),
    )
        .map(|(span, name, args)| MacroInvoc {
            name: Ident { name, span },
            args: args
                .iter()
                .filter(|t| {
                    t.token_type != TokenType::Eol
                        && if let TokenType::Indent(_) = t.token_type {
                            false
                        } else {
                            true
                        }
                })
                .cloned()
                .collect::<Vec<_>>(),
        })
        .process(stream)
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
                if colon_token.token_type == TokenType::Eof {
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
                if kind_token.token_type == TokenType::Eof {
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
                defs.push(MacroFragment::Token(token));
                stream = stream.advance(1)?;
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
                let ident = MacroFragment::Ident(Ident {
                    name: name.clone(),
                    span: token.span.clone(),
                });
                block.push(ident);
                stream = stream.advance(1)?;
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
                    if level == 0 || level == stream.indent_step as u8 {
                        // the definition is over
                        return Ok((stream, block));
                    }
                    if level % 2 != 0 {
                        return Err(ParseError::UnexpectedIndent(level, token.span.clone()));
                    }
                    let Some(base_indent) = (stream.indent_step as u8).checked_mul(2) else {
                        return Err(ParseError::UnexpectedIndent(level, token.span.clone()));
                    };
                    let Some(adjusted_indent) = level.checked_sub(base_indent) else {
                        return Err(ParseError::UnexpectedIndent(level, token.span.clone()));
                    };
                    token.token_type = TokenType::Indent(adjusted_indent);
                }

                block.push(MacroFragment::Token(token));
                stream = stream.advance(1)?;
            }
        }
    }

    Ok((stream, block))
}

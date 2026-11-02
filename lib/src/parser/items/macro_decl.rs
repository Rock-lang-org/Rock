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

/// Parses `%name tokens...` in expression position. The arguments run to the
/// end of the line, or to the first closing delimiter that has no matching
/// opener inside the arguments, so `(%name a, b).method!` works.
pub fn macro_invoc_expr(stream: Input) -> IResult<MacroInvoc> {
    let (stream, (span, name)) = (get_span, macro_invoc_token).process(stream)?;

    let mut depth = 0usize;
    let mut end = 0;
    for token in stream.tokens {
        match token.token_type {
            TokenType::OpenParen | TokenType::OpenBracket | TokenType::OpenBrace => depth += 1,
            TokenType::CloseParen
            | TokenType::CloseBracket
            | TokenType::CloseBrace
            | TokenType::MacroRepeatClose => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            TokenType::Eol | TokenType::Eof | TokenType::Indent(_) if depth == 0 => break,
            _ => {}
        }
        end += 1;
    }

    let args = stream.tokens[..end]
        .iter()
        .filter(|token| !matches!(token.token_type, TokenType::Eol | TokenType::Indent(_)))
        .cloned()
        .collect();
    let stream = Input {
        tokens: &stream.tokens[end..],
        ..stream
    };

    Ok((
        stream,
        MacroInvoc {
            name: Ident { name, span },
            args,
        },
    ))
}

fn parse_macro_head_recursive(stream: Input<'_>) -> IResult<'_, Vec<MacroFragment>> {
    let (defs, tokens) = parse_macro_head_recursive_inner(stream.tokens, stream, None)?;

    Ok((Input { tokens, ..stream }, defs))
}

fn parse_macro_block_recursive(stream: Input<'_>) -> IResult<'_, Vec<MacroFragment>> {
    let (block, tokens) = parse_macro_block_recursive_inner(stream.tokens, stream, None)?;

    Ok((Input { tokens, ..stream }, block))
}

/// A repetition opened by `$(` must be closed by `)*` before its matcher or
/// template ends; a plain `)` is an ordinary token inside the repetition.
fn unclosed_macro_repetition(open: &Token) -> ParseError {
    ParseError::HardError(
        "Unclosed macro repetition: expected `)*` to close this `$(`".to_string(),
        open.span.clone(),
    )
}

fn unexpected_macro_repetition_close(close: &Token) -> ParseError {
    ParseError::HardError(
        "Unexpected `)*` outside of a macro repetition".to_string(),
        close.span.clone(),
    )
}

fn incomplete_macro_head(message: &str, token: &Token) -> ParseError {
    ParseError::HardError(
        format!("Incomplete macro head: {}", message),
        token.span.clone(),
    )
}

fn parse_macro_head_recursive_inner<'a>(
    tokens: &'a [Token],
    parse_ctx: ParseCtx,
    repetition_open: Option<&Token>,
) -> Result<(Vec<MacroFragment>, &'a [Token]), ParseError> {
    let mut defs = Vec::new();
    let mut remaining_tokens = tokens;

    let mut skip_until = 0;
    while let Some(token) = remaining_tokens.first() {
        if skip_until > 0 {
            skip_until -= 1;
            remaining_tokens = &remaining_tokens[1..];
            continue;
        }
        match &token.token_type {
            TokenType::MacroVar(name) => {
                let Some(colon_token) = remaining_tokens.get(1) else {
                    return Err(incomplete_macro_head(
                        "expected ':' after macro variable",
                        token,
                    ));
                };
                if colon_token.token_type == TokenType::Eof {
                    return Err(incomplete_macro_head(
                        "expected ':' after macro variable",
                        colon_token,
                    ));
                }
                if colon_token.token_type != TokenType::Colon {
                    return Err(ParseError::UnexpectedToken(
                        TokenType::Colon.discriminant().to_string(),
                        colon_token.clone(),
                        // vec![TokenType::Colon],
                    ));
                }
                let Some(kind_token) = remaining_tokens.get(2) else {
                    return Err(incomplete_macro_head("expected macro fragment kind", token));
                };
                if kind_token.token_type == TokenType::Eof {
                    return Err(incomplete_macro_head(
                        "expected macro fragment kind",
                        kind_token,
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
                        kind_token.clone(),
                        /* vec![
                            TokenType::Ident("ident".to_string()),
                            TokenType::Ident("expr".to_string()),
                            TokenType::Ident("type".to_string()),
                        ], */
                    ));
                }

                skip_until = 3;

                continue;
            }
            TokenType::MacroRepeatOpen => {
                let (inner_block, new_remaining_tokens) = parse_macro_head_recursive_inner(
                    &remaining_tokens[1..],
                    parse_ctx,
                    Some(token),
                )?;

                remaining_tokens = new_remaining_tokens;
                defs.push(MacroFragment::Repetition(inner_block));
            }
            TokenType::MacroRepeatClose => {
                if repetition_open.is_none() {
                    return Err(unexpected_macro_repetition_close(token));
                }
                return Ok((defs, &remaining_tokens[1..]));
            }
            TokenType::FatArrow => {
                if let Some(open) = repetition_open {
                    return Err(unclosed_macro_repetition(open));
                }
                return Ok((defs, remaining_tokens));
            }
            _ => {
                defs.push(MacroFragment::Token(token.clone()));
                remaining_tokens = &remaining_tokens[1..];
            }
        }
    }

    if let Some(open) = repetition_open {
        return Err(unclosed_macro_repetition(open));
    }

    Ok((defs, remaining_tokens))
}

fn parse_macro_block_recursive_inner<'a>(
    tokens: &'a [Token],
    parse_ctx: ParseCtx,
    repetition_open: Option<&Token>,
) -> Result<(Vec<MacroFragment>, &'a [Token]), ParseError> {
    let mut block = Vec::new();
    let mut remaining_tokens = tokens;

    while let Some(token) = remaining_tokens.first() {
        match &token.token_type {
            TokenType::MacroVar(name) => {
                let ident = MacroFragment::Ident(Ident {
                    name: name.clone(),
                    span: token.span.clone(),
                });
                block.push(ident);
                remaining_tokens = &remaining_tokens[1..];
            }
            TokenType::MacroRepeatOpen => {
                let (inner_block, new_remaining_tokens) = parse_macro_block_recursive_inner(
                    &remaining_tokens[1..],
                    parse_ctx,
                    Some(token),
                )?;

                remaining_tokens = new_remaining_tokens;
                block.push(MacroFragment::Repetition(inner_block));
            }
            TokenType::MacroRepeatClose => {
                if repetition_open.is_none() {
                    return Err(unexpected_macro_repetition_close(token));
                }
                return Ok((block, &remaining_tokens[1..]));
            }
            _ => {
                let mut token = token.clone();

                // fix the indentation for the parser
                if let TokenType::Indent(level) = token.token_type {
                    if level == 0 || level == parse_ctx.indent_step as u8 {
                        // the definition is over
                        if let Some(open) = repetition_open {
                            return Err(unclosed_macro_repetition(open));
                        }
                        return Ok((block, remaining_tokens));
                    }
                    if level % 2 != 0 {
                        return Err(ParseError::UnexpectedIndent(level, token.span.clone()));
                    }
                    let Some(base_indent) = (parse_ctx.indent_step as u8).checked_mul(2) else {
                        return Err(ParseError::UnexpectedIndent(level, token.span.clone()));
                    };
                    let Some(adjusted_indent) = level.checked_sub(base_indent) else {
                        return Err(ParseError::UnexpectedIndent(level, token.span.clone()));
                    };
                    token.token_type = TokenType::Indent(adjusted_indent);
                }

                block.push(MacroFragment::Token(token));
                remaining_tokens = &remaining_tokens[1..];
            }
        }
    }

    if let Some(open) = repetition_open {
        return Err(unclosed_macro_repetition(open));
    }

    Ok((block, remaining_tokens))
}

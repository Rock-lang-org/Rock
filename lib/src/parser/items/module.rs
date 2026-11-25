use crate::{lexer::TokenType, parser::*};

use super::cfg::module_item;

pub fn module_inline(stream: Input) -> IResult<Module> {
    let mut current_stream = stream;
    let mut top_levels = Vec::new();
    let mut deepest_error: Option<ParseError> = None;

    // Manually parse top-level items to track the deepest error
    loop {
        if current_stream.is_empty() {
            break;
        }

        // Check if we've reached EOF
        if let Ok((_, token)) = current_stream.consume() {
            if token.token_type == TokenType::Eof {
                break;
            }
        }

        match module_item(current_stream) {
            Ok((new_stream, items)) => {
                current_stream = new_stream;
                top_levels.extend(items);
            }
            Err(err) => {
                // Track the deepest error
                deepest_error = Some(match deepest_error {
                    Some(prev_err) => prev_err.choose_better(err),
                    None => err,
                });
                break;
            }
        }
    }

    // If we have leftover tokens (not EOF), report the deepest error we found
    if !current_stream.is_empty() {
        if let Ok((_, token)) = current_stream.consume() {
            if token.token_type != TokenType::Eof {
                // If we have a deepest error from parsing attempts, use it
                if let Some(err) = deepest_error {
                    return Err(err);
                }
                // Otherwise, create a generic error
                return Err(ParseError::UnexpectedToken(
                    "top-level declaration (function, struct, enum, trait, impl, etc.)".to_string(),
                    token,
                ));
            }
        }
    }

    // Consume the EOF token
    let (stream, _) = TokenType::Eof.process(current_stream)?;

    Ok((
        stream,
        Module {
            name: None,
            top_levels,
            is_inline: true,
            filepath: None,
        },
    ))
}

pub fn module(stream: Input) -> IResult<Module> {
    (
        TokenType::Keyword("mod".to_string()),
        ident,
        TokenType::Eol,
        indented(many(module_item)),
    )
        .map(|(_, name, _, groups)| Module {
            name: Some(name),
            top_levels: groups.into_iter().flatten().collect(),
            is_inline: false,
            filepath: None,
        })
        .process(stream)
}

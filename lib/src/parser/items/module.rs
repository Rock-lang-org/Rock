use crate::{lexer::TokenType, parser::*};

use super::top_level;

pub fn module_inline(stream: Input) -> IResult<Module> {
    let mut current_stream = stream;
    let mut top_levels = Vec::new();
    let mut deepest_error: Option<ParseError> = None;

    // Manually parse top-level items to track the deepest error
    loop {
        let (rest, _) = super::empty_lines.process(current_stream)?;
        current_stream = rest;
        if current_stream.is_empty() {
            break;
        }

        // Check if we've reached EOF
        if let Ok((_, token)) = current_stream.consume() {
            if token.token_type == TokenType::Eof {
                break;
            }
        }

        match top_level(current_stream) {
            Ok((new_stream, top_level_item)) => {
                current_stream = new_stream;
                top_levels.push(top_level_item);
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
        indented(many1(top_level)),
    )
        .map(|(_, name, _, top_levels)| Module {
            name: Some(name),
            top_levels,
            is_inline: false,
            filepath: None,
        })
        .process(stream)
}

#[cfg(test)]
mod tests {
    use crate::{ast::TopLevel, parser::parse_string, Config};

    #[test]
    fn macro_a_08_inline_modules_do_not_commit_as_empty_file_headers() {
        let parsed = parse_string("mod file\nmod outer\n    value = -> 42\n", &Config::default()).unwrap();
        assert!(matches!(&parsed.module.top_levels[0], TopLevel::Mod(name, false) if name.name == "file"));
        let TopLevel::Module(outer) = &parsed.module.top_levels[1] else { panic!("expected inline module") };
        assert!(outer.0.top_level_from_ident("value").is_some());
    }
}

pub mod engine;
mod items;

use std::path::PathBuf;

pub use engine::*;
pub use items::*;

use crate::ast::tree::*;
use crate::lexer::{Lexer, TokenType};
use crate::{Config, DebugPrint};

pub fn parse_source(
    file_path: PathBuf,
    source: &str,
    config: &Config,
) -> Result<Module, ParseError> {
    let file_path_for_result = file_path.clone();
    let mut lexer = Lexer::new(file_path, source).map_err(ParseError::Lexer)?;
    let tokens = lexer.collect().map_err(ParseError::Lexer)?;

    if config.has_debug_print(DebugPrint::Tokens) {
        println!("{:#?}", tokens);
    }

    // Reset the best error tracker at the start of parsing
    engine::reset_best_error();

    let ctx = ParseCtx::from(&tokens, config);
    let result = module_inline.process(ctx);

    match result {
        Ok((_, mut program)) => {
            // Set the filepath on the module
            program.filepath = Some(file_path_for_result);
            Ok(program)
        }
        Err(e) => {
            // Return the best error we've seen during parsing
            Err(engine::get_best_error(e))
        }
    }
}

pub fn parse_module_tokens(
    tokens: &[crate::lexer::Token],
    config: &Config,
) -> Result<Module, ParseError> {
    engine::reset_best_error();
    let result = module_inline.process(ParseCtx::from(tokens, config));
    match result {
        Ok((_, module)) => Ok(module),
        Err(e) => Err(engine::get_best_error(e)),
    }
}

/// Parses tokens that must form exactly one expression, such as the output of
/// a macro called in expression position.
pub fn parse_expression_tokens(
    tokens: &[crate::lexer::Token],
    config: &Config,
) -> Result<Expression, ParseError> {
    engine::reset_best_error();
    match expression.process(ParseCtx::from(tokens, config)) {
        Ok((rest, expr)) => match rest.tokens.first() {
            None => Ok(expr),
            Some(token) if token.token_type == TokenType::Eof => Ok(expr),
            Some(token) => Err(ParseError::HardError(
                "Expected the macro expansion to be a single expression".to_string(),
                token.span.clone(),
            )),
        },
        Err(e) => Err(engine::get_best_error(e)),
    }
}

pub fn parse_string(input: &str, config: &Config) -> Result<Program, ParseError> {
    parse_source(PathBuf::from("<memory>"), input, config).map(|mut module| {
        module.filepath = None;
        Program { module }
    })
}

#[cfg(test)]
mod tests;

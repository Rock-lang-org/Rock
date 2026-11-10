use crate::{
    lexer::{Span, Token, TokenType},
    Config,
};

mod and;
mod delimited;
mod fns;
mod followed;
mod indented;
mod iresult;
mod many;
mod map;
mod not;
mod opt;
mod or;
mod parse_error;
mod parser_trait;
mod preceded;
mod separated;
mod token_type;
mod tuples;

#[cfg(test)]
pub(crate) mod tests;

pub use and::*;
pub use delimited::*;
pub use followed::*;
pub use indented::*;
pub use iresult::*;
pub use many::*;
pub use map::*;
pub use not::*;
pub use opt::*;
pub use or::*;
pub use parse_error::*;
pub use parser_trait::*;
pub use preceded::*;
pub use separated::*;

pub type Input<'a> = ParseCtx<'a>;

use std::cell::RefCell;

thread_local! {
    /// Track the best (furthest) error seen during parsing
    /// This is reset at the start of each parse and updated as parsing progresses
    static BEST_ERROR: RefCell<Option<ParseError>> = RefCell::new(None);
}

/// Track an error if it's better than the current best error
pub fn track_error(error: &ParseError) {
    BEST_ERROR.with(|best| {
        let mut best = best.borrow_mut();
        *best = Some(match best.take() {
            Some(prev) => prev.choose_better(error.clone()),
            None => error.clone(),
        });
    });
}

/// Get the best error seen so far, or return the given error if no better error exists
pub fn get_best_error(fallback: ParseError) -> ParseError {
    BEST_ERROR.with(|best| {
        best.borrow()
            .as_ref()
            .map(|e| e.clone().choose_better(fallback.clone()))
            .unwrap_or(fallback)
    })
}

/// Reset the best error tracker (should be called at the start of each parse)
pub fn reset_best_error() {
    BEST_ERROR.with(|best| {
        *best.borrow_mut() = None;
    });
}

#[derive(Clone, Debug)]
pub struct ParseCtx<'a> {
    pub tokens: &'a [Token],
    pub eof_location: Option<(&'a std::path::PathBuf, usize)>,
    pub indent_level: usize,
    pub config: &'a Config,
    pub indent_step: usize,
    // Borrowed from the token stream so parser contexts remain cheap Copy values;
    // ParseError takes an owned clone when the error escapes the parser.
    pub invalid_indent: Option<(u8, &'a Span)>,
    pub disallowed_multiline_fn_call: bool,
    pub inside_argument_list: bool,
    pub inside_inline_argument_list: bool, // For comma-separated args, not multiline
    pub after_closing_paren: bool, // Set after parsing a closing paren in an argument context
    pub after_multiline_dot: bool,
    pub is_inside_fn_type_decl: bool,
    pub disallow_multiline_operators: bool, // Disallow multiline operator continuations (e.g., in single-line function bodies)
    pub no_literal_type_args: bool, // Types parsed where an expression could also appear (cast targets, parenthesized path heads): numbers are never literal type arguments
}

impl ParseCtx<'_> {
    pub fn eof_span(&self) -> Span {
        self.eof_location
            .map(|(path, position)| Span {
                file_path: path.clone(),
                start: position,
                end: position,
            })
            .expect("parser input must retain its EOF source location")
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    pub fn consume(&self) -> Result<(Self, Token), ParseError> {
        if let Some((level, span)) = self.invalid_indent {
            return Err(ParseError::UnexpectedIndent(level, span.clone()));
        }

        if self.tokens.is_empty() {
            return Err(ParseError::UnexpectedEOF(self.eof_span()));
        }

        Ok((
            ParseCtx {
                tokens: &self.tokens[1..],
                ..*self
            },
            self.tokens[0].clone(),
        ))
    }

    pub fn indent(&self) -> Result<Self, ParseError> {
        if let Some((level, span)) = self.invalid_indent {
            return Err(ParseError::UnexpectedIndent(level, span.clone()));
        }

        Ok(ParseCtx {
            indent_level: self.indent_level + self.indent_step,
            ..*self
        })
    }

    pub fn dedent(&self) -> Result<Self, ParseError> {
        if let Some((level, span)) = self.invalid_indent {
            return Err(ParseError::UnexpectedIndent(level, span.clone()));
        }

        Ok(ParseCtx {
            indent_level: self.indent_level - self.indent_step,
            ..*self
        })
    }

    pub fn with_indent<'a, F, T>(self, f: F) -> IResult<'a, T>
    where
        F: FnOnce(Self) -> IResult<'a, T>,
    {
        let new_ctx = self.indent()?;
        let (new_ctx, res) = f(new_ctx)?;
        let new_ctx = new_ctx.dedent()?;

        Ok((new_ctx, res))
    }

    pub fn seek(&self) -> Result<Token, ParseError> {
        if let Some((level, span)) = self.invalid_indent {
            return Err(ParseError::UnexpectedIndent(level, span.clone()));
        }

        if self.tokens.is_empty() {
            return Err(ParseError::UnexpectedEOF(self.eof_span()));
        }

        Ok(self.tokens[0].clone())
    }

    pub fn seek_nth(&self, n: usize) -> Result<Token, ParseError> {
        if let Some((level, span)) = self.invalid_indent {
            return Err(ParseError::UnexpectedIndent(level, span.clone()));
        }

        if self.tokens.len() < n {
            return Err(ParseError::UnexpectedEOF(self.eof_span()));
        }

        Ok(self.tokens[n].clone())
    }

    pub fn from<'a>(tokens: &'a [Token], config: &'a Config) -> ParseCtx<'a> {
        let (mut indent_step, invalid_indent) = Self::determine_indent_step(tokens, config);

        if indent_step == 0 {
            indent_step = 4;
        }

        ParseCtx {
            tokens,
            eof_location: tokens
                .last()
                .map(|token| (&token.span.file_path, token.span.end)),
            indent_level: 0,
            indent_step,
            invalid_indent,
            config,
            disallowed_multiline_fn_call: false,
            inside_argument_list: false,
            inside_inline_argument_list: false,
            after_closing_paren: false,
            after_multiline_dot: false,
            is_inside_fn_type_decl: false,
            disallow_multiline_operators: false,
            no_literal_type_args: false,
        }
    }

    fn determine_indent_step<'a>(
        tokens: &'a [Token],
        _config: &Config,
    ) -> (usize, Option<(u8, &'a Span)>) {
        let mut indent_step = 0;
        let mut invalid_indent = None;

        for token in tokens {
            if let TokenType::Indent(level) = token.token_type {
                if level > 0 {
                    if level % 2 != 0 {
                        invalid_indent = Some((level, &token.span));
                        break;
                    }

                    if indent_step == 0 {
                        indent_step = level as usize;
                    }
                }
            }
        }

        (indent_step, invalid_indent)
    }

    pub fn disallow_multiline_fn_call(&self) -> Result<Self, ParseError> {
        Ok(ParseCtx {
            disallowed_multiline_fn_call: true,
            ..*self
        })
    }

    pub fn argument_list_short_circuit(&mut self) -> Result<(), ParseError> {
        if !self.inside_argument_list {
            return Ok(());
        }

        self.inside_argument_list = false;

        Err(ParseError::ShortCircuit)
    }
}

impl Copy for ParseCtx<'_> {}

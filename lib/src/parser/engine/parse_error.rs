use crate::lexer::{Span, Token};

#[derive(Debug, Clone)]
pub enum ParseError {
    UnexpectedToken(String, Token), // expected, got
    /// End-of-file location is a zero-width span at the source byte length.
    UnexpectedEOF(Span),
    UnknownFile(String),
    Lexer(crate::lexer::LexerError),
    /// The indentation token that caused the mismatch is retained verbatim.
    UnexpectedIndent(u8, Span),
    /// The location where the parser attempted to start the required item.
    ExpectedOneOrMore(Span),
    MacroNoCorrespondance {
        macro_name: Span,
        invoc_name: Span,
        invoc_arg: Option<Span>,
    },
    // Used to short-circuit the parser. These are internal control flow and
    // must never be rendered as source diagnostics.
    Fail,
    ShortCircuit, // should not be bubbled up to the user
    AssertFailed,
    /// Hard error: not recoverable, bypasses `or` alternatives.
    HardError(String, crate::lexer::Span),
    // Wraps an error with context about what was being parsed
    WithContext {
        context: String,
        error: Box<ParseError>,
    },
}

impl ParseError {
    pub fn discriminant(&self) -> &'static str {
        match self {
            ParseError::UnexpectedToken(_, _) => "UnexpectedToken",
            ParseError::UnexpectedEOF(_) => "UnexpectedEOF",
            ParseError::UnknownFile(_) => "UnknownFile",
            ParseError::Lexer(_) => "Lexer",
            ParseError::UnexpectedIndent(_, _) => "UnexpectedIndent",
            ParseError::ExpectedOneOrMore(_) => "ExpectedOneOrMore",
            ParseError::MacroNoCorrespondance { .. } => "MacroNoCorrespondance",
            ParseError::Fail => "Fail",
            ParseError::ShortCircuit => "ShortCircuit",
            ParseError::AssertFailed => "AssertFailed",
            ParseError::HardError(_, _) => "HardError",
            ParseError::WithContext { .. } => "WithContext",
        }
    }

    /// Get the position (span) of this error for comparison purposes.
    /// Returns the start position of the error's span.
    /// Errors that occurred later in the input are considered "better" to report.
    pub fn position(&self) -> usize {
        match self {
            ParseError::UnexpectedToken(_, token) => token.span.start,
            ParseError::UnexpectedEOF(_) => usize::MAX, // EOF errors are always at the end
            ParseError::MacroNoCorrespondance { invoc_name, .. } => invoc_name.start,
            ParseError::UnexpectedIndent(_, span) => span.start,
            ParseError::ExpectedOneOrMore(span) => span.start,
            ParseError::UnknownFile(_) => 0,
            ParseError::Lexer(crate::lexer::LexerError::UnknownToken(_, span)) => span.start,
            ParseError::Fail => 0,
            ParseError::ShortCircuit => 0,
            ParseError::AssertFailed => 0,
            ParseError::HardError(_, span) => span.start,
            ParseError::WithContext { error, .. } => error.position(),
        }
    }

    /// Add context to this error about what was being parsed
    pub fn with_context(self, context: impl Into<String>) -> ParseError {
        ParseError::WithContext {
            context: context.into(),
            error: Box::new(self),
        }
    }

    /// Get all context strings from this error and its nested errors
    pub fn get_context_chain(&self) -> Vec<String> {
        match self {
            ParseError::WithContext { context, error } => {
                let mut chain = vec![context.clone()];
                chain.extend(error.get_context_chain());
                chain
            }
            _ => vec![],
        }
    }

    /// Get the depth of context (number of WithContext wrappers)
    fn context_depth(&self) -> usize {
        match self {
            ParseError::WithContext { error, .. } => 1 + error.context_depth(),
            _ => 0,
        }
    }

    /// Determine if this error makes sense given the context.
    /// Some errors are clearly wrong (e.g., expecting 'if' when we got ')').
    /// Returns a score: higher is better (more sensible).
    fn sensibility_score(&self) -> i32 {
        match self {
            ParseError::UnexpectedToken(expected, got) => {
                use crate::lexer::TokenType;

                // If we're expecting a keyword but got a structural token (paren, bracket, etc.),
                // this is likely a wrong alternative being tried
                if expected.contains("keyword") {
                    match &got.token_type {
                        TokenType::CloseParen
                        | TokenType::CloseBracket
                        | TokenType::OpenParen
                        | TokenType::OpenBracket
                        | TokenType::Coma
                        | TokenType::Colon
                        | TokenType::Arrow
                        | TokenType::UnitArrow
                        | TokenType::FatArrow
                        | TokenType::Dot => {
                            return -10; // Very unlikely to be the right error
                        }
                        _ => {}
                    }
                }

                // If we're expecting an opening bracket/paren but got a closing one,
                // this is also likely wrong
                if expected.contains("[") || expected.contains("(") {
                    match &got.token_type {
                        TokenType::CloseParen | TokenType::CloseBracket => {
                            return -5; // Unlikely to be the right error
                        }
                        _ => {}
                    }
                }

                // If we're expecting an arrow but got a closing paren/bracket,
                // this is likely wrong (trying to parse a lambda when it's not)
                if expected.contains("->") {
                    match &got.token_type {
                        TokenType::CloseParen | TokenType::CloseBracket => {
                            return -8; // Unlikely to be the right error
                        }
                        _ => {}
                    }
                }

                0 // Default score
            }
            ParseError::WithContext { error, .. } => error.sensibility_score(),
            _ => 0,
        }
    }

    /// Choose the "better" error to report between two errors.
    /// Prefers errors that occurred later in the input, but if positions are equal,
    /// prefers errors with more context (deeper in the parsing tree).
    pub fn choose_better(self, other: ParseError) -> ParseError {
        if self.prefers(&other) {
            other
        } else {
            self
        }
    }

    /// Whether `other` is the better error to report: later in the input,
    /// then more sensible, then with more context. Internal control-flow
    /// errors (`ShortCircuit`, `Fail`) are never preferred; hard errors are.
    pub fn prefers(&self, other: &ParseError) -> bool {
        match (self, other) {
            (ParseError::HardError(_, _), _) => return false,
            (_, ParseError::HardError(_, _)) => return true,
            (ParseError::ShortCircuit, _) => return true,
            (_, ParseError::ShortCircuit) => return false,
            (ParseError::Fail, _) => return true,
            (_, ParseError::Fail) => return false,
            _ => {}
        }
        let (self_pos, other_pos) = (self.position(), other.position());
        if self_pos != other_pos {
            return other_pos > self_pos;
        }
        let (self_sensibility, other_sensibility) =
            (self.sensibility_score(), other.sensibility_score());
        if self_sensibility != other_sensibility {
            return other_sensibility > self_sensibility;
        }
        other.context_depth() > self.context_depth()
    }
}

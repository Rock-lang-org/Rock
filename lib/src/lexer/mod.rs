mod lexer;
pub mod groups;
pub mod interpretation;
pub mod lossless;
pub(crate) mod literal;
mod span;
mod token;

#[cfg(test)]
mod syntax_library_tests;

pub use lexer::LexerError;
#[cfg(test)]
mod test_support;
#[cfg(test)]
pub use lexer::Lexer;
pub use span::Span;
pub use token::{Token, TokenType};

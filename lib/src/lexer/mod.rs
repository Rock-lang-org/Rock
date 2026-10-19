mod lexer;
pub mod interpretation;
pub mod lossless;
mod span;
mod token;

pub use lexer::{Lexer, LexerError};
pub use span::Span;
pub use token::{Token, TokenType};

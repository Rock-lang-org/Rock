use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};

use crate::lexer::Span;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    pub token_type: TokenType,
    pub span: Span,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub enum TokenType {
    Ident(String),
    Type(String),
    Number(String),
    Float(String),
    Operator(String),
    Comment(String),
    //Operator that is not followed by a space
    StuckOperator(String),
    NativeOperator(String),
    Keyword(String),
    MacroVar(String),
    MacroInvoc(String),
    MacroRepeatOpen,
    MacroRepeatClose,
    Equal,
    OpenParen,
    CloseParen,
    OpenBracket,
    CloseBracket,
    OpenBrace,
    CloseBrace,
    Char(String),
    String(String),
    Arrow,
    CurriedArrow,
    TypeLambda,
    FatArrow,
    Coma,
    Colon,
    DoubleColon,
    Dot,
    DoubleDot,
    SpacedDot,
    Caret,
    Tilde,
    Arobase,
    Interogation,
    Ampersand,
    Indent(u8),
    Underscore,
    Eol,
    Eof,
    UnitArrow,
    DoubleDotEqual,
}

impl ToString for TokenType {
    fn to_string(&self) -> String {
        match self {
            TokenType::Ident(s) => s.clone(),
            TokenType::Type(s) => s.clone(),
            TokenType::Number(s) => s.clone(),
            TokenType::Float(s) => s.clone(),
            TokenType::Operator(s) => s.clone(),
            TokenType::Comment(s) => s.clone(),
            TokenType::StuckOperator(s) => s.clone(),
            TokenType::NativeOperator(s) => s.clone(),
            TokenType::Keyword(s) => s.clone(),
            TokenType::MacroVar(s) => s.clone(),
            TokenType::MacroInvoc(s) => s.clone(),
            TokenType::MacroRepeatOpen => "$(".to_string(),
            TokenType::MacroRepeatClose => ")".to_string(),
            TokenType::Equal => "=".to_string(),
            TokenType::OpenParen => "(".to_string(),
            TokenType::CloseParen => ")".to_string(),
            TokenType::OpenBracket => "[".to_string(),
            TokenType::CloseBracket => "]".to_string(),
            TokenType::OpenBrace => "{".to_string(),
            TokenType::CloseBrace => "}".to_string(),
            TokenType::Char(c) => format!("'{}'", c),
            TokenType::String(s) => format!("\"{}\"", s),
            TokenType::Arrow => "->".to_string(),
            TokenType::UnitArrow => "!->".to_string(),
            TokenType::CurriedArrow => "~>".to_string(),
            TokenType::TypeLambda => "\\".to_string(),
            TokenType::FatArrow => "=>".to_string(),
            TokenType::Coma => ",".to_string(),
            TokenType::Colon => ":".to_string(),
            TokenType::DoubleColon => "::".to_string(),
            TokenType::Dot => ".".to_string(),
            TokenType::DoubleDot => "..".to_string(),
            TokenType::DoubleDotEqual => "..=".to_string(),
            TokenType::SpacedDot => " .".to_string(),
            TokenType::Caret => "^".to_string(),
            TokenType::Tilde => "~".to_string(),
            TokenType::Arobase => "@".to_string(),
            TokenType::Interogation => "?".to_string(),
            TokenType::Ampersand => "&".to_string(),
            TokenType::Indent(i) => " ".repeat(*i as usize),
            TokenType::Underscore => "_".to_string(),
            TokenType::Eol => "\n".to_string(),
            TokenType::Eof => "".to_string(),
        }
    }
}

impl PartialEq for Token {
    fn eq(&self, other: &Self) -> bool {
        self.token_type == other.token_type
    }
}

impl Eq for Token {}

#[cfg(test)]
impl From<TokenType> for Token {
    fn from(token_type: TokenType) -> Self {
        Self {
            token_type,
            span: Span::test(),
        }
    }
}

impl Display for Token {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.token_type.to_string())
    }
}

impl TokenType {
    pub fn discriminant(&self) -> &'static str {
        match self {
            TokenType::Ident(_) => "Ident",
            TokenType::Type(_) => "Type",
            TokenType::Number(_) => "Number",
            TokenType::Float(_) => "Float",
            TokenType::Operator(_) => "Operator",
            TokenType::Comment(_) => "Comment",
            TokenType::StuckOperator(_) => "StuckOperator",
            TokenType::NativeOperator(_) => "NativeOperator",
            TokenType::Keyword(_) => "Keyword",
            TokenType::MacroVar(_) => "MacroVar",
            TokenType::MacroInvoc(_) => "MacroInvoc",
            TokenType::MacroRepeatOpen => "MacroRepeatOpen",
            TokenType::MacroRepeatClose => "MacroRepeatClose",
            TokenType::Equal => "Equal",
            TokenType::OpenParen => "OpenParen",
            TokenType::CloseParen => "CloseParen",
            TokenType::OpenBracket => "OpenBracket",
            TokenType::CloseBracket => "CloseBracket",
            TokenType::OpenBrace => "OpenBrace",
            TokenType::CloseBrace => "CloseBrace",
            TokenType::Char(_) => "Char",
            TokenType::String(_) => "String",
            TokenType::Arrow => "Arrow",
            TokenType::UnitArrow => "UnitArrow",
            TokenType::CurriedArrow => "CurriedArrow",
            TokenType::TypeLambda => "TypeLambda",
            TokenType::FatArrow => "FatArrow",
            TokenType::Coma => "Coma",
            TokenType::Colon => "Colon",
            TokenType::DoubleColon => "DoubleColon",
            TokenType::Dot => "Dot",
            TokenType::DoubleDot => "DoubleDot",
            TokenType::DoubleDotEqual => "DoubleDotEqual",
            TokenType::SpacedDot => "SpacedDot",
            TokenType::Caret => "Caret",
            TokenType::Tilde => "Tilde",
            TokenType::Arobase => "Arobase",
            TokenType::Interogation => "Interogation",
            TokenType::Ampersand => "Ampersand",
            TokenType::Indent(_) => "Indent",
            TokenType::Underscore => "Underscore",
            TokenType::Eol => "Eol",
            TokenType::Eof => "Eof",
        }
    }
}

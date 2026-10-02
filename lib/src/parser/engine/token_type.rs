use crate::lexer::TokenType;

use super::{parse_error::ParseError, parser_trait::Parser, IResult, Input};

fn token_type_description(tt: &TokenType) -> String {
    match tt {
        TokenType::Ident(_) => "identifier".to_string(),
        TokenType::Type(_) => "type".to_string(),
        TokenType::Number(_) => "number".to_string(),
        TokenType::Float(_) => "float".to_string(),
        TokenType::Operator(s) => format!("operator '{}'", s),
        TokenType::Comment(_) => "comment".to_string(),
        TokenType::StuckOperator(s) => format!("operator '{}'", s),
        TokenType::NativeOperator(s) => format!("native operator '{}'", s),
        TokenType::Keyword(s) => format!("keyword '{}'", s),
        TokenType::MacroVar(_) => "macro variable".to_string(),
        TokenType::MacroInvoc(_) => "macro invocation".to_string(),
        TokenType::MacroRepeatOpen => "'$('".to_string(),
        TokenType::MacroRepeatClose => "')'".to_string(),
        TokenType::Equal => "'='".to_string(),
        TokenType::OpenParen => "'('".to_string(),
        TokenType::CloseParen => "')'".to_string(),
        TokenType::OpenBracket => "'['".to_string(),
        TokenType::OpenBrace => "'{'".to_string(),
        TokenType::CloseBrace => "'}'".to_string(),
        TokenType::CloseBracket => "']'".to_string(),
        TokenType::Char(_) => "character literal".to_string(),
        TokenType::String(_) => "string literal".to_string(),
        TokenType::Arrow => "'->'".to_string(),
        TokenType::UnitArrow => "'!->'".to_string(),
        TokenType::CurriedArrow => "'~>'".to_string(),
        TokenType::TypeLambda => "'\\'".to_string(),
        TokenType::FatArrow => "'=>'".to_string(),
        TokenType::Coma => "','".to_string(),
        TokenType::Colon => "':'".to_string(),
        TokenType::DoubleColon => "'::'".to_string(),
        TokenType::Dot => "'.'".to_string(),
        TokenType::DoubleDot => "'..'".to_string(),
        TokenType::DoubleDotEqual => "'..='".to_string(),
        TokenType::SpacedDot => "spaced dot".to_string(),
        TokenType::Caret => "'^'".to_string(),
        TokenType::Tilde => "'~'".to_string(),
        TokenType::Arobase => "'@'".to_string(),
        TokenType::Interogation => "'?'".to_string(),
        TokenType::Ampersand => "'&'".to_string(),
        TokenType::Indent(level) => format!("indent level {}", level),
        TokenType::Underscore => "'_'".to_string(),
        TokenType::Eol => "newline".to_string(),
        TokenType::Eof => "end of file".to_string(),
    }
}

impl Parser for TokenType {
    type Output = Self;

    fn process<'a>(&mut self, stream: Input<'a>) -> IResult<'a, Self> {
        let (stream, token) = stream.consume()?;

        if token.token_type == *self {
            Ok((stream, self.clone()))
        } else {
            Err(ParseError::UnexpectedToken(
                token_type_description(self),
                token,
            ))
        }
    }
}

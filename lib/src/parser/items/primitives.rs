use crate::lexer::TokenType;
use crate::parser::*;

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

macro_rules! token {
    ($stream:ident, $pat:pat => $result:expr) => {{
        let ($stream, token) = $stream.consume()?;

        if let $pat = &token.token_type {
            Ok(($stream, $result))
        } else {
            Err(ParseError::UnexpectedToken(
                token_type_description(&token.token_type),
                token.clone(),
            ))
        }
    }};
}

macro_rules! token_with_span {
    ($stream:ident, $span:ident, $pat:pat => $result:expr) => {{
        let ($stream, token) = $stream.consume()?;

        let $span = token.span.clone();

        if let $pat = token.token_type {
            Ok(($stream, $result))
        } else {
            Err(ParseError::UnexpectedToken(
                token_type_description(&token.token_type),
                token,
            ))
        }
    }};
}

pub fn ident_token(stream: Input) -> IResult<Ident> {
    token_with_span!(stream, span, TokenType::Ident(name) =>
        Ident { name, span }
    )
}

pub fn indent(stream: Input) -> IResult<()> {
    let (stream, token) = stream.consume()?;
    if let TokenType::Indent(level) = token.token_type {
        if level as usize != stream.indent_level {
            return Err(ParseError::UnexpectedIndent(level, token.span));
        }
        Ok((stream, ()))
    } else {
        Err(ParseError::UnexpectedToken(
            "indentation".to_string(),
            token,
        ))
    }
}

// used to get any indent level
pub fn indent_token(stream: Input) -> IResult<u8> {
    token!(stream, TokenType::Indent(level) => {
        *level
    })
}

pub fn boolean(stream: Input) -> IResult<bool> {
    TokenType::Keyword("true".to_string())
        .map(|_| true)
        .or(TokenType::Keyword("false".to_string()).map(|_| false))
        .process(stream)
}

pub fn int(stream: Input) -> IResult<u64> {
    let (stream, token) = stream.consume()?;

    if let TokenType::Number(value) = &token.token_type {
        value
            .parse()
            .map(|value| (stream, value))
            .map_err(|_| ParseError::UnexpectedToken("valid integer literal".to_string(), token))
    } else {
        Err(ParseError::UnexpectedToken(
            token_type_description(&token.token_type),
            token,
        ))
    }
}

pub fn float(stream: Input) -> IResult<f64> {
    let (stream, token) = stream.consume()?;

    if let TokenType::Float(value) = &token.token_type {
        value
            .parse()
            .map(|value| (stream, value))
            .map_err(|_| ParseError::UnexpectedToken("valid float literal".to_string(), token))
    } else {
        Err(ParseError::UnexpectedToken(
            token_type_description(&token.token_type),
            token,
        ))
    }
}

pub fn string(stream: Input) -> IResult<String> {
    token!(stream, TokenType::String(value) => {
        value.clone()
    })
}

pub fn char(stream: Input) -> IResult<String> {
    token!(stream, TokenType::Char(value) => {
        value.clone()
    })
}

pub fn macro_invoc_token(stream: Input) -> IResult<String> {
    token!(stream, TokenType::MacroInvoc(name) => {
        name.to_string()
    })
}

pub fn mut_prefix(stream: Input) -> IResult<()> {
    TokenType::Keyword("mut".to_string())
        .or(TokenType::Caret)
        .map(|_| ())
        .process(stream)
}

pub fn operator_token(stream: Input) -> IResult<Operator> {
    let (stream, token) = stream.consume()?;
    let span = token.span.clone();
    let token_type = token.token_type.clone();

    match token_type {
        TokenType::Operator(name) => Ok((
            stream,
            Operator {
                value: name,
                span: span.clone(),
            },
        )),
        TokenType::Caret => Ok((
            stream,
            Operator {
                value: "^".to_string(),
                span: span.clone(),
            },
        )),
        TokenType::Tilde => Ok((
            stream,
            Operator {
                value: "~".to_string(),
                span: span.clone(),
            },
        )),
        _ => Err(ParseError::UnexpectedToken(
            token_type_description(&token.token_type),
            token,
        )),
    }
}

pub fn stuck_operator_token(stream: Input) -> IResult<Operator> {
    let (stream, token) = stream.consume()?;
    let span = token.span.clone();
    let token_type = token.token_type.clone();

    match token_type {
        TokenType::StuckOperator(name) => Ok((
            stream,
            Operator {
                value: name,
                span: span.clone(),
            },
        )),
        TokenType::Tilde => Ok((
            stream,
            Operator {
                value: "~".to_string(),
                span: span.clone(),
            },
        )),
        _ => Err(ParseError::UnexpectedToken(
            token_type_description(&token.token_type),
            token,
        )),
    }
}

pub fn ampersand_token(stream: Input) -> IResult<Operator> {
    token_with_span!(stream, span, TokenType::Ampersand => {
        Operator {
            value: "&".to_string(),
            span,
        }
    })
}

pub fn operator(stream: Input) -> IResult<Operator> {
    operator_token
        .or(stuck_operator_token)
        .or(ampersand_token)
        .process(stream)
}

pub fn type_token(stream: Input) -> IResult<String> {
    token!(stream, TokenType::Type(name) => {
        name.clone()
    })
}

pub fn native_operator(stream: Input) -> IResult<NativeOperator> {
    token_with_span!(stream, span, TokenType::NativeOperator(name) => {
        NativeOperator {
            name: name.clone(),
            span,
        }
    })
}

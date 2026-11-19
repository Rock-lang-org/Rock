use crate::{
    lexer::TokenType,
    parser::{
        engine::*, ArrayPattern, FieldPattern, FieldsPatternOrArgumentsPattern, IdentPattern,
        InstancePattern, Pattern, PatternKind,
    },
};

use super::{ident, literal, mut_prefix, parenthesis, type_path};

pub fn pattern(stream: Input) -> IResult<Pattern> {
    (followed(ident, TokenType::Arobase).opt(), pattern_kind)
        .map(|(binding, kind)| Pattern { binding, kind })
        .process(stream)
}

pub fn pattern_kind(stream: Input) -> IResult<PatternKind> {
    // Try reference pattern first: &pattern or &mut pattern
    reference_pattern
        .or(parenthesis(separated1(pattern, TokenType::Coma).map(
            |mut patterns| {
                if patterns.len() == 1 {
                    PatternKind::Nested(Box::new(patterns.pop().unwrap()))
                } else {
                    PatternKind::Tuple(patterns)
                }
            },
        )))
        .or(delimited(
            TokenType::OpenBracket,
            separated(array_pattern, TokenType::Coma),
            TokenType::CloseBracket,
        )
        .map(PatternKind::Array))
        .or(instance_pattern.map(PatternKind::Instance))
        .or(ident_pattern.map(PatternKind::Ident))
        .or(TokenType::Underscore.map(|_| PatternKind::Wildcard))
        .or(literal.map(PatternKind::Literal))
        .process(stream)
}

/// Parse reference patterns: &pattern or &mut pattern
fn reference_pattern(stream: Input) -> IResult<PatternKind> {
    // Try &mut first, then just &
    (TokenType::Ampersand, mut_prefix.opt(), pattern)
        .map(|(_, mut_, pattern)| PatternKind::Reference {
            pattern: Box::new(pattern),
            mutable: mut_.is_some(),
        })
        .or(
            (TokenType::Ampersand, pattern).map(|(_, pattern)| PatternKind::Reference {
                pattern: Box::new(pattern),
                mutable: false,
            }),
        )
        .process(stream)
}

pub fn ident_pattern(stream: Input) -> IResult<IdentPattern> {
    (mut_prefix.opt(), ident)
        .map(|(mut_, name)| IdentPattern {
            name,
            mut_: mut_.is_some(),
        })
        .process(stream)
}

pub fn array_pattern(stream: Input) -> IResult<ArrayPattern> {
    (TokenType::DoubleDot, ident_pattern)
        .map(|(_, ident)| ArrayPattern::Rest(ident))
        .or((super::get_span, TokenType::DoubleDot).map(|(span, _)| {
            ArrayPattern::Rest(IdentPattern {
                name: crate::ast::Ident {
                    name: "_".to_string(),
                    span,
                },
                mut_: false,
            })
        }))
        .or(pattern.map(ArrayPattern::Pattern))
        .process(stream)
}

pub fn instance_pattern(stream: Input) -> IResult<InstancePattern> {
    (type_path, field_pattern_or_arguments_pattern)
        .map(|(name, args)| InstancePattern { name, args })
        .process(stream)
}

pub fn field_pattern_or_arguments_pattern(
    stream: Input,
) -> IResult<FieldsPatternOrArgumentsPattern> {
    separated1(field_pattern, TokenType::Coma)
        .map(FieldsPatternOrArgumentsPattern::Fields)
        .or(separated(pattern, TokenType::Coma).map(FieldsPatternOrArgumentsPattern::Arguments))
        .process(stream)
}

pub fn field_pattern(stream: Input) -> IResult<FieldPattern> {
    (ident, TokenType::Colon, pattern)
        .map(|(name, _, pattern)| FieldPattern { name, pattern })
        .process(stream)
}

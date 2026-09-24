use crate::{
    lexer::TokenType,
    parser::{engine::*, Match, MatchArm},
};

use super::{block, disallow_multiline_fn_call, empty_lines, expression, indent, pattern};

pub fn r#match(stream: Input) -> IResult<Match> {
    (
        TokenType::Keyword("match".to_string()),
        disallow_multiline_fn_call(expression),
        TokenType::Eol.followed_by(empty_lines),
        indented(separated1(
            match_arm,
            TokenType::Eol.followed_by(empty_lines),
        )),
    )
        .map(|(_, expr, _, arms)| Match {
            expr,
            arms,
            do_syntax: None,
        })
        .process(stream)
        .map_err(|e| e.with_context("match expression"))
}

pub fn match_arm(stream: Input) -> IResult<MatchArm> {
    (
        indent,
        pattern,
        preceded(TokenType::Keyword("if".to_string()), expression).opt(),
        TokenType::FatArrow,
        block,
    )
        .map(|(_, pattern, condition, _, body)| MatchArm {
            pattern,
            condition,
            body,
        })
        .process(stream)
        .map_err(|e| e.with_context("match arm"))
}

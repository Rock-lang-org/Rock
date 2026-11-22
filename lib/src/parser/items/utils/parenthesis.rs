use crate::{lexer::TokenType, parser::engine::*};

pub fn parenthesis<P: Parser>(parser: P) -> impl FnMut(Input) -> IResult<P::Output> {
    let mut parser = (
        TokenType::OpenParen,
        super::empty_lines::empty_lines.opt(),
        parser,
        super::empty_lines::empty_lines.opt(),
        TokenType::CloseParen,
    )
        .map(|(_, _, x, _, _)| x);

    move |input: Input| parser.process(input)
}

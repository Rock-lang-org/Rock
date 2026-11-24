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

    move |mut input: Input| {
        // A dot inside explicit parentheses belongs to the grouped expression,
        // not to an enclosing infix continuation at the same indentation.
        let outer_indent = input.infix_continuation_indent.take();
        let (mut stream, value) = parser.process(input)?;
        stream.infix_continuation_indent = outer_indent;
        Ok((stream, value))
    }
}

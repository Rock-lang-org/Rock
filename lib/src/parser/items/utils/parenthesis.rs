use crate::{lexer::TokenType, parser::engine::*};

pub fn parenthesis<P: Parser>(parser: P) -> impl FnMut(Input) -> IResult<P::Output> {
    let mut parser = (
        TokenType::OpenParen,
        super::empty_lines::empty_lines.opt(),
        parser,
        super::empty_lines::empty_lines.opt(),
        TokenType::CloseParen.or(preceded(
            TokenType::Eol,
            preceded(super::empty_lines::empty_lines, preceded(crate::parser::indent_token, TokenType::CloseParen)),
        )),
    )
        .map(|(_, _, x, _, _)| x);

    move |input: Input| {
        let was_in_arg_list = input.inside_argument_list;
        let result = parser.process(input)?;
        let (mut output_stream, value) = result;

        // If we just closed a paren while inside an argument list,
        // set the flag so the next dot will close the argument list
        if was_in_arg_list {
            output_stream.after_closing_paren = true;
        }

        Ok((output_stream, value))
    }
}

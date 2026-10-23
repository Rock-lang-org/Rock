use super::{parser_trait::Parser, IResult, Input};

impl<F, T> Parser for F
where
    F: FnMut(Input<'_>) -> IResult<T>,
{
    type Output = T;

    fn process<'a>(&mut self, tokens: Input<'a>) -> IResult<'a, Self::Output> {
        super::budget::invoke(tokens, self)
    }
}

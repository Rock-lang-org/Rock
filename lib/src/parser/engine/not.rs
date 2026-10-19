use crate::parser::*;

// does not consume any tokens
pub fn not<P: Parser>(mut parser: P) -> impl FnMut(Input) -> IResult<()> {
    move |stream: Input| {
        if let Ok((stream, _)) = parser.process(stream) {
            Err(ParseError::UnexpectedToken(
                "not".to_string(),
                stream.seek()?,
            ))
        } else {
            Ok((stream, ()))
        }
    }
}

pub mod engine;
mod items;

#[cfg(test)]
mod conformance_tests;

#[cfg(test)]
mod source_pipeline_tests;

use std::path::PathBuf;

pub use engine::*;
pub use items::*;

use crate::ast::tree::*;
use crate::lexer::lossless::{LosslessStream, ScanLimits};
use crate::lexer::{Span, TokenType};
use crate::{Config, DebugPrint};

pub fn parse_source(
    file_path: PathBuf,
    source: &str,
    config: &Config,
) -> Result<Module, ParseError> {
    let anchor = Span::new(file_path.clone(), 0, 0);
    if source.len() > SOURCE_LIMITS.max_source_bytes {
        return Err(ParseError::HardError("parser source byte limit exceeded".into(), anchor));
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(source.len())
        .map_err(|_| ParseError::HardError("cannot allocate parser source".into(), anchor.clone()))?;
    bytes.extend_from_slice(source.as_bytes());
    let stream = LosslessStream::from_bytes(file_path, bytes, SOURCE_LIMITS)
        .map_err(|error| ParseError::HardError(format!("source preservation failed: {error:?}"), anchor))?;
    parse_lossless_source(&stream, config, None)
}

const SOURCE_LIMITS: ScanLimits = ScanLimits { max_source_bytes: 16 * 1024 * 1024, max_runs: 1024 * 1024 };
const CURSOR_LIMITS: source_cursor::SourceLimits = source_cursor::SourceLimits { max_entries: 1024 * 1024, max_branches: 65536 };

/// Ordinary and generated source share one preservation/interpretation boundary.
pub fn parse_lossless_source(
    stream: &LosslessStream,
    config: &Config,
    expansion_parent: Option<crate::macro_expansion::ExpansionId>,
) -> Result<Module, ParseError> {
    parse_lossless(stream, config, expansion_parent, None, module_inline).map(|mut module| {
        module.filepath = Some(stream.start_span().file_path);
        module
    })
}

pub fn parse_lossless_expression(
    stream: &LosslessStream,
    config: &Config,
    expansion_parent: Option<crate::macro_expansion::ExpansionId>,
) -> Result<Expression, ParseError> {
    parse_lossless(stream, config, expansion_parent, None, expression_fragment)
}

pub(crate) fn parse_expression_fragment(
    fragment: &crate::lexer::lossless::ExpressionFragment,
    config: &Config,
    expansion_parent: Option<crate::macro_expansion::ExpansionId>,
) -> Result<Expression, ParseError> {
    parse_lossless(&fragment.input, config, expansion_parent, Some(fragment.header_indent), expression_fragment)
}

pub(crate) fn operand_prefix(stream: &LosslessStream, config: &Config, header_indent: usize)
    -> Result<usize, ParseError>
{
    retained_prefix(stream, config, header_indent, operand)
}

pub(crate) fn expression_prefix(stream: &LosslessStream, config: &Config, header_indent: usize)
    -> Result<usize, ParseError>
{
    retained_prefix(stream, config, header_indent, expression)
}

fn retained_prefix<P: Parser>(stream: &LosslessStream, config: &Config, header_indent: usize, mut parser: P)
    -> Result<usize, ParseError>
{
    let _scope = engine::ErrorScope::new();
    let input = source_cursor::SourceInput::with_layout(stream, true, CURSOR_LIMITS, Some(header_indent))
        .map_err(|error| engine::source_error(error, stream.start_span()))?;
    let ctx = ParseCtx::from_source(&input, config);
    let (ctx, _) = indent.process(ctx)?;
    let (rest, _) = parser.process(ctx)?;
    rest.source_offset()?.ok_or_else(|| ParseError::HardError("macro fragment has no retained boundary".into(), stream.start_span()))
}

fn expression_fragment(stream: Input) -> IResult<Expression> {
    let (stream, _) = empty_lines.process(stream)?;
    let (stream, _) = indent.process(stream)?;
    let (stream, expression) = expression.process(stream)?;
    let (stream, _) = TokenType::Eol.process(stream)?;
    let (stream, _) = empty_lines.process(stream)?;
    let (stream, _) = TokenType::Eof.process(stream)?;
    Ok((stream, expression))
}

fn parse_lossless<P: Parser>(
    stream: &LosslessStream,
    config: &Config,
    expansion_parent: Option<crate::macro_expansion::ExpansionId>,
    relative_indent: Option<usize>,
    mut parser: P,
) -> Result<P::Output, ParseError> {
    let _scope = engine::ErrorScope::new();
    let input = source_cursor::SourceInput::with_layout(stream, true, CURSOR_LIMITS, relative_indent)
        .map_err(|error| engine::source_error(error, stream.start_span()))?;

    let ctx = ParseCtx { expansion_parent, ..ParseCtx::from_source(&input, config) };
    let result = parser.process(ctx).and_then(|(rest, value)| {
        if rest.is_empty() { Ok(value) } else {
            Err(ParseError::UnexpectedToken("end of source fragment".into(), rest.seek()?))
        }
    });

    if config.has_debug_print(DebugPrint::Tokens) {
        println!("{:#?}", input.tokens_debug());
    }

    match result {
        Ok(value) => Ok(value),
        Err(e) => {
            // Return the best error we've seen during parsing
            Err(engine::get_best_error(e).without_source_context())
        }
    }
}

pub fn parse_string(input: &str, config: &Config) -> Result<Program, ParseError> {
    parse_source(PathBuf::from("<memory>"), input, config).map(|mut module| {
        module.filepath = None;
        Program { module }
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::ast::TopLevel;
    use crate::parser::{parse_source, parse_string, ParseError};
    use crate::Config;

    #[test]
    fn parse_source_sets_module_filepath_without_reading_sibling_modules() {
        let path = PathBuf::from("/virtual/app/main.rk");
        let module = parse_source(
            path.clone(),
            "mod missing\nmain = -> 0\n",
            &Config::default(),
        )
        .expect("source text should parse without filesystem module lookup");

        assert_eq!(module.filepath, Some(path));
        match &module.top_levels[0] {
            TopLevel::Mod(ident, false) => assert_eq!(ident.name, "missing"),
            other => panic!("expected source-backed module declaration, got {other:?}"),
        }
    }

    #[test]
    fn parse_string_remains_parser_only_and_has_no_filepath() {
        let program =
            parse_string("main = -> 0\n", &Config::default()).expect("inline source should parse");

        assert_eq!(program.module.filepath, None);
    }

    #[test]
    fn parse_string_reports_oversized_integer_literal() {
        let result = parse_string("main = -> 184467440737095516160\n", &Config::default());

        assert!(result.is_err());
    }

    #[test]
    fn parse_string_reports_odd_indentation() {
        let result = parse_string("main = ->\n x\n", &Config::default());

        assert!(result.is_err());
    }

    #[test]
    fn parse_string_reports_odd_indentation_at_the_indent_token() {
        let input = "main = ->\n x\n";
        let error = parse_string(input, &Config::default()).expect_err("odd indent should fail");

        let ParseError::UnexpectedIndent(level, span) = error else {
            panic!("expected an indentation error");
        };
        assert_eq!(level, 1);
        assert_eq!(span.file_path, PathBuf::from("<memory>"));
        assert_eq!(span.start, 10);
        assert_eq!(span.end, 11);
    }

    #[test]
    fn parse_string_accepts_whitespace_only_blank_line() {
        let result = parse_string("main = ->\n    0\n \nfoo = -> 1\n", &Config::default());

        assert!(result.is_ok(), "parse failed: {result:?}");
    }

    #[test]
    fn parse_string_rejects_odd_multiline_call_indent() {
        let result = parse_string("main = -> foo\n x\n", &Config::default());

        assert!(result.is_err());
    }
}

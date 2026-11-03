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

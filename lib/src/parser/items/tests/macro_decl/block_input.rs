use crate::ast::*;
use crate::macro_expansion::{expand_macros_with_context, MacroExpansionContext};
use crate::parser::{parse_string, ParseError};
use crate::Config;

fn parse(source: &str) -> Program {
    parse_string(source, &Config::default()).unwrap()
}

fn text(invocation: &MacroInvoc) -> String {
    invocation.args.runs().iter().map(|run| run.text()).collect()
}

fn item(program: &Program) -> &MacroInvoc {
    let TopLevel::MacroInvoc(invocation) = &program.module.top_levels[0] else { panic!("expected item invocation") };
    invocation
}

fn expression(expression: &Expression) -> &MacroInvoc {
    let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = expression else { panic!("expected primary") };
    let Operand::MacroInvoc(invocation) = &primary.operand else { panic!("expected expression invocation") };
    invocation
}

#[test]
fn macro_a_03_item_blocks_preserve_arbitrary_input_before_physical_dedent() {
    for body in ["   \"broken ([} 🦀\n", "    /* unterminated\n", "    <p>Don't parse // this</p>\n", "    \0 ` %nested!\n"] {
        let program = parse(&format!("%pkg::dsl\n{body}main = !->\n    42\n"));
        let invocation = item(&program);
        assert_eq!(text(invocation), body);
        assert_eq!(invocation.path_name(), "pkg::dsl");
        assert_eq!(invocation.output, crate::ast::MacroOutputCategory::Items);
        assert!(invocation.block.is_some());
        assert_eq!(program.module.top_levels.len(), 2);
    }
}

#[test]
fn macro_a_03_expression_block_uses_its_physical_header_indent() {
    let body = "      <html>\n     odd internal margin\n        \"unterminated\n";
    let program = parse(&format!("main = !->\n    page = %html\n{body}    page\n"));
    let TopLevel::FunctionDecl(function) = &program.module.top_levels[0] else { panic!("expected function") };
    let [Statement::Assignment(assignment), Statement::Expression(_)] = &function.lambda.body.statements[..] else { panic!("expected two statements") };
    let invocation = expression(&assignment.rhs);
    assert_eq!(text(invocation), body);
    let layout = invocation.block.as_ref().unwrap();
    assert_eq!(layout.header_indent, 4);
    assert_eq!(layout.body_indent, 6);
    assert_eq!(invocation.output, crate::ast::MacroOutputCategory::Expression);
    assert!(invocation.arguments.is_empty());
}

#[test]
fn macro_a_03_blank_lines_and_comment_looking_lines_are_physical_data() {
    let body = "\n \t\n    // first DSL line\n\n    /* another\n \n";
    let program = parse(&format!("%dsl\n{body}// terminates the block\nmain = !-> 42\n"));
    assert_eq!(text(item(&program)), body);
    assert_eq!(program.module.top_levels.len(), 2);
    assert_eq!(item(&program).args.start_span().start, "%dsl\n".len());
    assert_eq!(item(&program).args.end_span().start, "%dsl\n".len() + body.len());
}

#[test]
fn macro_a_03_eof_and_crlf_retain_untrimmed_bytes() {
    for body in ["    \"🦀", "\r\n\t\t// 🦀\r\n \t", "    x\n\n"] {
        let program = parse(&format!("%dsl\r\n{body}"));
        assert_eq!(text(item(&program)), body);
    }
    let body = format!("{}wide\n", " ".repeat(512));
    let program = parse(&format!("%dsl\n{body}"));
    assert_eq!(text(item(&program)), body);
    assert_eq!(item(&program).block.as_ref().unwrap().body_indent, 512);
}

#[test]
fn macro_a_03_empty_body_is_not_an_implicit_expression_call() {
    for source in ["main = -> %m\n", "main = -> %m\n\nmain2 = !-> 42\n", "main = -> %m\n \t\n"] {
        assert!(parse_string(source, &Config::default()).is_err(), "{source}");
    }
    let program = parse("%m\n\nmain = !-> 42\n");
    assert!(item(&program).block.is_none());
    assert_eq!(program.module.top_levels.len(), 2);
}

#[test]
fn macro_a_03_strings_comments_bang_and_derive_do_not_become_block_headers() {
    let program = parse("// %dsl\nmain = -> \"%dsl\\n    literal\"\n");
    assert_eq!(program.module.top_levels.len(), 1);
    assert!(parse_string("main = -> %m!\n    `not Rock\n", &Config::default()).is_err());
    for source in ["%derive\n    unmatched \"\n", "main = -> %derive\n    unmatched \"\n"] {
        let error = parse_string(source, &Config::default()).unwrap_err();
        assert!(matches!(&error, ParseError::HardError(message, _) if message.contains("derive")), "{error:?}");
    }
}

#[test]
fn macro_a_03_following_rock_errors_are_not_swallowed_by_body_quotes() {
    let source = "%dsl\n    \"opaque\nmain = -> \"ordinary failure";
    let error = parse_string(source, &Config::default()).unwrap_err();
    assert!(matches!(&error, ParseError::Lexer(crate::lexer::LexerError::UnknownToken(_, span)) if span.start == source.rfind('"').unwrap()), "{error:?}");
}

#[test]
fn macro_a_03_ordinary_multiline_calls_still_parse_and_block_execution_diagnoses() {
    parse("main = !->\n    f\n        1\n        2\n");
    let program = parse("%dsl\n    arbitrary ([}\n");
    let config = Config::default();
    let context = MacroExpansionContext::new(&config);
    let diagnostics = expand_macros_with_context(program, &context).unwrap_err();
    assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.message.contains("block macro expansion is not implemented")));
}

#[test]
fn macro_a_03_parenthesized_block_arguments_retain_the_outer_input_range() {
    let source = "main = -> %outer (%inner\n    arbitrary \" ([}\n)\n";
    let program = parse(source);
    let TopLevel::FunctionDecl(function) = &program.module.top_levels[0] else { panic!("expected function") };
    let Statement::Expression(value) = &function.lambda.body.statements[0] else { panic!("expected expression") };
    let outer = expression(value);
    assert_eq!(text(outer), " (%inner\n    arbitrary \" ([}\n)");
    assert_eq!(outer.arguments.len(), 1);
    assert!(outer.block.is_none());
}

#[test]
fn macro_a_03_inline_macro_headers_do_not_capture_a_separate_dsl_body() {
    assert!(parse_string("main = -> %m 1\n    `opaque\n", &Config::default()).is_err());
    let source = "%dsl // header comment\n    arbitrary \"\nmain = !-> 42\n";
    let program = parse(source);
    assert_eq!(text(item(&program)), "    arbitrary \"\n");
    assert_eq!(program.module.top_levels.len(), 2);
}

#[test]
fn macro_a_03_inline_module_blocks_stop_at_the_header_margin() {
    let source = "mod nested\n    %dsl\n      arbitrary \"\n    keep = !-> 42\nmain = !-> 42\n";
    // Exercise the inline-module parser directly: top-level `mod` dispatch
    // currently selects the source-backed shell before the inline alternative.
    let stream = crate::macro_expansion::TokenStream::from_bytes("<memory>".into(), source.as_bytes().to_vec(),
        crate::macro_expansion::ScanLimits { max_source_bytes: 65536, max_runs: 4096 }).unwrap();
    let input = crate::parser::source_cursor::SourceInput::new(&stream, true,
        crate::parser::source_cursor::SourceLimits { max_entries: 4096, max_branches: 128 }).unwrap();
    let config = Config::default();
    let context = crate::parser::ParseCtx::from_source(&input, &config).advance(1).unwrap();
    let (rest, module) = crate::parser::module(context).unwrap();
    let TopLevel::MacroInvoc(invocation) = &module.top_levels[0] else { panic!("expected block") };
    assert_eq!(text(invocation), "      arbitrary \"\n");
    assert_eq!(module.top_levels.len(), 2);
    assert_eq!(rest.seek_nth(1).unwrap().token_type, crate::lexer::TokenType::Ident("main".into()));
}

#[test]
fn macro_a_03_array_lookahead_does_not_interpret_a_nested_block_body() {
    let source = "main = -> [\n    %dsl\n        \"opaque payload ([}\n]\ntail = -> \"after\"\n";
    let stream = crate::macro_expansion::TokenStream::from_bytes("<memory>".into(), source.as_bytes().to_vec(),
        crate::macro_expansion::ScanLimits { max_source_bytes: 65536, max_runs: 4096 }).unwrap();
    let input = crate::parser::source_cursor::SourceInput::new(&stream, true,
        crate::parser::source_cursor::SourceLimits { max_entries: 4096, max_branches: 128 }).unwrap();
    let config = Config::default();
    let context = crate::parser::ParseCtx::from_source(&input, &config);
    crate::parser::module_inline(context).unwrap();
    assert!(!format!("{:?}", input.tokens_debug()).contains("opaque payload"));
}

use crate::ast::*;
use crate::macro_expansion::{expand_macros_with_context, MacroExpansionContext, ScanLimits};
use crate::parser::parse_string;
use crate::Config;

fn parse(source: &str) -> Program {
    parse_string(&format!("main = -> {source}\n"), &Config::default()).unwrap()
}

fn body(program: &Program) -> &Expression {
    let TopLevel::FunctionDecl(function) = &program.module.top_levels[0] else { panic!("expected function") };
    let Statement::Expression(expression) = &function.lambda.body.statements[0] else { panic!("expected expression") };
    expression
}

fn primary(expression: &Expression) -> &PrimaryExpr {
    let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = expression else { panic!("expected primary: {expression:?}") };
    primary
}

fn invocation(expression: &Expression) -> &MacroInvoc {
    let Operand::MacroInvoc(invocation) = &primary(expression).operand else { panic!("expected macro") };
    invocation
}

fn argument_text(invocation: &MacroInvoc) -> Vec<String> {
    invocation.arguments.iter().map(|range| {
        invocation.args.slice(range.clone(), ScanLimits { max_source_bytes: 65536, max_runs: 4096 }).unwrap()
            .runs().iter().map(|run| run.text()).collect()
    }).collect()
}

#[test]
fn macro_a_02_qualified_calls_keep_ordinary_argument_ranges() {
    let program = parse("%pkg::m 1, (f 2, 3)");
    let invocation = invocation(body(&program));
    assert_eq!(invocation.path_name(), "pkg::m");
    assert_eq!(invocation.output, crate::ast::MacroOutputCategory::Expression);
    assert_eq!(argument_text(invocation), vec!["1", "(f 2, 3)"]);
    assert_eq!(invocation.args.runs().iter().map(|run| run.text()).collect::<String>(), " 1, (f 2, 3)");
}

#[test]
fn macro_a_02_bang_calls_have_empty_input_and_allow_result_postfixes() {
    let program = parse("%pkg::m!.field!");
    let invocation = invocation(body(&program));
    assert!(invocation.arguments.is_empty());
    assert_eq!(invocation.args.len_bytes(), 0);
    assert!(matches!(primary(body(&program)).secondaries.as_deref(), Some([SecondaryExpr::Dot(_), SecondaryExpr::Arguments(arguments)]) if arguments.is_empty()));
}

#[test]
fn macro_a_02_argument_names_and_nested_calls_need_no_resolution_or_execution() {
    let program = parse("%unknown::outer missing, (%also_unknown::inner!)");
    assert_eq!(argument_text(invocation(body(&program))), vec!["missing", "(%also_unknown::inner!)"]);
    let config = Config::default();
    let context = MacroExpansionContext::new(&config);
    let diagnostics = expand_macros_with_context(program, &context).unwrap_err();
    assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.message.contains("expression macro expansion is not implemented")));
    assert!(!diagnostics.0.iter().any(|diagnostic| diagnostic.message.contains("inner")));
}

#[test]
fn macro_a_02_propagation_completes_the_macro_before_following_operators() {
    let program = parse("%m first, second? + tail");
    let Expression::BinopExpr(UnaryExpr::PrimaryExpr(left), operator, _) = body(&program) else { panic!("expected outer operator") };
    assert_eq!(operator.value, "+");
    let Operand::MacroInvoc(invocation) = &left.operand else { panic!("expected macro") };
    assert_eq!(argument_text(invocation), vec!["first", "second"]);
    assert!(matches!(left.secondaries.as_deref(), Some([SecondaryExpr::Interogation])));
}

#[test]
fn macro_a_02_parentheses_choose_argument_or_result_postfix_ownership() {
    let program = parse("(%m value).field!");
    let outer = primary(body(&program));
    let Operand::Expression(inner) = &outer.operand else { panic!("expected grouping") };
    assert_eq!(argument_text(invocation(inner)), vec!["value"]);
    assert!(matches!(outer.secondaries.as_deref(), Some([SecondaryExpr::Dot(_), SecondaryExpr::Arguments(_)])));
    let program = parse("%m (value.field!)");
    assert_eq!(argument_text(invocation(body(&program))), vec!["(value.field!)"]);
    assert!(primary(body(&program)).secondaries.is_none());
}

#[test]
fn macro_a_02_exact_ranges_retain_internal_comments_and_crlf() {
    let program = parse("%m  (left /* 🦀 */ + right), (f 2, 3)\r");
    let invocation = invocation(body(&program));
    assert_eq!(argument_text(invocation), vec!["(left /* 🦀 */ + right)", "(f 2, 3)"]);
    assert!(invocation.span.end > invocation.span.start);
}

#[test]
fn macro_a_02_malformed_inline_rock_and_missing_arguments_reject() {
    for source in ["%m", "%m (", "%m 1, , 2", "%m \"broken", "%pkg:: 1", "%m?", "%m [1, )"] {
        assert!(parse_string(&format!("main = -> {source}\n"), &Config::default()).is_err(), "{source}");
    }
}

#[test]
fn macro_a_02_macro_and_ordinary_calls_share_argument_boundaries() {
    for (source, count) in [("1, (f 2, 3)", 2), ("nested value", 1), ("first, nested second", 2), ("(left?), right?", 2), ("&mut value", 1), ("(left, right)", 1)] {
        let macro_program = parse(&format!("%m {source}"));
        assert_eq!(invocation(body(&macro_program)).arguments.len(), count, "{source}");
        let ordinary_program = parse(&format!("m {source}"));
        let Some(SecondaryExpr::Arguments(arguments)) = primary(body(&ordinary_program)).secondaries.as_ref().and_then(|secondaries| secondaries.first()) else { panic!("expected ordinary call") };
        assert_eq!(arguments.len(), count, "{source}");
    }
}

#[test]
fn macro_a_02_inline_lexical_errors_keep_the_original_source_span() {
    let source = "main = -> %m \"broken";
    let error = parse_string(source, &Config::default()).unwrap_err();
    assert!(matches!(&error, crate::parser::ParseError::Lexer(crate::lexer::LexerError::UnknownToken(_, span)) if span.start == source.find('"').unwrap() && span.end == source.len()), "{error:?}");
}

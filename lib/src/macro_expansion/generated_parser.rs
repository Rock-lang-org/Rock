use crate::ast::{Expression, MacroOutputCategory, Module};
use crate::diagnostic::Diagnostics;
use crate::{parser, Config};

#[derive(Debug)]
pub enum GeneratedSyntax {
    Items(Module),
    Expression(Expression),
}

pub fn parse_generated_source(
    stream: &crate::lexer::lossless::LosslessStream,
    config: &Config,
    category: MacroOutputCategory,
    expansion_parent: Option<super::ExpansionId>,
) -> Result<GeneratedSyntax, Diagnostics> {
    match category {
        MacroOutputCategory::Items => parser::parse_lossless_source(stream, config, expansion_parent).map(GeneratedSyntax::Items),
        MacroOutputCategory::Expression => parser::parse_lossless_expression(stream, config, expansion_parent).map(GeneratedSyntax::Expression),
    }.map_err(Diagnostics::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use crate::macro_expansion::token_tree::source;
    use crate::parser::ParseError;
    use crate::Config;

    fn test_config() -> Config {
        Config {
            entry_file: PathBuf::from("/test.rk"),
            output_dir: PathBuf::new(),
            debug_print: Vec::new(),
            meta_files: Vec::new(),
            extern_artifacts: Vec::new(),
            source_providers: Vec::new(),
            current_crate_name: None,
            opt_level: 0,
            overflow_checks: None,
            emit_llvm: false,
            no_link: false,
            emit_object: None,
            no_prelude: false,
            no_std: false,
            sysroot: None,
            no_proc_execution: false,
        }
    }

    #[test]
    fn generated_parser_parses_top_level_tokens_with_explicit_config() {
        let stream = source("main = -> 0");
        let config = test_config();

        let GeneratedSyntax::Items(module) = parse_generated_source(&stream, &config, MacroOutputCategory::Items, None).unwrap() else { panic!("expected items") };

        assert_eq!(module.top_levels.len(), 1);
    }

    #[test]
    fn generated_parser_reports_incomplete_macro_head() {
        let stream = source("macro make\n    $name");
        let config = test_config();

        let parse_error = parser::parse_lossless_source(&stream, &config, None)
            .expect_err("incomplete macro head should fail before diagnostics conversion");
        match parse_error {
            ParseError::HardError(message, _) => {
                assert!(message.contains("Incomplete macro head"));
            }
            other => panic!("expected incomplete macro head error, got {other:?}"),
        }

        let diagnostics = parse_generated_source(&stream, &config, MacroOutputCategory::Items, None).unwrap_err();
        assert!(diagnostics
            .0
            .iter()
            .any(|diagnostic| diagnostic.message.contains("Incomplete macro head")));
    }

    fn generated(text: &str, category: crate::ast::MacroOutputCategory) -> Result<GeneratedSyntax, Diagnostics> {
        parse_generated_source(&source(text), &test_config(), category, Some(super::super::ExpansionId(7)))
    }

    #[test]
    fn macro_a_04_requested_expression_is_one_ordinary_expression() {
        use crate::ast::*;
        let GeneratedSyntax::Expression(value) = generated("f first, (g second, third)", MacroOutputCategory::Expression).unwrap() else { panic!("expected expression") };
        // Spelling and structure are ordinary Rock; the entry point does not wrap
        // the result in a lambda or reinterpret it as an item on failure.
        let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = value else { panic!("expected primary") };
        let Some([SecondaryExpr::Arguments(arguments)]) = primary.secondaries.as_deref() else { panic!("expected call") };
        assert_eq!(arguments.len(), 2);
        let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(grouped)) = &arguments[1].arg else { panic!("expected primary argument") };
        assert!(matches!(grouped.operand, Operand::Expression(_)));
    }

    #[test]
    fn macro_a_04_item_lists_allow_empty_and_multiple_items() {
        use crate::ast::MacroOutputCategory;
        for (text, count) in [("", 0), ("// empty\n", 0), ("first = !-> 1\nsecond = !-> 2\n", 2)] {
            let GeneratedSyntax::Items(module) = generated(text, MacroOutputCategory::Items).unwrap() else { panic!("expected items") };
            assert_eq!(module.top_levels.len(), count);
        }
    }

    #[test]
    fn macro_a_04_category_mismatches_and_partial_results_reject() {
        use crate::ast::MacroOutputCategory::{Expression, Items};
        for (text, category) in [("", Expression), ("// empty", Expression), ("main = -> 1", Expression),
            ("42", Items), ("42\n43", Expression), ("42\nmain = -> 0", Expression),
            ("42 )", Expression), ("main = -> 0\n42", Items), ("42\0ignored", Expression),
            ("main = -> 0\0ignored", Items), ("<div>🦀</div>", Expression), ("<div>🦀</div>", Items)] {
            assert!(generated(text, category).is_err(), "{category:?}: {text:?}");
        }
    }

    #[test]
    fn macro_a_04_nested_blocks_keep_input_opaque_and_attach_parent_edges() {
        use crate::ast::*;
        let payload = "    \"opaque ([} %discarded!\n";
        let GeneratedSyntax::Expression(value) = generated(&format!("%pkg::dsl\n{payload}"), MacroOutputCategory::Expression).unwrap() else { panic!("expected expression") };
        let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = value else { panic!("expected primary") };
        let Operand::MacroInvoc(invocation) = primary.operand else { panic!("expected invocation") };
        assert_eq!(invocation.args.runs().iter().map(|run| run.text()).collect::<String>(), payload);
        assert_eq!(invocation.parent, Some(super::super::ExpansionId(7)));
        assert_eq!(invocation.output, MacroOutputCategory::Expression);
        let GeneratedSyntax::Items(module) = generated(&format!("main = -> %dsl\n{payload}next = !-> 42\n"), MacroOutputCategory::Items).unwrap() else { panic!("expected items") };
        assert_eq!(module.top_levels.len(), 2);
    }

    #[test]
    fn macro_a_04_normal_block_and_control_flow_syntax_is_not_lambda_wrapped() {
        use crate::ast::*;
        for text in ["do\n    value = 42\n    value\n", "loop\n    break 7\n"] {
            let GeneratedSyntax::Expression(value) = generated(text, MacroOutputCategory::Expression).unwrap() else { panic!("expected expression") };
            let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = value else { panic!("expected primary") };
            assert!(!matches!(primary.operand, Operand::LambdaDecl(_)));
        }
    }

    #[test]
    fn macro_a_04_bounded_output_retains_original_offsets_and_errors() {
        use crate::ast::*;
        let stream = source("prefix 42 suffix");
        let stream = stream.slice(7..9, super::super::token_tree::MACRO_LIMITS).unwrap();
        let GeneratedSyntax::Expression(value) = parse_generated_source(&stream, &test_config(), MacroOutputCategory::Expression, None).unwrap() else { panic!("expected expression") };
        let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = value else { panic!("expected primary") };
        let Operand::Literal(literal) = primary.operand else { panic!("expected literal") };
        assert_eq!(literal.span.start..literal.span.end, 7..9);
        let stream = source("prefix \"broken\" suffix");
        let stream = stream.slice(7..11, super::super::token_tree::MACRO_LIMITS).unwrap();
        let diagnostics = parse_generated_source(&stream, &test_config(), MacroOutputCategory::Expression, None).unwrap_err();
        assert!(diagnostics.0.iter().any(|diagnostic| matches!(&diagnostic.location, crate::diagnostic::DiagnosticLocation::Source(span) if span.start == 7 && span.end == 11)));
    }
}

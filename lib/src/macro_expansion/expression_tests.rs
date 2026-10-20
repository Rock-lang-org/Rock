use super::*;
use crate::ast::*;
use super::token_tree::{append, source, MACRO_LIMITS};

fn expand(text: &str) -> Result<Program, Diagnostics> {
    let config = crate::Config::default();
    expand_macros_with_context(crate::parser::parse_string(text, &config).unwrap(), &MacroExpansionContext::new(&config))
}

fn body(program: &Program) -> &Block {
    let TopLevel::FunctionDecl(function) = program.module.top_level_from_ident("main").unwrap() else { panic!("expected function") };
    &function.lambda.body
}

#[test]
fn macro_a_06_expressions_expand_in_bindings_and_tails() {
    let program = expand("macro id\n    $value:expr =>\n        $value\nmain = ->\n    value = %id 42\n    %id value\n").unwrap();
    assert_eq!(body(&program).statements.len(), 2);
    assert!(!format!("{:?}", body(&program)).contains("MacroInvoc"));
}

#[test]
fn macro_a_06_nested_forwarding_keeps_expression_grouping_and_discards_input() {
    let program = expand("macro id\n    $value:expr =>\n        $value\nmacro scale\n    $value:expr =>\n        %id ($value * 3)\nmacro discard\n    $value:expr =>\n        42\nmain = ->\n    %discard (%undefined!)\n    %scale (1 + 2)\n").unwrap();
    assert!(!format!("{:?}", body(&program)).contains("MacroInvoc"));
}

#[test]
fn macro_a_06_expression_category_failures_keep_invocation_diagnostics() {
    for definition in ["        other = -> 1\n", "        $( $unused )*\n", "        1\n        2\n"] {
        let diagnostics = expand(&format!("macro bad\n    =>\n{definition}main = -> %bad!\n")).unwrap_err();
        assert!(!diagnostics.0.is_empty());
        assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.message.contains("expanded from macro")
            || !diagnostic.secondary.is_empty() || !diagnostic.notes.is_empty()), "{diagnostics:?}");
    }
}

#[test]
fn macro_a_06_expression_fragments_keep_spelling_origins_and_wire_boundaries() {
    let input = source("1 /* caller */ + 2");
    let marked = input.clone().expression_fragment(0, MACRO_LIMITS).unwrap();
    assert_eq!(marked.runs().iter().map(|run| run.text()).collect::<String>(), "1 /* caller */ + 2");
    let mut output = source("3 * ");
    append(&mut output, &marked).unwrap();
    let encoded = proc_macro::encode_stream(&output).unwrap();
    let decoded = proc_macro::decode_stream(&encoded).unwrap();
    assert_eq!(proc_macro::encode_stream(&decoded).unwrap(), encoded);
    let generated_parser::GeneratedSyntax::Expression(expression) = generated_parser::parse_generated_source(
        &decoded, &crate::Config::default(), MacroOutputCategory::Expression, None).unwrap() else { panic!("expected expression") };
    let Expression::BinopExpr(_, _, right) = expression else { panic!("expected outer multiply") };
    let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = *right else { panic!("expected fragment operand") };
    assert!(matches!(primary.operand, Operand::Expression(_)));
}

#[test]
fn macro_a_06_nested_opaque_blocks_are_forwarded_without_early_interpretation() {
    let config = crate::Config::default();
    let program = crate::parser::parse_string("macro id\n    $value:expr =>\n        $value\nmain = ->\n    %id (%dsl\n        \"opaque ([} 🦀\n    )\n", &config).unwrap();
    let TopLevel::MacroDecl(decl) = &program.module.top_levels[0] else { panic!("expected declaration") };
    let Statement::Expression(Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary))) = &body(&program).statements[0] else { panic!("expected expression") };
    let Operand::MacroInvoc(invocation) = &primary.operand else { panic!("expected call") };
    let context = MacroExpansionContext::new(&config);
    registry::MacroRegistry::build(&program.module, &context).unwrap();
    let generated_parser::GeneratedSyntax::Expression(expression) = expand_declarative(decl, invocation, &context).unwrap() else { panic!("expected expression result") };
    struct Calls(Vec<MacroInvoc>);
    impl<'a> crate::ast::visit::Visitor<'a> for Calls {
        fn visit_macro_invoc(&mut self, invocation: &'a MacroInvoc) { self.0.push(invocation.clone()); }
    }
    let mut calls = Calls(Vec::new());
    crate::ast::visit::walk_expression(&mut calls, &expression);
    assert_eq!(calls.0.len(), 1);
    assert_eq!(calls.0[0].args.runs().iter().map(|run| run.text()).collect::<String>(), "        \"opaque ([} 🦀\n");
    assert!(calls.0[0].parent.is_some());
}

#[test]
fn macro_a_06_repeated_array_opaque_values_cannot_become_semicolon_operators() {
    let source = "main = -> [(%dsl\n    \"opaque ([}\n); 2]\n";
    let result = crate::parser::parse_string(source, &crate::Config::default());
    match result {
        Ok(program) => {
            let Statement::Expression(Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary))) = &body(&program).statements[0] else { panic!("expected expression") };
            assert!(matches!(primary.operand, Operand::Literal(Literal { kind: LiteralKind::ArrayRepeat { len: 2, .. }, .. })));
        }
        Err(error) => assert!(format!("{error:?}").contains("repeat"), "{error:?}"),
    }
}

#[test]
fn macro_a_06_multiline_rock_operands_can_be_captured_and_forwarded() {
    let program = expand("macro id\n    $value:expr =>\n        $value\nmain = ->\n    %id (if true\n        1\n    else\n        2\n    )\n").unwrap();
    assert!(!format!("{:?}", body(&program)).contains("MacroInvoc"));
}

fn generated_expression(input: &TokenStream) -> Result<Expression, Diagnostics> {
    let generated_parser::GeneratedSyntax::Expression(expression) = generated_parser::parse_generated_source(
        input, &crate::Config::default(), MacroOutputCategory::Expression, None)? else { panic!("expected expression") };
    Ok(expression)
}

#[test]
fn macro_a_06_fragment_newlines_do_not_change_the_surrounding_header_margin() {
    let fragment = source("if true\n    1\nelse\n    2").expression_fragment(0, MACRO_LIMITS).unwrap();
    let mut output = source("f ");
    append(&mut output, &fragment).unwrap();
    append(&mut output, &source(", (%dsl\n    \"opaque ([}\n)\n")).unwrap();
    generated_expression(&output).unwrap();
}

#[test]
fn macro_a_06_block_capture_does_not_dedent_inside_an_expression_fragment() {
    let fragment = source("if true\n    1\nelse\n    2").expression_fragment(0, MACRO_LIMITS).unwrap();
    let mut output = source("%dsl\n    prefix ");
    append(&mut output, &fragment).unwrap();
    append(&mut output, &source("\n    end\n")).unwrap();
    let expression = generated_expression(&output).unwrap();
    let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = expression else { panic!("expected primary") };
    let Operand::MacroInvoc(invocation) = primary.operand else { panic!("expected invocation") };
    let text = invocation.args.runs().iter().map(|run| run.text()).collect::<String>();
    assert_eq!(text, "    prefix if true\n    1\nelse\n    2\n    end\n");
}

#[test]
fn macro_a_06_fragment_boundaries_are_not_validation_or_binding_authority() {
    for text in ["42\n43", "42\0ignored", "\"broken", "other = -> 1"] {
        let marked = source(text).expression_fragment(0, MACRO_LIMITS).unwrap();
        assert!(generated_expression(&marked).is_err(), "{text:?}");
    }
}

#[test]
fn macro_a_06_literal_matchers_do_not_request_group_or_nested_macro_parsing() {
    let program = expand("macro open\n    ( =>\n        main = -> 42\n%open (\n").unwrap();
    assert_eq!(body(&program).statements.len(), 1);
    let program = expand("macro token\n    %inner $value:expr =>\n        main = -> $value\n%token %inner 42\n").unwrap();
    assert_eq!(body(&program).statements.len(), 1);
}

#[test]
fn macro_a_06_relative_layout_rejects_underflow_instead_of_clamping_it() {
    let input = source("if true\n        1\n  else\n        2").expression_fragment(4, MACRO_LIMITS).unwrap();
    assert!(generated_expression(&input).is_err());
    // Ordinary dedented closing wrappers remain legal for grouped captures.
    let input = source("(if true\n        1\n    else\n        2\n)").expression_fragment(4, MACRO_LIMITS).unwrap();
    generated_expression(&input).unwrap();
}

#[test]
fn macro_a_06_relocating_a_capture_under_generated_layout_keeps_dsl_bytes() {
    let config = crate::Config::default();
    let program = crate::parser::parse_string("macro wrap\n    $value:expr =>\n        if true\n            $value\n        else\n            0\nmain = ->\n    %wrap (%dsl\n        \"opaque ([} 🦀\n    )\n", &config).unwrap();
    let TopLevel::MacroDecl(decl) = &program.module.top_levels[0] else { panic!("expected declaration") };
    let Statement::Expression(Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary))) = &body(&program).statements[0] else { panic!("expected expression") };
    let Operand::MacroInvoc(invocation) = &primary.operand else { panic!("expected call") };
    let context = MacroExpansionContext::new(&config);
    registry::MacroRegistry::build(&program.module, &context).unwrap();
    let generated_parser::GeneratedSyntax::Expression(expression) = expand_declarative(decl, invocation, &context).unwrap() else { panic!("expected expression") };
    let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(primary)) = expression else { panic!("expected primary") };
    let Operand::If(branch) = primary.operand else { panic!("expected generated branch") };
    struct Calls(Vec<MacroInvoc>);
    impl<'a> crate::ast::visit::Visitor<'a> for Calls {
        fn visit_macro_invoc(&mut self, invocation: &'a MacroInvoc) { self.0.push(invocation.clone()); }
    }
    let mut calls = Calls(Vec::new());
    crate::ast::visit::walk_block(&mut calls, &branch.then);
    assert_eq!(calls.0.len(), 1);
    assert_eq!(calls.0[0].args.runs().iter().map(|run| run.text()).collect::<String>(), "        \"opaque ([} 🦀\n");
}

use crate::ast::{Program, TopLevel};
use crate::Config;

use super::{expand_macros_with_context, ExpansionLimits, MacroExpansionContext};

fn parse(text: &str) -> Program {
    crate::parser::parse_string(text, &Config::default()).unwrap()
}

fn fail(text: &str, limits: ExpansionLimits, expected: &str) {
    let config = Config::default();
    let context = MacroExpansionContext::new(&config).with_limits(limits);
    let diagnostics = expand_macros_with_context(parse(text), &context).unwrap_err();
    assert!(format!("{diagnostics:?}").contains(expected), "{diagnostics:?}");
}

#[test]
fn macro_a_10_many_siblings_share_the_invocation_budget() {
    fail("macro number\n    =>\n        42\nmain = ->\n    %number!\n    %number!\n", ExpansionLimits { max_invocations: 1, ..ExpansionLimits::default() }, "invocation budget");
}

#[test]
fn macro_a_10_spelling_trivia_and_output_segments_are_aggregate_resources() {
    let text = "macro make\n    =>\n        value = -> 42 /* long retained trivia */\n%make\n";
    fail(text, ExpansionLimits { max_output_bytes: 12, ..ExpansionLimits::default() }, "output byte budget");
    fail(text, ExpansionLimits { max_output_units: 1, ..ExpansionLimits::default() }, "output unit budget");
}

#[test]
fn macro_a_10_matching_limits_are_fatal_and_do_not_fall_through_to_another_arm() {
    fail("macro choose\n    $value:expr =>\n        $value\n    $other:expr =>\n        0\nmain = -> %choose 42\n", ExpansionLimits { max_matching_work: 1, ..ExpansionLimits::default() }, "matching work budget");
}

#[test]
fn macro_a_10_equal_spans_have_independent_ids_and_reusing_one_id_rejects() {
    let config = Config::default();
    let mut program = parse("macro make\n    =>\n        made = -> 42\n%make\n%make\n");
    let TopLevel::MacroInvoc(first) = program.module.top_levels[1].clone() else { panic!("call") };
    let TopLevel::MacroInvoc(second) = &mut program.module.top_levels[2] else { panic!("call") };
    second.span = first.span.clone();
    expand_macros_with_context(program.clone(), &MacroExpansionContext::new(&config)).unwrap();
    let TopLevel::MacroInvoc(second) = &mut program.module.top_levels[2] else { panic!("call") };
    second.id = first.id;
    let diagnostics = expand_macros_with_context(program, &MacroExpansionContext::new(&config)).unwrap_err();
    assert!(format!("{diagnostics:?}").contains("already executed"));
}

#[test]
fn macro_a_10_mutual_recursion_uses_parent_depth_not_rounds_or_unrelated_siblings() {
    let config = Config::default();
    let context = MacroExpansionContext::new(&config).with_max_depth(2);
    let diagnostics = expand_macros_with_context(parse("macro one\n    =>\n        %two\nmacro two\n    =>\n        %one\nmacro sibling\n    =>\n        unrelated = -> 0\n%one\n%sibling\n"), &context).unwrap_err();
    let labels = format!("{diagnostics:?}");
    assert!(labels.contains("depth exceeded"));
    assert!(labels.contains("expanded from macro 'one'"));
    assert!(labels.contains("expanded from macro 'two'"));
    assert!(!labels.contains("expanded from macro 'sibling'"));
}

#[test]
fn macro_a_10_wide_generated_modules_and_source_discovery_are_bounded() {
    let config = Config {
        entry_file: "/macro-budget/main.rk".into(), current_crate_name: Some("app".into()),
        ..Config::default()
    };
    let mut sources = crate::source_loader::SourceDatabase::new();
    sources.add_virtual_source(config.entry_file.clone(), "macro setup\n    =>\n        mod one\n        mod two\n%setup\n");
    sources.add_virtual_source("/macro-budget/one.rk".into(), "value = -> 1\n");
    sources.add_virtual_source("/macro-budget/two.rk".into(), "value = -> 2\n");
    let mut graph = sources.load_entry(config.entry_file.clone(), &config).unwrap();
    let initial_modules = graph.modules().count();
    let context = MacroExpansionContext::new(&config).with_limits(ExpansionLimits { max_source_discovery_work: 1, ..ExpansionLimits::default() });
    let diagnostics = super::expand_macros_in_graph(&mut graph, &mut sources, &context).unwrap_err();
    assert!(format!("{diagnostics:?}").contains("source discovery budget"), "{diagnostics:?}");
    assert!(format!("{diagnostics:?}").contains("expanded from macro 'setup'"));
    assert!(graph.modules().count() <= initial_modules + 1);
}

#[test]
fn macro_a_10_forged_or_cyclic_parent_edges_are_diagnostics() {
    let config = Config::default();
    for cyclic in [false, true] {
        let context = MacroExpansionContext::new(&config);
        let mut input = parse("macro make\n    =>\n        made = -> 42\n%make\n");
        let TopLevel::MacroInvoc(call) = &mut input.module.top_levels[1] else { panic!("call") };
        call.parent = Some(super::ExpansionId(1));
        if cyclic {
            context.record_expansion(super::MacroExpansionRecord {
                invocation: crate::ast::InvocationId::fresh().unwrap(), macro_name: "cycle".into(),
                invocation_span: call.span.clone(), definition_span: None, parent: Some(super::ExpansionId(1)),
            });
        }
        let diagnostics = expand_macros_with_context(input, &context).unwrap_err();
        assert!(format!("{diagnostics:?}").contains(if cyclic { "cyclic" } else { "unknown macro expansion parent" }));
    }
}

#[test]
fn macro_a_10_generated_source_bytes_are_charged_before_parsing() {
    let config = Config { entry_file: "/macro-source-budget/main.rk".into(), ..Config::default() };
    let mut sources = crate::source_loader::SourceDatabase::new();
    sources.add_virtual_source(config.entry_file.clone(), "macro setup\n    =>\n        mod huge\n%setup\n");
    sources.add_virtual_source("/macro-source-budget/huge.rk".into(), format!("// {}\nthis is not valid Rock", "retained trivia".repeat(128)));
    let mut graph = sources.load_entry(config.entry_file.clone(), &config).unwrap();
    let context = MacroExpansionContext::new(&config).with_limits(ExpansionLimits { max_source_bytes: 10, ..ExpansionLimits::default() });
    let diagnostics = super::expand_macros_in_graph(&mut graph, &mut sources, &context).unwrap_err();
    let message = format!("{diagnostics:?}");
    assert!(message.contains("source byte budget"), "{message}");
    assert!(message.contains("expanded from macro 'setup'"));
}

#[test]
fn macro_a_10_collection_boundary_rejects_any_remaining_invocation() {
    let input = parse("struct Point\n    value: Int = %unexpanded!\n");
    assert!(super::validate_no_invocations(&input.module).is_err());
}

#[test]
fn macro_a_10_named_graph_root_is_one_executable_identity() {
    let config = Config { entry_file: "/macro-root-budget/main.rk".into(), current_crate_name: Some("app".into()), ..Config::default() };
    let mut sources = crate::source_loader::SourceDatabase::new();
    sources.add_virtual_source(config.entry_file.clone(), "macro make\n    =>\n        main = -> 42\n%make\n");
    let mut graph = sources.load_entry(config.entry_file.clone(), &config).unwrap();
    let context = MacroExpansionContext::new(&config).with_limits(ExpansionLimits { max_invocations: 1, ..ExpansionLimits::default() });
    super::expand_macros_in_graph(&mut graph, &mut sources, &context).unwrap();
    assert_eq!(context.latest_expansion_id(), Some(super::ExpansionId(1)));
    assert!(graph.root_module().top_level_from_ident("main").is_some());
    assert!(graph.module_by_qualified_name("app").unwrap().module.top_level_from_ident("main").is_some());
}

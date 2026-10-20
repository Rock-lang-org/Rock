use crate::ast::{Program, TopLevel};
use crate::Config;

use super::{expand_macros_with_context, MacroExpansionContext};

fn parse(text: &str) -> Program {
    crate::parser::parse_string(text, &Config::default()).unwrap()
}

#[test]
fn macro_a_09_generated_inline_definitions_and_imports_settle_before_body_errors() {
    let config = Config::default();
    let input = parse("> tools::make\n%make\nmacro setup\n    $module:ident, $name:ident =>\n        mod $module\n            < $name\n            macro $name\n                =>\n                    main = -> %value!\n            macro value\n                =>\n                    42\n%setup tools, make\n");
    let context = MacroExpansionContext::new(&config);
    let output = expand_macros_with_context(input, &context).unwrap();
    assert!(matches!(output.module.top_level_from_ident("main"), Some(TopLevel::FunctionDecl(_))));
    assert_eq!(context.latest_expansion_id(), Some(super::ExpansionId(3)));
}

#[test]
fn macro_a_09_generated_exports_can_make_an_initially_private_import_available() {
    let config = Config::default();
    let input = parse("> tools::make\n%make\nmod tools\n    macro make\n        =>\n            main = -> 42\n    macro publish\n        $name:ident =>\n            < $name\n    %publish make\n");
    let context = MacroExpansionContext::new(&config);
    assert!(expand_macros_with_context(input, &context).unwrap().module.top_level_from_ident("main").is_some());
    assert_eq!(context.latest_expansion_id(), Some(super::ExpansionId(2)));
}

#[test]
fn macro_a_09_generated_file_modules_are_discovered_and_expanded_before_collection() {
    let config = Config {
        entry_file: "/macro-schedule/main.rk".into(),
        current_crate_name: Some("app".into()), no_prelude: true,
        source_providers: vec![
            crate::SourceProvider::Virtual { path: "/macro-schedule/main.rk".into(), text: "> tools::make\n%make\nmacro setup\n    $name:ident =>\n        mod $name\n%setup tools\n".into() },
            crate::SourceProvider::Virtual { path: "/macro-schedule/tools.rk".into(), text: "< make\nmacro make\n    =>\n        main = -> work!\nmacro setup\n    $name:ident =>\n        mod $name\n%setup inner\n< work = -> inner::answer!\n".into() },
            crate::SourceProvider::Virtual { path: "/macro-schedule/inner.rk".into(), text: "< answer = -> 42\n".into() },
        ], ..Config::default()
    };
    crate::analyze(&config).unwrap();
}

#[test]
fn macro_a_09_generated_imports_are_available_before_expression_expansion() {
    let config = Config::default();
    let input = parse("main = -> %number!\nmacro imports\n    $module:ident, $name:ident =>\n        > $module::$name\n%imports tools, number\nmod tools\n    < number\n    macro number\n        =>\n            42\n");
    let context = MacroExpansionContext::new(&config);
    expand_macros_with_context(input, &context).unwrap();
    assert_eq!(context.latest_expansion_id(), Some(super::ExpansionId(2)));
}

#[test]
fn macro_a_09_no_progress_and_import_cycles_do_not_consume_the_depth_budget() {
    let config = Config::default();
    for (text, expected) in [("%missing\n", "no progress"), ("> second::first\n> first::second\n%first::make\n", "import cycle")] {
        let context = MacroExpansionContext::new(&config).with_max_depth(50);
        let diagnostics = expand_macros_with_context(parse(text), &context).unwrap_err();
        assert!(format!("{diagnostics:?}").contains(expected), "{diagnostics:?}");
        assert!(!format!("{diagnostics:?}").contains("depth exceeded"));
        assert_eq!(context.latest_expansion_id(), None);
    }
}

#[test]
fn macro_a_09_loaded_registration_order_does_not_change_resolution_or_execution() {
    let config = Config { current_crate_name: Some("app".into()), ..Config::default() };
    let mut sources = crate::source_loader::SourceDatabase::new();
    sources.add_virtual_source("/macro-schedule-order/main.rk".into(), "mod one\nmod two\n");
    sources.add_virtual_source("/macro-schedule-order/one.rk".into(), "macro item\n    =>\n        value = -> 1\n%item\n");
    sources.add_virtual_source("/macro-schedule-order/two.rk".into(), "macro item\n    =>\n        value = -> 2\n%item\n");
    let graph = sources.load_entry("/macro-schedule-order/main.rk".into(), &config).unwrap();
    let mut modules = graph.modules().cloned().collect::<Vec<_>>();
    let input = Program { module: graph.root_module().clone() };
    let first = MacroExpansionContext::new(&config).with_source_modules(modules.clone().into());
    expand_macros_with_context(input.clone(), &first).unwrap();
    modules.reverse();
    let second = MacroExpansionContext::new(&config).with_source_modules(modules.into());
    expand_macros_with_context(input, &second).unwrap();
    assert_eq!(first.trace_labels(super::ExpansionId(1)), second.trace_labels(super::ExpansionId(1)));
    assert_eq!(first.latest_expansion_id(), Some(super::ExpansionId(2)));
    assert_eq!(second.latest_expansion_id(), Some(super::ExpansionId(2)));
}

#[test]
fn macro_a_09_errors_in_newly_discovered_sources_keep_their_source_snapshot() {
    let config = Config {
        entry_file: "/macro-schedule-error/main.rk".into(),
        current_crate_name: Some("app".into()), no_prelude: true,
        source_providers: vec![
            crate::SourceProvider::Virtual { path: "/macro-schedule-error/main.rk".into(), text: "macro setup\n    =>\n        mod tools\n%setup\nmain = -> 0\n".into() },
            crate::SourceProvider::Virtual { path: "/macro-schedule-error/tools.rk".into(), text: "work = -> %missing!\n".into() },
        ], ..Config::default()
    };
    let diagnostics = crate::analyze(&config).unwrap_err();
    assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.source.as_ref()
        .is_some_and(|source| source.text == "work = -> %missing!\n")));
}

#[test]
fn macro_a_09_successful_native_calls_are_removed_before_later_resolution_rounds() {
    let config = Config::default();
    let input = parse("%provider::native\n%later\nmacro setup\n    $name:ident =>\n        macro $name\n            =>\n                main = -> 42\n%setup later\n");
    let (provider, directory) = super::tests::proc_macro_artifact_with_response("native", super::tests::constructed_response(&super::tests::first_invocation(&input), "native_result = -> 7\n"));
    let context = MacroExpansionContext::new(&config).with_named_proc_macro_artifact("provider".into(), provider);
    let result = expand_macros_with_context(input, &context);
    std::fs::remove_dir_all(directory).unwrap();
    let result = result.unwrap();
    assert!(result.module.top_level_from_ident("native_result").is_some());
    assert!(result.module.top_level_from_ident("main").is_some());
    assert_eq!(context.latest_expansion_id(), Some(super::ExpansionId(3)));
}

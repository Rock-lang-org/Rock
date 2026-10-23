use super::*;
use super::declarative::{CaptureSet, MacroTemplate, TemplateFragment};
use super::token_tree::{append, source};

fn text(input: &TokenStream) -> String {
    input.runs().iter().map(|run| run.text()).collect()
}

fn expand(code: &str) -> Result<Program, Diagnostics> {
    let config = crate::Config::default();
    let program = crate::parser::parse_string(code, &config).map_err(Diagnostics::from)?;
    expand_macros_with_context(program, &MacroExpansionContext::new(&config))
}

fn matches(head: &str, input: &TokenStream) -> Result<Correspondance, Diagnostics> {
    let config = crate::Config::default();
    let program = crate::parser::parse_string(&format!("macro m\n    {head} =>\n        main = -> 0\n"), &config).unwrap();
    let TopLevel::MacroDecl(decl) = &program.module.top_levels[0] else { panic!("expected declaration") };
    let compiled = DeclarativeMacro::from_ast(decl);
    let context = MacroExpansionContext::new(&config);
    let (tokens, ranges) = macro_arg_matcher::rock_arguments(input, &context, 0)?;
    MacroArgMatcher::new(&tokens, input, 0, &ranges, compiled.arms[0].matcher.fragments.clone(),
        decl.name.span.clone(), input.start_span(), &context).run()
        .map_err(macro_arg_matcher::MatchError::into_diagnostics)
}

#[test]
fn macro_a_05_matching_ignores_trivia_but_capture_retains_internal_bytes() {
    let input = source("\r\n   1 /* caller 🦀 */ +  2\r\n// trailing\r\n");
    let captures = matches("$value:expr", &input).unwrap();
    let captured = &captures.entries["value"][0];
    assert_eq!(text(captured), "1 /* caller 🦀 */ +  2");
    assert_eq!(captured.start_span().start, 5);
    assert_eq!(captured.runs()[0].context(), input.runs()[0].context());
    // Layout is ignored by the flat matcher, not narrowed to u8 first.
    matches("$name:ident", &source(&format!("\n{}name\n", " ".repeat(512)))).unwrap();
}

#[test]
fn macro_a_05_each_arm_consumes_all_significant_input() {
    for (head, input) in [("$name:ident", "one two"), ("$value:expr", "1, 2"),
        ("$ty:ty", "Vec I64"), ("literal", "literal other")] {
        assert!(matches(head, &source(input)).is_err(), "{head}: {input}");
    }
    matches("$ty:ty", &source("I64")).unwrap();
}

#[test]
fn macro_a_05_repetition_does_not_accept_a_partial_group_at_eof() {
    assert!(matches("$( $name:ident , )*", &source("one")).is_err());
    let captures = matches("$( $name:ident , )*", &source("one, two,")).unwrap();
    assert_eq!(captures.get("name", 1).unwrap().iter().map(text).collect::<Vec<_>>(), ["one", "two"]);
}

#[test]
fn macro_a_05_equal_spelled_and_duplicated_capture_occurrences_stay_independent() {
    let captures = matches("$name:ident $name:ident", &source("same same")).unwrap();
    assert_eq!(captures.entries["name"].len(), 2);
    assert_ne!(captures.entries["name"][0].start_span(), captures.entries["name"][1].start_span());
    let same = source("same");
    let mut duplicated = same.clone();
    append(&mut duplicated, &same).unwrap();
    append(&mut duplicated, &same).unwrap();
    let captures = matches("$name:ident $name:ident $name:ident", &duplicated).unwrap();
    assert_eq!(captures.entries["name"].len(), 3);
    let captured = capture_set_from_correspondance(&captures, ExpansionId(3), &Span::test());
    assert_eq!(captured.get_direct("name").unwrap().len(), 3);
}

#[test]
fn macro_a_05_flat_repetition_keeps_each_iteration_and_stable_key_groups() {
    let captures = matches("$( $z:ident , $a:ident ; )*", &source("same , same ; same , same ; ")).unwrap();
    assert_eq!(captures.nested_corresp.len(), 1);
    assert_eq!(captures.nested_corresp[0].keys(), ["a", "z"]);
    for name in ["a", "z"] {
        let values = captures.get(name, 1).unwrap();
        assert_eq!(values.len(), 2);
        assert_ne!(values[0].start_span(), values[1].start_span());
    }
}

#[test]
fn macro_a_05_nested_repetitions_reject_during_definition_validation_even_unused() {
    for code in ["macro bad\n    $( $( $name:ident )* )* =>\n        main = -> 0\n",
        "macro bad\n    $name:ident =>\n        $( $( $name = -> 0 )* )*\n"] {
        let diagnostics = expand(code).unwrap_err();
        assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.message.contains("nested repetition")), "{diagnostics:?}");
    }
}

#[test]
fn macro_review_deep_repetitions_reject_at_the_second_opening() {
    let nested = format!("{}$name:ident{}", "$(".repeat(10_000), ")*".repeat(10_000));
    for code in [format!("macro bad\n    {nested} =>\n        ()\n"),
        format!("macro bad\n    =>\n        {nested}\n")] {
        let error = crate::parser::parse_string(&code, &crate::Config::default()).unwrap_err();
        let crate::parser::ParseError::HardError(message, span) = error else { panic!("{error:?}") };
        assert!(message.contains("nested repetition"), "{message}");
        assert_eq!(span.start, code.find("$(").unwrap() + 2);
    }
}

#[test]
fn macro_a_05_empty_repetition_cannot_silently_make_no_progress() {
    let diagnostics = matches("$( )*", &source("")).unwrap_err();
    assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.message.contains("progress")), "{diagnostics:?}");
}

#[test]
fn macro_a_05_empty_definitions_cannot_return_empty_diagnostics() {
    let diagnostics = expand("macro empty\n%empty\n").unwrap_err();
    assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.message.contains("at least one arm")), "{diagnostics:?}");
}

#[test]
fn macro_a_05_ambiguous_flat_repetition_splits_diagnose() {
    let diagnostics = matches("$( $a:ident )* $( $b:ident )*", &source("one two")).unwrap_err();
    assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.message.contains("ambiguous")), "{diagnostics:?}");
    let diagnostics = expand("macro split\n    $( $a:ident )* $( $b:ident )* =>\n        main = -> 1\n    $value:expr =>\n        main = -> 2\n%split one two\n").unwrap_err();
    assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.message.contains("ambiguous")), "{diagnostics:?}");
}

#[test]
fn macro_a_05_mismatched_template_lengths_and_nested_templates_reject() {
    let mut captures = CaptureSet::new();
    captures.insert_repeated("a".into(), vec![source("one"), source("two")]);
    captures.insert_repeated("b".into(), vec![source("other")]);
    let body = vec![TemplateFragment::Capture { name: "a".into() }, TemplateFragment::Capture { name: "b".into() }];
    let template = MacroTemplate { input: source(""), fragments: vec![TemplateFragment::Repetition(body)] };
    assert!(template.expand(&captures).unwrap_err().contains("mismatched"));
    let template = MacroTemplate { input: source(""), fragments: vec![TemplateFragment::Repetition(vec![TemplateFragment::Repetition(vec![TemplateFragment::Capture { name: "a".into() }])])] };
    assert!(template.expand(&captures).unwrap_err().contains("nested repetition"));
}

#[test]
fn macro_a_05_selected_empty_output_is_success_not_arm_fallthrough() {
    let expanded = expand("macro empty\n    =>\n        $( $name = -> 1 )*\n    =>\n        fallback = -> 2\n%empty\n").unwrap();
    assert_eq!(expanded.module.top_levels.len(), 1);
    let expanded = expand("macro choose\n    $name:ident =>\n        first = -> 1\n    $name:ident =>\n        second = -> 2\n%choose name\n").unwrap();
    assert!(expanded.module.top_level_from_ident("first").is_some());
    assert!(expanded.module.top_level_from_ident("second").is_none());
}

#[test]
fn macro_a_05_non_rock_block_input_gets_a_structured_match_diagnostic() {
    let diagnostics = expand("macro parse\n    $value:expr =>\n        main = -> $value\n%parse\n    \"not Rock ([}\n").unwrap_err();
    assert!(!diagnostics.0.is_empty());
    assert!(diagnostics.0.iter().any(|diagnostic| diagnostic.message.contains("Rock macro arguments")), "{diagnostics:?}");
}

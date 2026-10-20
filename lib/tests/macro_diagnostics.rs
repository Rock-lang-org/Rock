use rock_lib::diagnostic::{Diagnostic, DiagnosticCode, DiagnosticLocation, Diagnostics};
use rock_lib::{Config, SourceProvider};

fn config(text: &str) -> Config {
    let entry_file = "/macro-a15/main.rk".into();
    Config { entry_file, no_std: true, no_prelude: true,
        source_providers: vec![SourceProvider::Virtual { path: "/macro-a15/main.rk".into(), text: text.into() }],
        ..Config::default() }
}

fn traced<'a>(diagnostics: &'a Diagnostics, names: &[&str]) -> &'a Diagnostic {
    diagnostics.0.iter().find(|diagnostic| names.iter().all(|name|
        diagnostic.secondary.iter().any(|label| label.message == format!("expanded from macro '{name}'"))))
        .unwrap_or_else(|| panic!("missing ancestry {names:?}: {diagnostics:?}"))
}

fn valid_snapshot(diagnostic: &Diagnostic) {
    diagnostic.validate().unwrap();
    let DiagnosticLocation::Source(span) = &diagnostic.location else { panic!("source diagnostic: {diagnostic:?}") };
    let source = diagnostic.source.as_ref().expect("retained source text");
    assert!(source.text.get(span.start..span.end).is_some(), "{diagnostic:?}");
    for label in &diagnostic.secondary {
        let text = if label.span.file_path == span.file_path { &source.text }
            else { &source.related.get(&label.span.file_path).expect("related source snapshot").text };
        assert!(text.get(label.span.start..label.span.end).is_some(), "{label:?}");
    }
}

#[test]
fn macro_a_15_inference_failures_keep_nested_ancestry_and_definition_ranges() {
    let source = "macro inner\n    =>\n        take true\nmacro outer\n    =>\n        %inner!\ntake: I64 -> I64\ntake = value -> value\nmain = -> %outer!\n";
    let diagnostics = rock_lib::analyze(&config(source)).unwrap_err();
    let diagnostic = traced(&diagnostics, &["inner", "outer"]);
    valid_snapshot(diagnostic);
    assert_ne!(diagnostic.code, Some(DiagnosticCode::Macro));
    let DiagnosticLocation::Source(span) = &diagnostic.location else { unreachable!() };
    assert_eq!(span.file_path.to_str(), Some("/macro-a15/main.rk"));
    assert_eq!(span.start, source.find("%outer!").unwrap(), "{diagnostic:?}");
    assert!(diagnostic.secondary.iter().any(|label| label.message == "expanded syntax here"
        && label.span.start < source.find("macro outer").unwrap()));
}

#[test]
fn macro_a_15_forwarded_failures_point_at_caller_without_sibling_ancestry() {
    let source = "macro id\n    $value:expr =>\n        $value\nmain = ->\n    ok = %id 1\n    bad = %id missing\n    ok\n";
    let diagnostics = rock_lib::analyze(&config(source)).unwrap_err();
    let diagnostic = traced(&diagnostics, &["id"]);
    valid_snapshot(diagnostic);
    let DiagnosticLocation::Source(span) = &diagnostic.location else { unreachable!() };
    assert_eq!(&source[span.start..span.end], "missing");
    let calls = diagnostic.secondary.iter().filter(|label| label.message == "expanded from macro 'id'").collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].span.start >= source.find("bad =").unwrap());
}

#[test]
fn macro_a_15_borrow_failures_map_primary_and_ownership_labels() {
    let source = "macro conflict\n    =>\n        if true\n            mut value = 1\n            loan = &mut value\n            value = 3\n            *loan = 4\n        else\n            ()\nmain = !-> %conflict!\n";
    let diagnostics = rock_lib::compile(&config(source)).unwrap_err();
    let diagnostic = traced(&diagnostics, &["conflict"]);
    assert_eq!(diagnostic.code, Some(DiagnosticCode::Borrow));
    valid_snapshot(diagnostic);
}

#[test]
fn macro_a_15_expansion_inspection_stops_before_typing_but_uses_the_loaded_forest() {
    let mut config = config("macro setup\n    $name:ident =>\n        mod $name\n%setup tools\nmain = -> %tools::number!\n");
    config.source_providers.push(SourceProvider::Virtual { path: "/macro-a15/tools.rk".into(),
        text: "< number\nmacro number\n    =>\n        missing\n".into() });
    let expansion = rock_lib::expand(&config).unwrap();
    assert!(!expansion.ast.module.has_macro_invoc());
    assert_eq!(expansion.modules.len(), 1);
    assert!(!expansion.modules[0].module.has_macro_invoc());
    let diagnostics = rock_lib::analyze(&config).unwrap_err();
    valid_snapshot(traced(&diagnostics, &["number"]));
}

#[test]
fn macro_a_15_generated_parse_failures_retain_valid_source_snapshots() {
    let diagnostics = rock_lib::expand(&config("macro broken\n    =>\n        main = ->\n%broken\n")).unwrap_err();
    valid_snapshot(traced(&diagnostics, &["broken"]));
}

#[test]
fn macro_a_15_unsafe_rules_are_not_bypassed_by_generated_calls() {
    let source = "unsafe secret = -> 42\nmacro call\n    =>\n        secret!\nmain = -> %call!\n";
    let diagnostics = rock_lib::analyze(&config(source)).unwrap_err();
    let diagnostic = traced(&diagnostics, &["call"]);
    assert!(diagnostic.message.to_lowercase().contains("unsafe"), "{diagnostic:?}");
    valid_snapshot(diagnostic);
}

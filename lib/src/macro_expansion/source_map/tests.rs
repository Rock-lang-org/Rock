use super::*;

use crate::lexer::Span;

fn span(start: usize, end: usize) -> Span {
    Span {
        file_path: "macro_test.rk".into(),
        start,
        end,
    }
}

#[test]
fn expansion_source_map_records_invocation_definition_and_parent() {
    let mut map = MacroSourceMap::new();
    let parent = map.record_expansion(MacroExpansionRecord {
        macro_name: "outer".to_string(),
        invocation_span: span(1, 7),
        definition_span: Some(span(10, 20)),
        parent: None,
    });
    let child = map.record_expansion(MacroExpansionRecord {
        macro_name: "inner".to_string(),
        invocation_span: span(30, 36),
        definition_span: Some(span(40, 50)),
        parent: Some(parent),
    });

    let trace = map.trace(child);

    assert_eq!(trace.len(), 2);
    assert_eq!(trace[0].macro_name, "inner");
    assert_eq!(trace[1].macro_name, "outer");
}

#[test]
fn expansion_trace_converts_to_diagnostic_labels() {
    let mut map = MacroSourceMap::new();
    let id = map.record_expansion(MacroExpansionRecord {
        macro_name: "make".to_string(),
        invocation_span: Span::test(),
        definition_span: Some(Span::test()),
        parent: None,
    });

    let labels = map.trace_labels(id);

    assert!(labels
        .iter()
        .any(|(label, _)| label.contains("expanded from macro 'make'")));
}

#[test]
fn expansion_trace_omits_definition_label_without_definition_location() {
    let mut map = MacroSourceMap::new();
    let id = map.record_expansion(MacroExpansionRecord {
        macro_name: "make".to_string(),
        invocation_span: span(1, 7),
        definition_span: None,
        parent: None,
    });

    let labels = map.trace_labels(id);

    assert!(!labels
        .iter()
        .any(|(label, span)| label == "macro definition here"
            && span.file_path.as_os_str().is_empty()
            && span.start == 0
            && span.end == 0));
}

#[test]
fn token_origin_distinguishes_capture_and_generated_tokens() {
    let expansion = ExpansionId(3);
    let source = TokenOrigin::Source(span(0, 1));
    let captured = TokenOrigin::Captured {
        expansion,
        invocation_span: span(1, 5),
        capture_span: span(6, 9),
    };
    let generated = TokenOrigin::Generated {
        expansion,
        generated_source: GeneratedSourceId(1),
        definition_span: span(10, 12),
    };

    assert_eq!(source.expansion_id(), None);
    assert_eq!(captured.expansion_id(), Some(expansion));
    assert_eq!(generated.expansion_id(), Some(expansion));
}

#[test]
fn trace_stops_when_parent_links_cycle() {
    let mut map = MacroSourceMap::new();
    let first = map.record_expansion(MacroExpansionRecord {
        macro_name: "first".to_string(),
        invocation_span: span(1, 7),
        definition_span: Some(span(10, 20)),
        parent: None,
    });
    let second = map.record_expansion(MacroExpansionRecord {
        macro_name: "second".to_string(),
        invocation_span: span(30, 36),
        definition_span: Some(span(40, 50)),
        parent: Some(first),
    });
    map.records[0].parent = Some(second);

    let trace = map.trace(second);

    assert_eq!(trace.len(), 2);
    assert_eq!(trace[0].macro_name, "second");
    assert_eq!(trace[1].macro_name, "first");
}

#[test]
fn recorded_expansion_has_generated_source_identity() {
    let mut map = MacroSourceMap::new();
    let expansion = map.record_expansion(MacroExpansionRecord {
        macro_name: "make_item".to_string(),
        invocation_span: span(1, 10),
        definition_span: Some(span(20, 40)),
        parent: None,
    });

    let generated_source = map
        .generated_source_id(expansion)
        .expect("expansion should have generated source identity");

    assert_eq!(generated_source, GeneratedSourceId(1));
    assert!(map.record(expansion).is_some());
}

#[test]
fn generated_token_origin_carries_generated_source_identity() {
    let generated_source = GeneratedSourceId(7);
    let origin = TokenOrigin::Generated {
        expansion: ExpansionId(2),
        generated_source,
        definition_span: span(10, 12),
    };

    assert_eq!(origin.generated_source_id(), Some(generated_source));
}

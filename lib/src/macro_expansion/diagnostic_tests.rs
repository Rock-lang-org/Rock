use crate::ast::InvocationId;
use crate::diagnostic::{Diagnostic, DiagnosticCode, DiagnosticLocation, DiagnosticSourceMap, Diagnostics};
use crate::lexer::lossless::{ConstructionContext, ConstructionKind, JoinBoundary};
use crate::lexer::Span;

use super::{ExpansionId, GeneratedSourceId, MacroExpansionRecord, MacroSourceMap, TokenStream};
use super::token_tree::{source, MACRO_LIMITS};

fn expansion(map: &mut MacroSourceMap, name: &str, span: Span, parent: Option<ExpansionId>) -> ExpansionId {
    map.record_expansion(MacroExpansionRecord { invocation: InvocationId::fresh().unwrap(), macro_name: name.into(),
        invocation_span: span, definition_span: None, parent })
}

#[test]
fn macro_a_15_codegen_mapping_keeps_phase_and_maps_secondary_nested_labels() {
    let original = source("first second");
    let mut map = MacroSourceMap::new();
    let outer = expansion(&mut map, "outer", original.start_span(), None);
    let output = original.project_output(outer, GeneratedSourceId(1), 0);
    map.record_output(outer, &original);
    let inner = expansion(&mut map, "inner", output.slice(6..12, MACRO_LIMITS).unwrap().start_span(), Some(outer));
    let forwarded = output.slice(6..12, MACRO_LIMITS).unwrap();
    map.record_output(inner, &forwarded);
    let projected = forwarded.project_output(inner, GeneratedSourceId(2), 0);
    let diagnostic = Diagnostic::new("backend rejected output".into(), projected.runs()[0].span()).with_code(DiagnosticCode::Codegen)
        .with_labels(vec![("related generated site".into(), output.runs()[0].span())]);
    let mapped = map.map_diagnostics(Diagnostics(vec![diagnostic]));
    let diagnostic = &mapped.0[0];
    assert_eq!(diagnostic.code, Some(DiagnosticCode::Codegen));
    assert!(matches!(&diagnostic.location, DiagnosticLocation::Source(span) if span.start == 6 && span.end == 12 && span.file_path == original.start_span().file_path));
    assert!(diagnostic.secondary.iter().any(|label| label.message == "expanded from macro 'inner'"));
    assert!(diagnostic.secondary.iter().any(|label| label.message == "expanded from macro 'outer'"));
    assert!(diagnostic.secondary.iter().all(|label| label.span.file_path == original.start_span().file_path));
}

#[test]
fn macro_a_15_distinct_constructed_snapshots_receive_disjoint_inspectable_offsets() {
    let invocation = InvocationId::fresh().unwrap();
    let request = ConstructionContext { invocation, kind: ConstructionKind::CallSite };
    let left = TokenStream::source_fragment("left".into(), request, MACRO_LIMITS).unwrap();
    let right = TokenStream::source_fragment("right".into(), request, MACRO_LIMITS).unwrap();
    let joined = left.join(&right, JoinBoundary::Separate, MACRO_LIMITS).unwrap();
    let resolved = joined.resolve_proc_output(&source(""), invocation, ExpansionId(1), GeneratedSourceId(1), |_| Ok(Default::default())).unwrap();
    let mut map = MacroSourceMap::new();
    let id = expansion(&mut map, "native", source("call").start_span(), None);
    map.record_output(id, &resolved);
    let output = resolved.project_output(id, GeneratedSourceId(1), 0);
    assert_eq!(output.runs()[0].origin().provenance, resolved.runs()[0].origin().provenance);
    assert_eq!(output.runs()[0].span().start..output.runs()[0].span().end, 0..4);
    assert_eq!(output.runs()[1].span().start..output.runs()[1].span().end, 4..9);
    let wire = bincode::serialize(&output).unwrap();
    let decoded: TokenStream = bincode::deserialize(&wire).unwrap();
    assert_eq!(decoded.runs()[1].span(), output.runs()[1].span());
    assert_eq!(decoded.runs()[1].origin(), output.runs()[1].origin());
    assert_eq!(decoded.slice(5..8, MACRO_LIMITS).unwrap().start_span().start, 5);
    let mut sources = DiagnosticSourceMap::default();
    sources.insert_virtual(source("call").start_span().file_path, "call".into());
    map.attach_sources(&mut sources);
    let diagnostic = Diagnostic::new("right fragment".into(), output.runs()[1].span());
    let mapped = map.map_diagnostics(Diagnostics(vec![diagnostic])).with_sources(&sources);
    let diagnostic = &mapped.0[0];
    assert_eq!(diagnostic.source.as_ref().unwrap().text, "call");
    assert_eq!(map.output_text(id), Some("leftright"));
    let output_label = diagnostic.secondary.iter().find(|label| label.message == "expanded syntax here").unwrap();
    assert_eq!(diagnostic.source.as_ref().unwrap().related[&output_label.span.file_path].text, "leftright");
    diagnostic.validate().unwrap();
}

#[test]
fn macro_a_15_consumer_maps_cannot_claim_provider_output_locations() {
    let provider = crate::Config { entry_file: "/provider/lib.rk".into(), ..Default::default() };
    let consumer = crate::Config { entry_file: "/consumer/main.rk".into(), ..Default::default() };
    let mut provider = MacroSourceMap::for_config(&provider);
    let mut consumer = MacroSourceMap::for_config(&consumer);
    let id = expansion(&mut provider, "provider", source("provider").start_span(), None);
    let other = expansion(&mut consumer, "consumer", source("consumer").start_span(), None);
    assert_eq!(id, other);
    provider.record_output(id, &source("provider"));
    consumer.record_output(other, &source("consumer"));
    let projected = source("provider").project_output(id, GeneratedSourceId(1), provider.namespace());
    let span = projected.runs()[0].span();
    let mapped = consumer.map_diagnostics(Diagnostics(vec![Diagnostic::new("provider failure".into(), span.clone())]));
    assert_eq!(mapped.0[0].location, DiagnosticLocation::Source(span));
    assert!(mapped.0[0].secondary.is_empty());
}

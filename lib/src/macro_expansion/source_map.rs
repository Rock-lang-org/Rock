use crate::lexer::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub struct ExpansionId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct GeneratedSourceId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TokenProvenance {
    Source(Span),
    Captured {
        expansion: ExpansionId,
        invocation_span: Span,
        capture_span: Span,
    },
    Generated {
        expansion: ExpansionId,
        generated_source: GeneratedSourceId,
        definition_span: Span,
    },
}

/// Logical diagnostic addresses never prove adjacency or binding identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct OutputProjection {
    pub namespace: u64,
    pub expansion: ExpansionId,
    pub generated_source: GeneratedSourceId,
    pub source_start: usize,
    pub output_start: usize,
}

/// Projection is independent of the original provenance retained for transport.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct TokenOrigin {
    pub provenance: TokenProvenance,
    pub projection: Option<OutputProjection>,
}

impl From<TokenProvenance> for TokenOrigin {
    fn from(provenance: TokenProvenance) -> Self { Self { provenance, projection: None } }
}

impl TokenOrigin {
    pub fn expansion_id(&self) -> Option<ExpansionId> {
        if let Some(projection) = self.projection { return Some(projection.expansion); }
        match &self.provenance {
            TokenProvenance::Source(_) => None,
            TokenProvenance::Captured { expansion, .. } | TokenProvenance::Generated { expansion, .. } => {
                Some(*expansion)
            }
        }
    }

    pub fn generated_source_id(&self) -> Option<GeneratedSourceId> {
        if let Some(projection) = self.projection { return Some(projection.generated_source); }
        match &self.provenance {
            TokenProvenance::Generated {
                generated_source, ..
            } => Some(*generated_source),
            TokenProvenance::Source(_) | TokenProvenance::Captured { .. } => None,
        }
    }

    pub(crate) fn output_span(&self, start: usize, end: usize) -> Option<Span> {
        let OutputProjection { namespace, expansion, source_start, output_start, .. } = self.projection?;
        Some(Span::new(Self::output_path(namespace, expansion), output_start.checked_add(start.checked_sub(source_start)?)?,
            output_start.checked_add(end.checked_sub(source_start)?)?))
    }

    pub(crate) fn output_path(namespace: u64, expansion: ExpansionId) -> std::path::PathBuf {
        format!("<macro-output:{namespace:016x}:{}>", expansion.0).into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroExpansionRecord {
    pub invocation: crate::ast::InvocationId,
    pub macro_name: String,
    pub invocation_span: Span,
    pub definition_span: Option<Span>,
    pub parent: Option<ExpansionId>,
}

#[derive(Debug, Clone, Default)]
pub struct MacroSourceMap {
    namespace: u64,
    records: Vec<MacroExpansionRecord>,
    generated_sources: Vec<GeneratedSourceId>,
    outputs: std::collections::BTreeMap<ExpansionId, OutputSource>,
}

#[derive(Debug, Clone)]
struct OutputSegment {
    range: std::ops::Range<usize>,
    source: Span,
    generated: bool,
}

#[derive(Debug, Clone)]
struct OutputSource {
    text: String,
    segments: Vec<OutputSegment>,
}

impl MacroSourceMap {
    pub(crate) fn for_config(config: &crate::Config) -> Self {
        use std::hash::{Hash, Hasher};
        // Imported body spans can outlive their compilation-local expansion IDs.
        // Never attribute a provider's output to a same-numbered consumer macro.
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        if config.entry_file.is_relative() { std::env::current_dir().ok().hash(&mut hash); }
        config.entry_file.hash(&mut hash);
        config.current_crate_name.hash(&mut hash);
        Self { namespace: hash.finish(), ..Self::default() }
    }

    pub(crate) fn namespace(&self) -> u64 { self.namespace }

    /// Inspection only; this text must not be reparsed to forward hygienic syntax.
    pub fn output_text(&self, expansion: ExpansionId) -> Option<&str> {
        self.outputs.get(&expansion).map(|output| output.text.as_str())
    }

    pub(crate) fn record_output(&mut self, expansion: ExpansionId, stream: &super::TokenStream) {
        let mut output = OutputSource { text: String::with_capacity(stream.len_bytes()), segments: Vec::new() };
        for run in stream.runs() {
            let start = output.text.len();
            output.text.push_str(run.text());
            let origin = run.origin();
            let generated = matches!(&origin.provenance, TokenProvenance::Generated { .. });
            let source = match origin.provenance {
                TokenProvenance::Generated { definition_span, .. } if origin.projection.is_none() => definition_span,
                _ => run.span(),
            };
            if let Some(previous) = output.segments.last_mut() {
                if previous.generated == generated && previous.source.file_path == source.file_path && previous.source.end == source.start {
                    previous.range.end = output.text.len(); previous.source.end = source.end;
                    continue;
                }
            }
            output.segments.push(OutputSegment { range: start..output.text.len(), source, generated });
        }
        self.outputs.insert(expansion, output);
    }

    fn expansion_at(&self, span: &Span) -> Option<ExpansionId> {
        let path = span.file_path.to_str()?;
        let (namespace, id) = path.strip_prefix("<macro-output:")?.strip_suffix('>')?.split_once(':')?;
        if u64::from_str_radix(namespace, 16).ok()? != self.namespace { return None; }
        let id = ExpansionId(id.parse().ok()?);
        self.outputs.contains_key(&id).then_some(id)
    }

    /// Map only proved contiguous ranges; mixed-origin extents stay in the
    /// retained output snapshot instead of guessing a caller/definition range.
    pub fn original_span(&self, span: &Span) -> Span {
        let mut span = span.clone();
        for _ in 0..self.records.len() {
            let Some(id) = self.expansion_at(&span) else { break; };
            let Some(output) = self.outputs.get(&id) else { break; };
            let end = output.segments.partition_point(|segment| segment.range.start <= span.start);
            let Some(segment) = end.checked_sub(1).and_then(|index| output.segments.get(index)).filter(|segment|
                span.end <= segment.range.end && segment.source.end - segment.source.start == segment.range.len()) else { break; };
            // Proc-constructed fragments have no original source snapshot.
            if segment.source.file_path == std::path::Path::new("<macro-construction>") { break; }
            span = Span::new(segment.source.file_path.clone(), segment.source.start + span.start - segment.range.start,
                segment.source.start + span.end - segment.range.start);
        }
        span
    }

    fn diagnostic_span(&self, span: &Span) -> Span {
        let mut span = span.clone();
        for _ in 0..self.records.len() {
            let Some(id) = self.expansion_at(&span) else { break; };
            let Some(output) = self.outputs.get(&id) else { break; };
            let end = output.segments.partition_point(|segment| segment.range.start <= span.start);
            let captured = end.checked_sub(1).and_then(|index| output.segments.get(index)).filter(|segment|
                !segment.generated && span.end <= segment.range.end
                    && segment.source.end - segment.source.start == segment.range.len());
            span = match captured {
                Some(segment) => Span::new(segment.source.file_path.clone(), segment.source.start + span.start - segment.range.start,
                    segment.source.start + span.end - segment.range.start),
                None => match self.record(id) { Some(record) => record.invocation_span.clone(), None => break },
            };
        }
        span
    }

    pub(crate) fn attach_sources(&self, sources: &mut crate::diagnostic::DiagnosticSourceMap) {
        for (id, output) in &self.outputs { sources.insert_virtual(TokenOrigin::output_path(self.namespace, *id), output.text.clone()); }
    }

    /// Preserve the phase's code/severity while adding invocation ancestry and
    /// mapping every label, including borrow-check secondary ownership sites.
    pub fn map_diagnostics(&self, mut diagnostics: crate::diagnostic::Diagnostics) -> crate::diagnostic::Diagnostics {
        use crate::diagnostic::{DiagnosticLabel, DiagnosticLocation};
        for diagnostic in &mut diagnostics.0 {
            let mut expansions = Vec::new();
            if let DiagnosticLocation::Source(span) = &diagnostic.location {
                if let Some(id) = self.expansion_at(span) { expansions.push(id); }
            }
            for label in diagnostic.primary.iter().chain(&diagnostic.secondary) {
                if let Some(id) = self.expansion_at(&label.span) {
                    if !expansions.contains(&id) { expansions.push(id); }
                }
            }
            for id in expansions {
                for (message, span) in self.trace_labels(id) {
                    let label = DiagnosticLabel::new(message, span);
                    if !diagnostic.secondary.contains(&label) { diagnostic.secondary.push(label); }
                }
            }
            if let DiagnosticLocation::Source(span) = &mut diagnostic.location {
                let original = self.diagnostic_span(span);
                if self.expansion_at(span).is_some() && original != *span {
                    diagnostic.secondary.push(DiagnosticLabel::new("expanded syntax here", span.clone()));
                }
                if original.file_path != span.file_path { diagnostic.source = None; }
                *span = original;
            }
            if let Some(label) = &mut diagnostic.primary { label.span = self.diagnostic_span(&label.span); }
            for label in &mut diagnostic.secondary { label.span = self.original_span(&label.span); }
            let mut seen = Vec::new();
            diagnostic.secondary.retain(|label| {
                if seen.contains(label) { false } else { seen.push(label.clone()); true }
            });
        }
        diagnostics
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_expansion(&mut self, record: MacroExpansionRecord) -> ExpansionId {
        let id = ExpansionId(self.records.len() as u32 + 1);
        let generated_source = GeneratedSourceId(self.generated_sources.len() as u32 + 1);
        self.records.push(record);
        self.generated_sources.push(generated_source);
        id
    }

    pub fn record(&self, id: ExpansionId) -> Option<&MacroExpansionRecord> {
        if id.0 == 0 {
            return None;
        }

        self.records.get(id.0 as usize - 1)
    }

    pub fn generated_source_id(&self, id: ExpansionId) -> Option<GeneratedSourceId> {
        if id.0 == 0 {
            return None;
        }

        self.generated_sources.get(id.0 as usize - 1).copied()
    }

    pub fn trace(&self, id: ExpansionId) -> Vec<MacroExpansionRecord> {
        let mut trace = Vec::new();
        let mut visited = Vec::new();
        let mut current = Some(id);

        while let Some(id) = current {
            if visited.contains(&id) {
                break;
            }
            visited.push(id);

            let Some(record) = self.record(id) else {
                break;
            };

            trace.push(record.clone());
            current = record.parent;
        }

        trace
    }

    pub fn trace_labels(&self, id: ExpansionId) -> Vec<(String, Span)> {
        self.trace(id)
            .into_iter()
            .flat_map(|record| {
                let mut labels = vec![(
                    format!("expanded from macro '{}'", record.macro_name),
                    record.invocation_span,
                )];

                if let Some(definition_span) = record.definition_span {
                    labels.push(("macro definition here".to_string(), definition_span));
                }

                labels
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
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
            invocation: crate::ast::InvocationId::fresh().unwrap(),
            macro_name: "outer".to_string(),
            invocation_span: span(1, 7),
            definition_span: Some(span(10, 20)),
            parent: None,
        });
        let child = map.record_expansion(MacroExpansionRecord {
            invocation: crate::ast::InvocationId::fresh().unwrap(),
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
            invocation: crate::ast::InvocationId::fresh().unwrap(),
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
            invocation: crate::ast::InvocationId::fresh().unwrap(),
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
        let source = TokenOrigin::from(TokenProvenance::Source(span(0, 1)));
        let captured = TokenOrigin::from(TokenProvenance::Captured {
            expansion,
            invocation_span: span(1, 5),
            capture_span: span(6, 9),
        });
        let generated = TokenOrigin::from(TokenProvenance::Generated {
            expansion,
            generated_source: GeneratedSourceId(1),
            definition_span: span(10, 12),
        });

        assert_eq!(source.expansion_id(), None);
        assert_eq!(captured.expansion_id(), Some(expansion));
        assert_eq!(generated.expansion_id(), Some(expansion));
    }

    #[test]
    fn trace_stops_when_parent_links_cycle() {
        let mut map = MacroSourceMap::new();
        let first = map.record_expansion(MacroExpansionRecord {
            invocation: crate::ast::InvocationId::fresh().unwrap(),
            macro_name: "first".to_string(),
            invocation_span: span(1, 7),
            definition_span: Some(span(10, 20)),
            parent: None,
        });
        let second = map.record_expansion(MacroExpansionRecord {
            invocation: crate::ast::InvocationId::fresh().unwrap(),
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
            invocation: crate::ast::InvocationId::fresh().unwrap(),
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
        let origin = TokenOrigin::from(TokenProvenance::Generated {
            expansion: ExpansionId(2),
            generated_source,
            definition_span: span(10, 12),
        });

        assert_eq!(origin.generated_source_id(), Some(generated_source));
    }
}

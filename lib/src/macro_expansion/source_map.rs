use crate::lexer::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ExpansionId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct GeneratedSourceId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TokenOrigin {
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

impl TokenOrigin {
    pub fn expansion_id(&self) -> Option<ExpansionId> {
        match self {
            TokenOrigin::Source(_) => None,
            TokenOrigin::Captured { expansion, .. } | TokenOrigin::Generated { expansion, .. } => {
                Some(*expansion)
            }
        }
    }

    pub fn generated_source_id(&self) -> Option<GeneratedSourceId> {
        match self {
            TokenOrigin::Generated {
                generated_source, ..
            } => Some(*generated_source),
            TokenOrigin::Source(_) | TokenOrigin::Captured { .. } => None,
        }
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
    records: Vec<MacroExpansionRecord>,
    generated_sources: Vec<GeneratedSourceId>,
}

impl MacroSourceMap {
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
        let origin = TokenOrigin::Generated {
            expansion: ExpansionId(2),
            generated_source,
            definition_span: span(10, 12),
        };

        assert_eq!(origin.generated_source_id(), Some(generated_source));
    }
}

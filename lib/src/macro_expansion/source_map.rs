use crate::lexer::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExpansionId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GeneratedSourceId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq)]
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
mod tests;

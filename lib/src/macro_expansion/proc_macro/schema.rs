//! Version-locked one-shot worker data. Ranges are logical UTF-8 byte offsets,
//! never paths, compiler definition IDs, or permission to load source files.

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::diagnostic::{Diagnostic, DiagnosticCode, DiagnosticSeverity, Diagnostics};
use crate::lexer::lossless::{ConstructionContext, ConstructionKind};
use crate::lexer::Span;

use super::{decode_stream, ProcMacroRequest, PROC_MACRO_PROTOCOL_VERSION};

pub const MAX_METADATA_ENTRIES: usize = 16_384;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcMacroViews {
    pub arguments: Option<ProcMacroArguments>,
    #[serde(deserialize_with = "crate::serialization::bounded_vec::<'de, _, _, MAX_METADATA_ENTRIES>")]
    pub groups: Vec<ProcMacroGroup>,
    #[serde(deserialize_with = "crate::serialization::bounded_vec::<'de, _, _, MAX_METADATA_ENTRIES>")]
    pub literals: Vec<ProcMacroLiteral>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcMacroArguments {
    #[serde(deserialize_with = "crate::serialization::bounded_vec::<'de, _, _, MAX_METADATA_ENTRIES>")]
    pub ranges: Vec<Range<usize>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcMacroGroup {
    pub delimiter: crate::lexer::groups::Delimiter,
    pub range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcMacroLiteral {
    pub source: Range<usize>,
    pub decoded: String,
    #[serde(deserialize_with = "crate::serialization::bounded_vec::<'de, _, _, MAX_METADATA_ENTRIES>")]
    pub mapping: Vec<LiteralMapping>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteralMapping {
    pub decoded: Range<usize>,
    pub source: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcMacroPart {
    /// An encoded shared lossless stream; metadata is reauthorized by compiler.
    Stream(#[serde(deserialize_with = "crate::serialization::bounded_vec::<'de, _, _, 1048576>")] Vec<u8>),
    /// Deferred output-only descriptors, interpreted by the compiler in B.19.
    Source { text: String, context: ConstructionContext },
    Literal { view: u32, range: Range<usize> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcMacroOutput {
    #[serde(deserialize_with = "crate::serialization::bounded_vec::<'de, _, _, MAX_METADATA_ENTRIES>")]
    pub parts: Vec<ProcMacroPart>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcMacroLocation {
    Invocation,
    Input(Range<usize>),
    Output(Range<usize>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcMacroLabel {
    pub message: String,
    pub location: ProcMacroLocation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcMacroDiagnostic {
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub primary: ProcMacroLocation,
    #[serde(deserialize_with = "crate::serialization::bounded_vec::<'de, _, _, MAX_METADATA_ENTRIES>")]
    pub labels: Vec<ProcMacroLabel>,
    #[serde(deserialize_with = "crate::serialization::bounded_vec::<'de, _, _, MAX_METADATA_ENTRIES>")]
    pub notes: Vec<String>,
    #[serde(deserialize_with = "crate::serialization::bounded_vec::<'de, _, _, MAX_METADATA_ENTRIES>")]
    pub help: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcMacroResponse {
    pub protocol_version: u32,
    /// None is failure; Some(empty parts) is an intentional empty expansion.
    pub output: Option<ProcMacroOutput>,
    #[serde(deserialize_with = "crate::serialization::bounded_vec::<'de, _, _, MAX_METADATA_ENTRIES>")]
    pub diagnostics: Vec<ProcMacroDiagnostic>,
}

impl ProcMacroResponse {
    pub fn diagnostics_at(&self, request: &ProcMacroRequest, invocation: Span, output: Option<&super::TokenStream>) -> Diagnostics {
        let input = decode_stream(&request.input).ok();
        let span = |location: &ProcMacroLocation| {
            let (stream, range) = match location {
                ProcMacroLocation::Invocation => return invocation.clone(),
                ProcMacroLocation::Input(range) => (input.as_ref(), range),
                ProcMacroLocation::Output(range) => (output, range),
            };
            stream.and_then(|stream| stream.slice(range.clone(), super::super::token_tree::MACRO_LIMITS).ok())
                .map(|stream| { let start = stream.start_span(); let end = stream.end_span();
                    if start.file_path == end.file_path && start.start <= end.end { Span::new(start.file_path, start.start, end.end) }
                    else { invocation.clone() }
                }).unwrap_or_else(|| invocation.clone())
        };
        Diagnostics(self.diagnostics.iter().map(|report| {
            let mut diagnostic = Diagnostic::new(format!("Proc macro '{}': {}", request.macro_name, report.message), span(&report.primary))
                .with_code(DiagnosticCode::Macro).with_severity(report.severity)
                .with_labels(report.labels.iter().map(|label| (label.message.clone(), span(&label.location))).collect());
            diagnostic.notes = report.notes.clone(); diagnostic.help = report.help.clone(); diagnostic
        }).collect())
    }

    pub fn expanded(output: Vec<u8>) -> Self {
        Self { protocol_version: PROC_MACRO_PROTOCOL_VERSION,
            output: Some(ProcMacroOutput { parts: vec![ProcMacroPart::Stream(output)] }), diagnostics: Vec::new() }
    }

    pub fn failure(messages: Vec<String>) -> Self {
        Self { protocol_version: PROC_MACRO_PROTOCOL_VERSION, output: None,
            diagnostics: messages.into_iter().map(|message| ProcMacroDiagnostic {
                severity: DiagnosticSeverity::Error, message, primary: ProcMacroLocation::Invocation,
                labels: Vec::new(), notes: Vec::new(), help: Vec::new(),
            }).collect() }
    }

    pub fn validate_for(&self, request: &ProcMacroRequest) -> Result<(), String> {
        request.validate()?;
        self.validate()?;
        let input = decode_stream(&request.input).map_err(|error| error.to_string())?;
        let input_text: String = input.runs().iter().map(|run| run.text()).collect();
        let mut output = String::new();
        if let Some(result) = &self.output {
            for part in &result.parts {
                match part {
                    ProcMacroPart::Stream(bytes) => {
                        let stream = decode_stream(bytes).map_err(|error| error.to_string())?;
                        stream.resolve_proc_output(&input, request.invocation_id, super::super::ExpansionId(request.expansion_id.unwrap_or(0)),
                            super::super::GeneratedSourceId(1), |context| {
                                validate_context(context, request)?;
                                Ok(Default::default())
                            })?;
                        check_output_length(output.len(), stream.len_bytes())?;
                        for run in stream.runs() { output.push_str(run.text()); }
                    }
                    ProcMacroPart::Source { text, context } => {
                        validate_context(*context, request)?; check_output_length(output.len(), text.len())?; output.push_str(text);
                    }
                    ProcMacroPart::Literal { view, range } => {
                        let literal = request.views.literals.get(*view as usize).ok_or("invalid literal view handle")?;
                        let spelling = literal.decoded.get(range.clone()).ok_or("invalid literal fragment range")?;
                        check_output_length(output.len(), spelling.len())?; output.push_str(spelling);
                    }
                }
            }
        }
        for diagnostic in &self.diagnostics {
            for location in std::iter::once(&diagnostic.primary).chain(diagnostic.labels.iter().map(|label| &label.location)) {
                match location {
                    ProcMacroLocation::Invocation => {}
                    ProcMacroLocation::Input(range) => { input_text.get(range.clone()).ok_or("invalid input diagnostic range")?; }
                    ProcMacroLocation::Output(range) => {
                        if self.output.is_none() { return Err("output diagnostic without output".into()); }
                        output.get(range.clone()).ok_or("invalid output diagnostic range")?;
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn validate(&self) -> Result<(), String> {
        if self.protocol_version != PROC_MACRO_PROTOCOL_VERSION { return Err("unsupported proc macro response version".into()); }
        if self.output.is_none() && !self.diagnostics.iter().any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error) {
            return Err("failed macro result requires an error diagnostic".into());
        }
        if let Some(output) = &self.output {
            for part in &output.parts {
                match part {
                    ProcMacroPart::Stream(bytes) => { decode_stream(bytes).map_err(|error| error.to_string())?; }
                    ProcMacroPart::Literal { range, .. } if range.start > range.end => return Err("invalid literal fragment range".into()),
                    _ => {}
                }
            }
        }
        Ok(())
    }

}

fn validate_context(context: ConstructionContext, request: &ProcMacroRequest) -> Result<(), String> {
    if context.invocation != request.invocation_id { return Err("construction context belongs to another invocation".into()); }
    if let ConstructionKind::Dependency(index) = context.kind {
        if index as usize >= request.definition_dependencies.len() { return Err("undeclared definition-side dependency handle".into()); }
    }
    Ok(())
}

fn check_output_length(left: usize, right: usize) -> Result<(), String> {
    if left.checked_add(right).is_none_or(|length| length > super::MAX_PROC_MACRO_MESSAGE_BYTES as usize) {
        return Err("expanded output byte limit exceeded".into());
    }
    Ok(())
}

impl ProcMacroRequest {
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.protocol_version != PROC_MACRO_PROTOCOL_VERSION { return Err("unsupported proc macro protocol version".into()); }
        if self.compiler_version != env!("CARGO_PKG_VERSION") { return Err("unsupported proc macro compiler version".into()); }
        if self.macro_identity.is_empty() || self.macro_name.is_empty() { return Err("missing macro export identity".into()); }
        if !self.invocation_id.0.is_valid() || self.expansion_id == Some(0) || self.parent == Some(super::super::ExpansionId(0)) {
            return Err("invalid invocation/expansion handle".into());
        }
        if self.header_indent > super::MAX_PROC_MACRO_MESSAGE_BYTES as usize
            || self.block_base.is_some_and(|base| base <= self.header_indent || base > super::MAX_PROC_MACRO_MESSAGE_BYTES as usize) {
            return Err("invalid invocation layout".into());
        }
        let input = decode_stream(&self.input).map_err(|error| error.to_string())?;
        if input.runs().iter().any(|run| run.has_construction_request()) { return Err("unresolved construction in proc input".into()); }
        let dependencies: std::collections::HashSet<_> = self.definition_dependencies.iter().collect();
        if dependencies.len() != self.definition_dependencies.len() { return Err("duplicate dependency handles".into()); }
        let text: String = input.runs().iter().map(|run| run.text()).collect();
        if let Some(arguments) = &self.views.arguments {
            let mut end = 0;
            for range in &arguments.ranges {
                if range.start < end || range.is_empty() || text.get(range.clone()).is_none() { return Err("invalid argument view range".into()); }
                end = range.end;
            }
        }
        let mut ends = Vec::new();
        let mut last_start = 0;
        for group in &self.views.groups {
            let spelling = text.get(group.range.clone()).ok_or("invalid group view range")?;
            let (open, close) = match group.delimiter {
                crate::lexer::groups::Delimiter::Paren => ('(', ')'),
                crate::lexer::groups::Delimiter::Bracket => ('[', ']'),
                crate::lexer::groups::Delimiter::Brace => ('{', '}'),
            };
            if spelling.len() < 2 || !spelling.starts_with(open) || !spelling.ends_with(close) || group.range.start < last_start {
                return Err("invalid group view delimiters/order".into());
            }
            while ends.last().is_some_and(|end| *end <= group.range.start) { ends.pop(); }
            if ends.last().is_some_and(|end| *end < group.range.end) || ends.len() >= 128 { return Err("invalid group view nesting".into()); }
            ends.push(group.range.end); last_start = group.range.start;
        }
        for literal in &self.views.literals {
            text.get(literal.source.clone()).ok_or("invalid literal view source")?;
            let mut decoded_end = 0;
            let mut source_end = literal.source.start;
            for mapping in &literal.mapping {
                if mapping.decoded.start != decoded_end || mapping.decoded.is_empty()
                    || literal.decoded.get(mapping.decoded.clone()).is_none() || text.get(mapping.source.clone()).is_none()
                    || mapping.source.is_empty() || mapping.source.start < source_end || mapping.source.end > literal.source.end {
                    return Err("invalid literal view mapping coverage".into());
                }
                decoded_end = mapping.decoded.end; source_end = mapping.source.end;
            }
            if decoded_end != literal.decoded.len() { return Err("incomplete literal view mapping".into()); }
        }
        Ok(())
    }
}

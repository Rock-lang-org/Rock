use super::*;
use ariadne::{Color, ColorGenerator, Label, Report, ReportKind};

impl Diagnostic {
    pub fn report(&self) {
        if let Err(error) = self.validate() {
            eprintln!(
                "error [internal]: invalid diagnostic payload ({error:?}): {}",
                self.message
            );
            return;
        }
        let source_content = match &self.location {
            DiagnosticLocation::Source(span) => {
                if let Some(source) = &self.source {
                    Some((
                        source.display_path.to_string_lossy().to_string(),
                        source.text.clone(),
                        source.related.clone(),
                        span.clone(),
                    ))
                } else if span.file_path.exists() {
                    std::fs::read_to_string(&span.file_path).ok().map(|source| {
                        (
                            span.file_path.to_string_lossy().to_string(),
                            source,
                            BTreeMap::new(),
                            span.clone(),
                        )
                    })
                } else {
                    None
                }
            }
            _ => None,
        };

        if let Some((file_id, source, related_sources, primary_span)) = source_content {
            if !span_is_valid_for_text(&primary_span, &source) {
                eprintln!(
                    "error [internal]: invalid diagnostic source range: {}",
                    self.message
                );
                return;
            }
            if self
                .primary
                .iter()
                .chain(self.secondary.iter())
                .any(|label| {
                    label.span.file_path == primary_span.file_path
                        && !span_is_valid_for_text(&label.span, &source)
                })
            {
                eprintln!(
                    "error [internal]: invalid diagnostic label range: {}",
                    self.message
                );
                return;
            }
            // File exists - use ariadne for pretty printing
            let mut colors = ColorGenerator::new();

            let report_kind = match self.severity {
                DiagnosticSeverity::Error => ReportKind::Error,
                DiagnosticSeverity::Warning => ReportKind::Warning,
                DiagnosticSeverity::Info | DiagnosticSeverity::Hint => ReportKind::Advice,
            };
            let mut builder = Report::build(report_kind, file_id.clone(), primary_span.start)
                .with_message(self.message.clone());
            if let Some(code) = self.code {
                builder = builder.with_code(code);
            }
            if !self.notes.is_empty() {
                builder = builder.with_note(self.notes.join("\n"));
            }
            if !self.help.is_empty() {
                builder = builder.with_help(self.help.join("\n"));
            }

            for (i, label) in self.primary.iter().chain(self.secondary.iter()).enumerate() {
                let span = &label.span;
                let label_file_id = if span.file_path == primary_span.file_path {
                    file_id.clone()
                } else if let Some(related) = related_sources.get(&span.file_path) {
                    related.display_path.to_string_lossy().to_string()
                } else {
                    span.file_path.to_string_lossy().to_string()
                };
                let color = if i == 0 {
                    Color::Fixed(9)
                } else {
                    colors.next()
                };
                builder = builder.with_label(
                    Label::new((label_file_id, span.start..span.end))
                        .with_message(&label.message)
                        .with_color(color),
                );
            }

            let mut report_sources = BTreeMap::from([(file_id, source)]);
            for related in related_sources.values() {
                report_sources.insert(
                    related.display_path.to_string_lossy().to_string(),
                    related.text.clone(),
                );
            }
            for label in self.primary.iter().chain(self.secondary.iter()) {
                let span = &label.span;
                if span.file_path != primary_span.file_path
                    && !related_sources.contains_key(&span.file_path)
                {
                    if let Ok(text) = std::fs::read_to_string(&span.file_path) {
                        if !span_is_valid_for_text(span, &text) {
                            eprintln!(
                                "error [internal]: invalid diagnostic label range: {}",
                                self.message
                            );
                            return;
                        }
                        report_sources.insert(span.file_path.to_string_lossy().to_string(), text);
                    }
                }
            }
            if let Err(error) = builder.finish().print(ariadne::sources(report_sources)) {
                eprintln!("error [internal]: failed to render diagnostic: {error}");
                return;
            }

            println!();
        } else {
            // No source available - print a simple error message with location info
            let code = self
                .code
                .map(|code| format!(" [{}]", code))
                .unwrap_or_default();
            eprintln!("{}{}: {}", self.severity.as_str(), code, self.message);

            match &self.location {
                DiagnosticLocation::Source(span) => eprintln!(
                    "  --> {}:{}-{}",
                    span.file_path.display(),
                    span.start,
                    span.end
                ),
                DiagnosticLocation::File(path) => eprintln!("  --> file {}", path.display()),
                DiagnosticLocation::Project(path) => {
                    eprintln!("  --> project {}", path.display())
                }
                DiagnosticLocation::Artifact(path) => {
                    eprintln!("  --> artifact {}", path.display())
                }
                DiagnosticLocation::Toolchain => eprintln!("  --> toolchain"),
            }

            for label in self.primary.iter().chain(self.secondary.iter()) {
                let span = &label.span;
                if !span.file_path.as_os_str().is_empty() {
                    eprintln!(
                        "      {}: {} ({}-{})",
                        label.message,
                        span.file_path.display(),
                        span.start,
                        span.end
                    );
                }
            }

            for note in &self.notes {
                eprintln!("  note: {note}");
            }
            for help in &self.help {
                eprintln!("  help: {help}");
            }

            println!();
        }
    }
}

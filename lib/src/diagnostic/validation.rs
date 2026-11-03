use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticValidationError {
    InvalidLocation,
    MissingPrimaryLabel,
    PrimaryLabelDoesNotMatchLocation,
    InvalidLabel,
    SourceAttachedToNonSource,
}

impl Diagnostic {
    /// Validate the public payload before handing it to a protocol adapter.
    pub fn validate(&self) -> Result<(), DiagnosticValidationError> {
        let location_valid = match &self.location {
            DiagnosticLocation::Source(span) => span_is_valid(span),
            DiagnosticLocation::File(path)
            | DiagnosticLocation::Project(path)
            | DiagnosticLocation::Artifact(path) => !path.as_os_str().is_empty(),
            DiagnosticLocation::Toolchain => true,
        };
        if !location_valid {
            return Err(DiagnosticValidationError::InvalidLocation);
        }

        match &self.location {
            DiagnosticLocation::Source(span) => {
                let Some(primary) = &self.primary else {
                    return Err(DiagnosticValidationError::MissingPrimaryLabel);
                };
                if !spans_match(&primary.span, span) {
                    return Err(DiagnosticValidationError::PrimaryLabelDoesNotMatchLocation);
                }
            }
            DiagnosticLocation::File(_)
            | DiagnosticLocation::Project(_)
            | DiagnosticLocation::Artifact(_)
            | DiagnosticLocation::Toolchain => {
                if self.primary.is_some() || !self.secondary.is_empty() || self.source.is_some() {
                    return Err(DiagnosticValidationError::SourceAttachedToNonSource);
                }
            }
        }

        if self
            .primary
            .iter()
            .chain(self.secondary.iter())
            .any(|label| !span_is_valid(&label.span))
        {
            return Err(DiagnosticValidationError::InvalidLabel);
        }

        if let DiagnosticLocation::Source(primary_span) = &self.location {
            if let Some(source) = &self.source {
                if source.display_path.as_os_str().is_empty()
                    || source.related.iter().any(|(path, related)| {
                        path.as_os_str().is_empty() || related.display_path.as_os_str().is_empty()
                    })
                {
                    return Err(DiagnosticValidationError::InvalidLocation);
                }
                if !span_is_valid_for_text(primary_span, &source.text) {
                    return Err(DiagnosticValidationError::InvalidLocation);
                }
                for label in self.primary.iter().chain(self.secondary.iter()) {
                    let text = if label.span.file_path == primary_span.file_path {
                        Some(source.text.as_str())
                    } else {
                        source
                            .related
                            .get(&label.span.file_path)
                            .map(|related| related.text.as_str())
                    };
                    let Some(text) = text else {
                        return Err(DiagnosticValidationError::InvalidLabel);
                    };
                    if !span_is_valid_for_text(&label.span, text) {
                        return Err(DiagnosticValidationError::InvalidLabel);
                    }
                }
            }
        }
        Ok(())
    }
}

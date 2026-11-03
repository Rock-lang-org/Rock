use super::*;
use crate::parser::ParseError;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostics(pub Vec<Diagnostic>);

impl Diagnostics {
    pub fn from_resolve_errors(errors: Vec<crate::lower::ResolveError>) -> Self {
        Self(errors.iter().map(Diagnostic::from_resolve_error).collect())
    }

    pub fn push<T: Into<Diagnostic>>(&mut self, diagnostic: T) -> Self {
        let diagnostic = diagnostic.into();

        self.0.push(diagnostic);

        self.clone()
    }

    /// Push a SpannedError as a diagnostic
    pub fn push_error<E: SpannedError>(&mut self, err: &E) -> Self {
        self.0.push(Diagnostic::from_spanned(err));
        self.clone()
    }

    /// Push multiple errors
    pub fn extend_errors<E: SpannedError>(&mut self, errors: &[E]) -> Self {
        for err in errors {
            self.0.push(Diagnostic::from_spanned(err));
        }
        self.clone()
    }

    pub fn merge(&mut self, mut diagnostics: Diagnostics) {
        self.0.append(&mut diagnostics.0);
    }

    pub fn attach_sources(&mut self, sources: &DiagnosticSourceMap) {
        for diagnostic in &mut self.0 {
            sources.attach(diagnostic);
        }
    }

    pub fn with_sources(mut self, sources: &DiagnosticSourceMap) -> Self {
        self.attach_sources(sources);
        self
    }

    pub fn to_dto(&self) -> Result<Vec<DiagnosticDto>, DiagnosticValidationError> {
        self.0.iter().map(Diagnostic::to_dto).collect()
    }

    pub fn report(&self) {
        for diagnostic in &self.0 {
            diagnostic.report();
        }
    }

    pub fn return_if_err(&self) -> Result<(), Self> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(self.clone())
        }
    }

    pub fn contains(&self, text: &str) -> bool {
        self.0.iter().any(|diagnostic| {
            diagnostic.message.contains(text)
                || diagnostic.notes.iter().any(|note| note.contains(text))
        })
    }
}

impl std::fmt::Display for Diagnostics {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (index, diagnostic) in self.0.iter().enumerate() {
            if index > 0 {
                formatter.write_str("\n")?;
            }
            formatter.write_str(&diagnostic.message)?;
        }
        Ok(())
    }
}

impl From<ParseError> for Diagnostics {
    fn from(err: ParseError) -> Self {
        let mut diagnostics = Diagnostics::default();

        diagnostics.push(Diagnostic::from(err));

        diagnostics
    }
}

impl From<Vec<crate::lower::ResolveError>> for Diagnostics {
    fn from(errors: Vec<crate::lower::ResolveError>) -> Self {
        let mut diagnostics = Diagnostics::default();
        for err in errors {
            diagnostics.push(Diagnostic::from_resolve_error(&err));
        }
        diagnostics
    }
}

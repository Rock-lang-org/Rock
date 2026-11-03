use super::*;

impl Diagnostic {
    /// Convert into an owned payload with string codes for protocol adapters.
    pub fn to_dto(&self) -> Result<DiagnosticDto, DiagnosticValidationError> {
        self.validate()?;
        Ok(DiagnosticDto::from_validated(self))
    }
}

impl DiagnosticDto {
    fn from_validated(diagnostic: &Diagnostic) -> Self {
        Self {
            code: diagnostic.code.map(|code| code.as_str().to_string()),
            severity: diagnostic.severity,
            message: diagnostic.message.clone(),
            location: diagnostic.location.clone(),
            primary: diagnostic.primary.clone(),
            secondary: diagnostic.secondary.clone(),
            notes: diagnostic.notes.clone(),
            help: diagnostic.help.clone(),
            source: diagnostic.source.clone(),
        }
    }
}

impl TryFrom<&Diagnostic> for DiagnosticDto {
    type Error = DiagnosticValidationError;

    fn try_from(diagnostic: &Diagnostic) -> Result<Self, Self::Error> {
        diagnostic.to_dto()
    }
}

impl TryFrom<Diagnostic> for DiagnosticDto {
    type Error = DiagnosticValidationError;

    fn try_from(diagnostic: Diagnostic) -> Result<Self, Self::Error> {
        diagnostic.to_dto()
    }
}

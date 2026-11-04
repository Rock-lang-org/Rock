use super::*;

impl MonoError {
    pub(crate) fn diagnostic(
        self,
        resolver: &crate::collect::resolver::ResolverTables,
    ) -> crate::diagnostic::Diagnostic {
        let context = crate::type_services::display::TypeDisplayContext::from_resolver(resolver);
        let display = |ty: &Type| {
            crate::type_services::display::display_type_with_context(ty, &context).to_string()
        };
        let message = match self.kind {
            MonoErrorKind::MissingImpl { .. } => {
                "selected implementation is unavailable during specialization".to_string()
            }
            MonoErrorKind::MissingMethod { .. } => {
                "selected method is unavailable during specialization".to_string()
            }
            MonoErrorKind::ReceiverMismatch {
                receiver, pattern, ..
            } => match pattern {
                Some(HirImplReceiverPattern::Exact(expected))
                | Some(HirImplReceiverPattern::Constructor(expected)) => format!(
                    "method receiver type mismatch: expected {}, found {}",
                    display(&expected),
                    display(&receiver)
                ),
                Some(HirImplReceiverPattern::SliceFamily { element }) => format!(
                    "method receiver type mismatch: expected a slice of {}, found {}",
                    display(&element),
                    display(&receiver)
                ),
                None => format!("method receiver type mismatch for {}", display(&receiver)),
            },
            MonoErrorKind::InvalidBindings { .. } => {
                "method has invalid generic arguments during specialization".to_string()
            }
            MonoErrorKind::TraitArgsMismatch {
                expected, selected, ..
            } => format!(
                "trait argument mismatch: expected {}, found {}",
                display_type_list(&expected, &display),
                display_type_list(&selected, &display)
            ),
            MonoErrorKind::NoMatchingImpl { .. } => {
                "no matching trait implementation was found during specialization".to_string()
            }
            MonoErrorKind::AmbiguousImpls { .. } => {
                "multiple trait implementations match during specialization".to_string()
            }
            MonoErrorKind::MissingEffectiveMethod { .. } => {
                "trait implementation does not provide the selected method".to_string()
            }
            MonoErrorKind::MissingInstance { .. } => {
                "required callable instance is unavailable during specialization".to_string()
            }
        };
        crate::diagnostic::Diagnostic::new(message, self.span).with_code(DiagnosticCode::Mono)
    }
}

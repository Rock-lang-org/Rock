use crate::diagnostic::DiagnosticCode;
use crate::ids::DefId;
use crate::types::Type;

use super::hir_types::{HirImplReceiverPattern, HirSelectedMethodTarget};
use super::InstanceOrigin;

#[derive(Debug, Clone, PartialEq)]
pub enum MonoErrorKind {
    MissingImpl {
        impl_id: DefId,
    },
    MissingMethod {
        impl_id: DefId,
        method_id: DefId,
    },
    ReceiverMismatch {
        impl_id: DefId,
        receiver: Type,
        pattern: Option<HirImplReceiverPattern>,
    },
    InvalidBindings {
        target: HirSelectedMethodTarget,
    },
    TraitArgsMismatch {
        impl_id: DefId,
        expected: Vec<Type>,
        selected: Vec<Type>,
    },
    NoMatchingImpl {
        trait_id: DefId,
        member_id: DefId,
    },
    AmbiguousImpls {
        trait_id: DefId,
        member_id: DefId,
        impls: Vec<DefId>,
    },
    MissingEffectiveMethod {
        impl_id: DefId,
        member_id: DefId,
    },
    MissingInstance {
        origin: InstanceOrigin,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct MonoError {
    pub kind: MonoErrorKind,
    pub span: crate::lexer::Span,
}

impl MonoError {
    pub(super) fn diagnostic_with_context(
        self,
        context: &crate::type_services::display::TypeDisplayContext,
    ) -> crate::diagnostic::Diagnostic {
        let display = |ty: &Type| {
            crate::type_services::display::display_type_with_context(ty, context).to_string()
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

fn display_type_list(types: &[Type], display: &impl Fn(&Type) -> String) -> String {
    types.iter().map(display).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests;

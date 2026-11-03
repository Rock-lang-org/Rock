use crate::diagnostic::{Diagnostic, DiagnosticCode};
use crate::lexer::Span;
use crate::mir::Place;

pub fn borrow_error(message: impl Into<String>, span: Span) -> Diagnostic {
    Diagnostic::new(message.into(), span).with_code(DiagnosticCode::Borrow)
}

pub fn borrow_conflict(
    _place: &Place,
    use_span: Option<Span>,
    borrow_span: Option<Span>,
) -> Diagnostic {
    let Some(use_span) = use_span else {
        return Diagnostic::for_internal("borrow conflict is missing its use origin");
    };
    let Some(borrow_span) = borrow_span else {
        return Diagnostic::for_internal("borrow conflict is missing its loan origin");
    };
    Diagnostic::new(
        "borrow conflict on this value".to_string(),
        use_span.clone(),
    )
    .with_code(DiagnosticCode::Borrow)
    .with_label("conflicting access".to_string(), use_span)
    .with_label("borrow introduced here".to_string(), borrow_span)
}

#[cfg(test)]
mod tests;

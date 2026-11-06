//! `infix` declarations: each operator is declared once per crate graph.
//!
//! A crate may not redeclare an operator that it or one of its dependencies
//! already declared, even with the same precedence. A dependency's artifact
//! lists every operator it can see, including the ones it inherits from its
//! own dependencies, so two dependencies listing the same operator with the
//! same precedence are the same declaration seen twice.

use crate::ast;
use crate::diagnostic::{DiagnosticCode, DiagnosticLabel};
use crate::lexer::Span;
use crate::lower::ResolveError;

use super::CollectContext;

/// Where an operator's precedence was declared.
#[derive(Debug, Clone)]
pub(crate) enum InfixOrigin {
    Source(Span),
    Dependency(String),
}

impl CollectContext {
    /// Records an `infix` declaration from the crate being compiled.
    pub(crate) fn declare_infix_operator(&mut self, precedence: u8, operator: &ast::Operator) {
        let name = &operator.value;
        match self.infix_origins.get(name) {
            // The same declaration, visited again by another collection pass.
            Some(InfixOrigin::Source(span)) if *span == operator.span => return,
            Some(InfixOrigin::Source(first)) => {
                let first = first.clone();
                self.errors.push(ResolveError::with_span_and_labels(
                    format!("Operator '{name}' is declared more than once"),
                    operator.span.clone(),
                    vec![DiagnosticLabel::new(
                        format!("'{name}' is first declared here"),
                        first,
                    )],
                ));
                return;
            }
            Some(InfixOrigin::Dependency(crate_name)) => {
                let message = format!(
                    "Operator '{name}' is already declared by dependency '{crate_name}' \
                     with precedence {}",
                    self.infix_precedence.get(name).copied().unwrap_or_default()
                );
                self.push_error_with_span(message, operator.span.clone());
                return;
            }
            None => {}
        }

        self.infix_precedence.insert(name.clone(), precedence);
        self.infix_origins
            .insert(name.clone(), InfixOrigin::Source(operator.span.clone()));
    }

    /// Records an operator listed by a dependency's artifact.
    pub(crate) fn register_dependency_infix_operator(
        &mut self,
        crate_name: &str,
        name: &str,
        precedence: u8,
    ) {
        if let Some(existing) = self.infix_precedence.get(name).copied() {
            if existing != precedence {
                let first = match self.infix_origins.get(name) {
                    Some(InfixOrigin::Dependency(first)) => first.clone(),
                    Some(InfixOrigin::Source(_)) | None => "this crate".to_string(),
                };
                self.errors.push(ResolveError::non_source_code(
                    format!(
                        "Operator '{name}' is declared with precedence {existing} by \
                         '{first}' and with precedence {precedence} by '{crate_name}'"
                    ),
                    DiagnosticCode::Resolve,
                ));
            }
            return;
        }

        self.infix_precedence.insert(name.to_string(), precedence);
        self.infix_origins.insert(
            name.to_string(),
            InfixOrigin::Dependency(crate_name.to_string()),
        );
    }
}

//! `infix` declarations: each operator is declared once in a crate graph.
//!
//! Declaring an operator that the crate or any dependency already declared is
//! an error, whatever the precedence. Artifacts re-list the operators a crate
//! inherits, each tagged with the crate that declares it, so the same
//! declaration reached through two dependencies is recognized as one.

use std::collections::HashMap;

use crate::ast;
use crate::diagnostic::{DiagnosticCode, DiagnosticLabel};
use crate::lexer::Span;
use crate::lower::ResolveError;
use crate::products::ProductInfixOperator;

use super::CollectContext;

/// Where an operator was declared.
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
            Some(InfixOrigin::Dependency(declaring_crate)) => {
                let message =
                    format!("Operator '{name}' is already declared by crate '{declaring_crate}'");
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
        name: &str,
        operator: &ProductInfixOperator,
    ) {
        let declaring_crate = &operator.declaring_crate;
        if let Some(InfixOrigin::Dependency(first)) = self.infix_origins.get(name) {
            let same_declaration = first == declaring_crate
                && self.infix_precedence.get(name) == Some(&operator.precedence);
            if !same_declaration {
                self.errors.push(ResolveError::non_source_code(
                    format!(
                        "Operator '{name}' is declared more than once: by crate '{first}' \
                         and by crate '{declaring_crate}'"
                    ),
                    DiagnosticCode::Resolve,
                ));
            }
            return;
        }

        self.infix_precedence.insert(name.to_string(), operator.precedence);
        self.infix_origins.insert(
            name.to_string(),
            InfixOrigin::Dependency(declaring_crate.clone()),
        );
    }

    /// The declaring crate of each operator that came from a dependency.
    pub(crate) fn dependency_infix_owners(&self) -> HashMap<String, String> {
        self.infix_origins
            .iter()
            .filter_map(|(name, origin)| match origin {
                InfixOrigin::Dependency(declaring_crate) => {
                    Some((name.clone(), declaring_crate.clone()))
                }
                InfixOrigin::Source(_) => None,
            })
            .collect()
    }
}

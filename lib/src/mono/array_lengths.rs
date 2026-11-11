use crate::diagnostic::{Diagnostic, DiagnosticCode};
use crate::type_services::visit::type_any;
use crate::types::Type;

use super::Monomorphizer;

impl Monomorphizer {
    /// Rejects instances whose signature still carries a symbolic array length.
    ///
    /// This is the authority for array lengths: MIR and code generation assume
    /// every length they see is concrete.
    pub(super) fn check_instance_array_lengths(&mut self) {
        let mut offenders = Vec::new();
        for record in self.instances.records() {
            let Some(function) = self.instances.pre_mir_body(record.id) else {
                continue;
            };
            let unresolved = |ty: &Type| {
                type_any(ty, |nested| {
                    matches!(nested, Type::Array(_, len) if !len.is_concrete())
                })
            };
            if function.params.iter().any(|param| unresolved(&param.ty)) || unresolved(&function.ret_type) {
                offenders.push((function.id, function.name.clone()));
            }
        }
        for (id, name) in offenders {
            let message = format!("cannot determine the array length used by `{name}`");
            let diagnostic = match self.source_map.definition_span(id) {
                Some(span) => Diagnostic::new(message, span.clone()),
                None => Diagnostic::for_internal(message),
            }
            .with_code(DiagnosticCode::Mono)
            .with_help("add a type annotation that fixes the array length");
            self.diagnostics.push(diagnostic);
        }
    }
}

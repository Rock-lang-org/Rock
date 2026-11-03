use crate::diagnostic::Diagnostic;
use crate::lexer::Span;
use crate::mir::borrowck::diagnostics::borrow_error;
use crate::mir::dataflow::analyses::InitError;
use crate::mir::{MirFunction, Operand};

use super::*;

impl BorrowChecker {
    pub(super) fn make_error(
        error: InitError,
        func: &MirFunction,
        operand: &Operand,
        stmt_span: Option<Span>,
    ) -> Diagnostic {
        let span = stmt_span;
        let is_move_error = error.message.contains("moved");
        let move_span = error.move_span.clone();

        let msg = match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                if place.local.0 < func.local_decls.len() {
                    if let Some(name) = &func.local_decls[place.local.0].name {
                        format!("{} `{name}`", error.message)
                    } else {
                        error.message.clone()
                    }
                } else {
                    error.message.clone()
                }
            }
            _ => error.message.clone(),
        };

        if span.is_none() || (is_move_error && move_span.is_none()) {
            return mir_diagnostic(msg, None);
        }

        let label_msg = match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                if place.local.0 < func.local_decls.len() {
                    if let Some(name) = &func.local_decls[place.local.0].name {
                        if move_span.is_some() {
                            format!("`{}` used here after move", name)
                        } else {
                            format!("`{}` used here", name)
                        }
                    } else {
                        "used here".to_string()
                    }
                } else {
                    "used here".to_string()
                }
            }
            _ => "here".to_string(),
        };

        let mut diagnostic = match span {
            Some(span) => borrow_error(msg, span.clone()).with_label(label_msg, span),
            None => mir_diagnostic(msg, None),
        };

        if let Some(move_span) = move_span {
            let move_label_msg = match operand {
                Operand::Copy(place) | Operand::Move(place) => {
                    if place.local.0 < func.local_decls.len() {
                        if let Some(name) = &func.local_decls[place.local.0].name {
                            format!("value moved from `{}` here", name)
                        } else {
                            "value moved here".to_string()
                        }
                    } else {
                        "value moved here".to_string()
                    }
                }
                _ => "value moved here".to_string(),
            };
            diagnostic = diagnostic.with_label(move_label_msg, move_span);
        }

        diagnostic
    }
}

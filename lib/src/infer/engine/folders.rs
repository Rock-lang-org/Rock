use std::collections::{HashMap, HashSet};

use crate::ids::TypeVarId;
use crate::lexer::Span;
use crate::type_services::kind::Kind;
use crate::type_services::visit::{fold_type_children, TypeFolder};
use crate::types::Type;

use super::errors::{InferenceError, UnifyError};

pub(super) struct ResolveFolder<'a> {
    pub(super) substitutions: &'a HashMap<TypeVarId, Type>,
}

impl TypeFolder for ResolveFolder<'_> {
    fn fold_type(&mut self, ty: Type) -> Type {
        match ty {
            Type::TypeVar(id) => self
                .substitutions
                .get(&id)
                .cloned()
                .map(|replacement| self.fold_type(replacement))
                .unwrap_or(Type::TypeVar(id)),
            other => refresh_callable(fold_type_children(other, self)),
        }
    }
}

pub(super) struct StrictFinalizer<'a> {
    pub(super) substitutions: &'a HashMap<TypeVarId, Type>,
    pub(super) var_spans: &'a HashMap<TypeVarId, Span>,
    pub(super) var_kinds: &'a HashMap<TypeVarId, Kind>,
    pub(super) errors: &'a mut Vec<InferenceError>,
    pub(super) reported: HashSet<TypeVarId>,
}

impl TypeFolder for StrictFinalizer<'_> {
    fn fold_type(&mut self, ty: Type) -> Type {
        match ty {
            Type::TypeVar(id) => {
                if let Some(replacement) = self.substitutions.get(&id).cloned() {
                    return self.fold_type(replacement);
                }
                if self.reported.insert(id) {
                    let kind = self.var_kinds.get(&id).cloned().unwrap_or(Kind::Type);
                    self.errors.push(InferenceError {
                        error: UnifyError::AmbiguousType { kind },
                        span: self.var_spans.get(&id).cloned(),
                    });
                }
                Type::Error
            }
            other => refresh_callable(fold_type_children(other, self)),
        }
    }
}

/// Body effects remain typed until resolution/defaulting finishes. Owning an
/// environment never itself makes its invocation consuming.
fn refresh_callable(ty: Type) -> Type {
    match ty {
        Type::Function { params, ret, safety, callable_kind, mut captures } => {
            for capture in &mut captures {
                if capture.kind == crate::types::CaptureKind::SharedBorrow
                    && crate::type_services::facts::TypeFacts::is_copy(&capture.ty)
                {
                    capture.kind = crate::types::CaptureKind::Move;
                }
            }
            let callable_kind = if captures.is_empty() { callable_kind } else {
                crate::types::CallableKind::from_captures(&captures)
            };
            Type::Function { params, ret, safety, callable_kind, captures }
        }
        other => other,
    }
}

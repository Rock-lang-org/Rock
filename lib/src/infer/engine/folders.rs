use std::collections::{HashMap, HashSet};

use crate::ids::TypeVarId;
use crate::lexer::Span;
use crate::type_services::kind::Kind;
use crate::type_services::visit::{fold_type_children, TypeFolder};
use crate::types::{ArrayLen, Type};

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
            other => {
                let resolved = fold_type_children(other, self);
                match resolved {
                    Type::Function {
                        params,
                        ret,
                        safety,
                        captures,
                        ..
                    } => Type::Function {
                        params,
                        ret,
                        safety,
                        callable_kind: crate::types::CallableKind::from_captures(&captures),
                        captures,
                    },
                    other => other,
                }
            }
        }
    }

    fn fold_array_len(&mut self, len: ArrayLen) -> ArrayLen {
        match len {
            ArrayLen::Var(id) => match self.substitutions.get(&id).cloned() {
                Some(replacement) => {
                    let resolved = self.fold_type(replacement);
                    ArrayLen::from_const_type(&resolved)
                }
                None => len,
            },
            other => other,
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
            other => fold_type_children(other, self),
        }
    }

    fn fold_array_len(&mut self, len: ArrayLen) -> ArrayLen {
        let ArrayLen::Var(id) = len else {
            return len;
        };
        match self.fold_type(Type::TypeVar(id)) {
            Type::Error => ArrayLen::Error,
            resolved => ArrayLen::from_const_type(&resolved),
        }
    }
}

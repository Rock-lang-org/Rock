use std::collections::HashSet;

use crate::ids::TypeVarId;
use crate::type_services::normalize::TypeNormalizer;
use crate::type_services::visit::fold_type;
use crate::types::Type;

use super::errors::{InferenceError, UnifyError};
use super::folders::StrictFinalizer;
use super::InferenceEngine;

impl InferenceEngine {
    /// Check if a type variable occurs in a type (for infinite type prevention)
    pub fn occurs_in(&self, var: TypeVarId, ty: &Type) -> bool {
        let ty = self.resolve(ty);
        match &ty {
            Type::TypeVar(id) => *id == var,
            Type::Slice(inner) | Type::Pointer(inner) => self.occurs_in(var, inner),
            Type::Array(inner, _) => self.occurs_in(var, inner),
            Type::Reference { inner, .. } => self.occurs_in(var, inner),
            Type::Tuple(elems) => elems.iter().any(|t| self.occurs_in(var, t)),
            Type::Function {
                params,
                ret,
                captures,
                ..
            } => {
                params.iter().any(|t| self.occurs_in(var, t))
                    || self.occurs_in(var, ret)
                    || captures
                        .iter()
                        .any(|capture| self.occurs_in(var, &capture.ty))
            }
            Type::Struct { args, .. } | Type::Enum { args, .. } => {
                args.iter().any(|t| self.occurs_in(var, t))
            }
            Type::Projection { ty, trait_args, .. } => {
                self.occurs_in(var, ty) || trait_args.iter().any(|arg| self.occurs_in(var, arg))
            }
            Type::Apply { constructor, args } => {
                self.occurs_in(var, constructor) || args.iter().any(|arg| self.occurs_in(var, arg))
            }
            Type::Lambda { body, .. } => self.occurs_in(var, body),
            _ => false,
        }
    }

    /// Finalize a type in strict mode: unresolved TypeVars produce an error entry.
    /// Returns `Type::Error` for ambiguous TypeVars so compilation can continue.
    pub fn finalize_strict(&self, ty: &Type, errors: &mut Vec<InferenceError>) -> Type {
        let finalized = fold_type(
            ty.clone(),
            &mut StrictFinalizer {
                substitutions: &self.substitutions,
                var_spans: &self.var_spans,
                var_kinds: &self.var_kinds,
                errors,
                reported: HashSet::new(),
            },
        );
        match TypeNormalizer::new(&self.normalization_env).normalize(&finalized) {
            Ok(normalized) => normalized,
            Err(error) => {
                let mut span = None;
                crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
                    if span.is_none() {
                        if let Type::TypeVar(id) = nested {
                            span = self.var_spans.get(id).cloned();
                        }
                    }
                });
                errors.push(InferenceError {
                    error: UnifyError::Normalization {
                        ty: finalized.clone(),
                        detail: error,
                    },
                    span,
                });
                Type::Error
            }
        }
    }
}

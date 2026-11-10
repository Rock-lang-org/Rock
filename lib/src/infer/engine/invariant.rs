use crate::type_services::normalize::TypeNormalizer;
use crate::types::Type;

use super::errors::UnifyError;
use super::InferenceEngine;

impl InferenceEngine {
    pub(crate) fn unify_invariant(&mut self, a: &Type, b: &Type) -> Result<(), UnifyError> {
        let mut probe = self.clone_for_probe();
        probe.unify(a, b)?;
        probe.unify(b, a)?;
        let resolved_a = TypeNormalizer::new(&probe.normalization_env)
            .normalize(&probe.resolve(a))
            .map_err(|error| UnifyError::Normalization {
                ty: probe.resolve(a),
                detail: error,
            })?;
        let resolved_b = TypeNormalizer::new(&probe.normalization_env)
            .normalize(&probe.resolve(b))
            .map_err(|error| UnifyError::Normalization {
                ty: probe.resolve(b),
                detail: error,
            })?;
        if !Self::same_unification_shape(&resolved_a, &resolved_b) {
            return Err(UnifyError::Mismatch {
                expected: resolved_a,
                found: resolved_b,
            });
        }
        *self = probe;
        Ok(())
    }

    pub(super) fn preserve_function_metadata(
        &mut self,
        original_a: &Type,
        original_b: &Type,
        a: &Type,
        b: &Type,
    ) {
        let Type::Function {
            callable_kind: a_kind,
            captures: a_captures,
            ..
        } = a
        else {
            return;
        };
        let Type::Function {
            callable_kind: b_kind,
            captures: b_captures,
            ..
        } = b
        else {
            return;
        };

        let a_is_default = a_kind.is_default(a_captures);
        let b_is_default = b_kind.is_default(b_captures);
        if !a_is_default && !b_is_default {
            return;
        }

        let metadata = if a_is_default {
            (*b_kind, b_captures.clone())
        } else {
            (*a_kind, a_captures.clone())
        };
        let replacement = |ty: &Type| match ty {
            Type::Function {
                params,
                ret,
                safety,
                ..
            } => Type::Function {
                params: params.clone(),
                ret: ret.clone(),
                safety: *safety,
                callable_kind: metadata.0,
                captures: metadata.1.clone(),
            },
            _ => ty.clone(),
        };

        for ty in [original_a, original_b] {
            let Type::TypeVar(id) = ty else {
                continue;
            };
            if self.substitutions.contains_key(id) {
                let current = self
                    .substitutions
                    .get(id)
                    .cloned()
                    .map(|ty| self.resolve(&ty))
                    .unwrap_or_else(|| ty.clone());
                let replacement = replacement(&current);
                if current != replacement {
                    self.substitutions.insert(*id, replacement);
                    self.substitution_generation = self.substitution_generation.wrapping_add(1);
                    self.rigid_substitution_generation =
                        self.rigid_substitution_generation.wrapping_add(1);
                    self.changed_type_vars.push(*id);
                }
            }
        }
    }

    fn same_unification_shape(a: &Type, b: &Type) -> bool {
        match (a, b) {
            (
                Type::Function {
                    params: a_params,
                    ret: a_ret,
                    safety: a_safety,
                    ..
                },
                Type::Function {
                    params: b_params,
                    ret: b_ret,
                    safety: b_safety,
                    ..
                },
            ) => {
                a_safety == b_safety
                    && a_params.len() == b_params.len()
                    && a_params
                        .iter()
                        .zip(b_params)
                        .all(|(a, b)| Self::same_unification_shape(a, b))
                    && Self::same_unification_shape(a_ret, b_ret)
            }
            (Type::Slice(a), Type::Slice(b)) | (Type::Pointer(a), Type::Pointer(b)) => {
                Self::same_unification_shape(a, b)
            }
            (Type::Array(a, a_len), Type::Array(b, b_len)) => {
                (a_len == b_len || matches!(a_len, crate::types::ArrayLen::Var(_)) || matches!(b_len, crate::types::ArrayLen::Var(_)))
                    && Self::same_unification_shape(a, b)
            }
            (Type::Tuple(a), Type::Tuple(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b)
                        .all(|(a, b)| Self::same_unification_shape(a, b))
            }
            (
                Type::Reference {
                    mutable: a_mutable,
                    inner: a_inner,
                },
                Type::Reference {
                    mutable: b_mutable,
                    inner: b_inner,
                },
            ) => a_mutable == b_mutable && Self::same_unification_shape(a_inner, b_inner),
            (
                Type::Struct {
                    id: a_id,
                    args: a_args,
                },
                Type::Struct {
                    id: b_id,
                    args: b_args,
                },
            )
            | (
                Type::Enum {
                    id: a_id,
                    args: a_args,
                },
                Type::Enum {
                    id: b_id,
                    args: b_args,
                },
            ) => {
                a_id == b_id
                    && a_args.len() == b_args.len()
                    && a_args
                        .iter()
                        .zip(b_args)
                        .all(|(a, b)| Self::same_unification_shape(a, b))
            }
            (
                Type::Projection {
                    ty: a_ty,
                    trait_id: a_trait,
                    assoc_type: a_assoc,
                    trait_args: a_args,
                },
                Type::Projection {
                    ty: b_ty,
                    trait_id: b_trait,
                    assoc_type: b_assoc,
                    trait_args: b_args,
                },
            ) => {
                a_trait == b_trait
                    && a_assoc == b_assoc
                    && a_args.len() == b_args.len()
                    && Self::same_unification_shape(a_ty, b_ty)
                    && a_args
                        .iter()
                        .zip(b_args)
                        .all(|(a, b)| Self::same_unification_shape(a, b))
            }
            _ => a == b,
        }
    }
}

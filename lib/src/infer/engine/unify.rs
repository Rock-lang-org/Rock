use crate::type_services::facts::TypeFacts;
use crate::type_services::normalize::TypeNormalizer;
use crate::type_services::visit::fold_type;
use crate::types::Type;

use super::errors::UnifyError;
use super::folders::ResolveFolder;
use super::InferenceEngine;

impl InferenceEngine {
    /// Find the representative type for a type variable (path compression)
    pub fn resolve(&self, ty: &Type) -> Type {
        fold_type(
            ty.clone(),
            &mut ResolveFolder {
                substitutions: &self.substitutions,
            },
        )
    }

    /// Unify two types, producing substitutions
    pub fn unify(&mut self, a: &Type, b: &Type) -> Result<(), UnifyError> {
        let original_a = a.clone();
        let original_b = b.clone();
        let resolved_a = self.resolve(a);
        let a = TypeNormalizer::new(&self.normalization_env)
            .normalize(&resolved_a)
            .map_err(|error| UnifyError::Normalization {
                ty: resolved_a.clone(),
                detail: error,
            })?;
        let resolved_b = self.resolve(b);
        let b = TypeNormalizer::new(&self.normalization_env)
            .normalize(&resolved_b)
            .map_err(|error| UnifyError::Normalization {
                ty: resolved_b.clone(),
                detail: error,
            })?;

        if a == b {
            if let Type::TypeVar(id) = original_a {
                if self.substitutions.contains_key(&id) && self.substitutions.get(&id) != Some(&a) {
                    self.substitutions.insert(id, a.clone());
                    self.substitution_generation = self.substitution_generation.wrapping_add(1);
                    self.changed_type_vars.push(id);
                }
            }
            return Ok(());
        }

        let a_kind = TypeNormalizer::new(&self.normalization_env)
            .kind_of(&a)
            .map_err(|error| UnifyError::Normalization {
                ty: a.clone(),
                detail: error,
            })?;
        let b_kind = TypeNormalizer::new(&self.normalization_env)
            .kind_of(&b)
            .map_err(|error| UnifyError::Normalization {
                ty: b.clone(),
                detail: error,
            })?;
        if a_kind != b_kind {
            return Err(UnifyError::KindMismatch {
                expected: a_kind,
                found: b_kind,
                ty: None,
                context: "unifying",
            });
        }

        if let Some(result) = self.try_unify_constructor_spine(&a, &b) {
            if result.is_ok() {
                self.canonicalize_type_var_alias(&original_a, &b);
            }
            return result;
        }
        if let Some(result) = self.try_unify_constructor_spine(&b, &a) {
            if result.is_ok() {
                self.canonicalize_type_var_alias(&original_b, &a);
            }
            return result;
        }

        match (&a, &b) {
            (Type::TypeVar(id), _) => self.bind_type_var(*id, b),
            (_, Type::TypeVar(id)) => self.bind_type_var(*id, a),
            (Type::Never, _) | (_, Type::Never) => Ok(()),
            (Type::Error, _) | (_, Type::Error) => Ok(()),
            (Type::Tuple(elements), Type::Unit) | (Type::Unit, Type::Tuple(elements))
                if elements.is_empty() =>
            {
                Ok(())
            }
            (Type::Slice(a_inner), Type::Slice(b_inner)) => self.unify(a_inner, b_inner),
            (Type::Array(a_inner, a_len), Type::Array(b_inner, b_len)) => {
                // Lengths unify as const-kinded types: `Var` binds to a
                // `ConstValue`, `Param` stays rigid.
                if self
                    .unify(&a_len.to_const_type(), &b_len.to_const_type())
                    .is_err()
                {
                    return Err(UnifyError::Mismatch {
                        expected: a.clone(),
                        found: b.clone(),
                    });
                }
                self.unify(a_inner, b_inner)
            }
            (Type::Tuple(a_elems), Type::Tuple(b_elems)) => {
                if a_elems.len() != b_elems.len() {
                    return Err(UnifyError::ArityMismatch {
                        kind: "tuple",
                        expected: a_elems.len(),
                        found: b_elems.len(),
                        left: a.clone(),
                        right: b.clone(),
                    });
                }
                for (ae, be) in a_elems.iter().zip(b_elems.iter()) {
                    self.unify(ae, be)?;
                }
                Ok(())
            }
            (
                Type::Function {
                    params: a_args,
                    ret: a_ret,
                    safety: a_safety,
                    ..
                },
                Type::Function {
                    params: b_args,
                    ret: b_ret,
                    safety: b_safety,
                    ..
                },
            ) => {
                if matches!(a_safety, crate::types::FunctionSafety::Unsafe)
                    && matches!(b_safety, crate::types::FunctionSafety::Safe)
                {
                    return Err(UnifyError::FunctionSafetyMismatch {
                        expected: a.clone(),
                        found: b.clone(),
                    });
                }
                if a_args.len() != b_args.len() {
                    return Err(UnifyError::ArityMismatch {
                        kind: "function",
                        expected: a_args.len(),
                        found: b_args.len(),
                        left: a.clone(),
                        right: b.clone(),
                    });
                }
                for (aa, ba) in a_args.iter().zip(b_args.iter()) {
                    self.unify(ba, aa)?;
                }
                self.unify(a_ret, b_ret)?;
                self.preserve_function_metadata(&original_a, &original_b, &a, &b);
                Ok(())
            }
            (
                Type::Struct {
                    id: a_id,
                    args: a_gen,
                },
                Type::Struct {
                    id: b_id,
                    args: b_gen,
                },
            )
            | (
                Type::Enum {
                    id: a_id,
                    args: a_gen,
                },
                Type::Enum {
                    id: b_id,
                    args: b_gen,
                },
            ) => {
                if a_id != b_id {
                    return Err(UnifyError::Mismatch {
                        expected: a.clone(),
                        found: b.clone(),
                    });
                }
                if a_gen.len() != b_gen.len() {
                    return Err(UnifyError::ArityMismatch {
                        kind: "generic",
                        expected: a_gen.len(),
                        found: b_gen.len(),
                        left: a.clone(),
                        right: b.clone(),
                    });
                }
                for (ag, bg) in a_gen.iter().zip(b_gen.iter()) {
                    self.unify(ag, bg)?;
                }
                Ok(())
            }
            (
                Type::Reference {
                    mutable: am,
                    inner: ai,
                },
                Type::Reference {
                    mutable: bm,
                    inner: bi,
                },
            ) => {
                if am != bm {
                    return Err(UnifyError::MutabilityMismatch);
                }
                if *am {
                    self.unify_invariant(ai, bi)
                } else {
                    self.unify(ai, bi)
                }
            }
            (Type::Pointer(a_inner), Type::Pointer(b_inner)) => {
                self.unify_invariant(a_inner, b_inner)
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
                if a_trait != b_trait || a_assoc != b_assoc || a_args.len() != b_args.len() {
                    return Err(UnifyError::Mismatch {
                        expected: a.clone(),
                        found: b.clone(),
                    });
                }
                self.unify(a_ty, b_ty)?;
                for (a_arg, b_arg) in a_args.iter().zip(b_args.iter()) {
                    self.unify(a_arg, b_arg)?;
                }
                Ok(())
            }
            (
                Type::Constructor {
                    id: a_id,
                    flavor: a_flavor,
                },
                Type::Constructor {
                    id: b_id,
                    flavor: b_flavor,
                },
            ) if a_id == b_id && a_flavor == b_flavor => Ok(()),
            (
                Type::Apply {
                    constructor: a_constructor,
                    args: a_args,
                },
                Type::Apply {
                    constructor: b_constructor,
                    args: b_args,
                },
            ) => {
                if let (Type::TypeVar(a_id), Type::TypeVar(b_id)) =
                    (a_constructor.as_ref(), b_constructor.as_ref())
                {
                    if a_id != b_id {
                        return Err(UnifyError::AmbiguousConstructorHeads {
                            left: a.clone(),
                            right: b.clone(),
                        });
                    }
                }
                if a_args.len() != b_args.len() {
                    return Err(UnifyError::ArityMismatch {
                        kind: "application",
                        expected: a_args.len(),
                        found: b_args.len(),
                        left: a.clone(),
                        right: b.clone(),
                    });
                }
                self.unify(a_constructor, b_constructor)?;
                for (a_arg, b_arg) in a_args.iter().zip(b_args) {
                    self.unify(a_arg, b_arg)?;
                }
                Ok(())
            }
            (
                Type::Lambda {
                    params: a_params,
                    body: a_body,
                },
                Type::Lambda {
                    params: b_params,
                    body: b_body,
                },
            ) => {
                if a_params != b_params {
                    return Err(UnifyError::Mismatch {
                        expected: a.clone(),
                        found: b.clone(),
                    });
                }
                self.unify(a_body, b_body)
            }
            (
                Type::BoundVar {
                    depth: a_depth,
                    index: a_index,
                    kind: a_kind,
                },
                Type::BoundVar {
                    depth: b_depth,
                    index: b_index,
                    kind: b_kind,
                },
            ) if a_depth == b_depth && a_index == b_index && a_kind == b_kind => Ok(()),
            // Allow integer literal type coercion
            (a_ty, b_ty) if TypeFacts::is_integer(a_ty) && TypeFacts::is_integer(b_ty) => {
                // We'll allow implicit integer coercion for now
                Ok(())
            }
            _ => Err(UnifyError::Mismatch {
                expected: a.clone(),
                found: b.clone(),
            }),
        }
    }
}

use crate::ids::TypeVarId;
use crate::type_services::normalize::TypeNormalizer;
use crate::types::Type;

use super::errors::UnifyError;
use super::InferenceEngine;

impl InferenceEngine {
    pub(super) fn bind_type_var(&mut self, id: TypeVarId, ty: Type) -> Result<(), UnifyError> {
        if self.substitutions.get(&id) == Some(&ty) {
            return Ok(());
        }
        let variable_kind = self.kind_of_type_var(id);
        let type_kind = TypeNormalizer::new(&self.normalization_env)
            .kind_of(&ty)
            .map_err(|error| UnifyError::Normalization {
                ty: ty.clone(),
                detail: error,
            })?;
        if variable_kind != type_kind {
            return Err(UnifyError::KindMismatch {
                expected: variable_kind,
                found: type_kind,
                ty: Some(ty),
                context: "binding",
            });
        }
        if Self::has_escaping_bound_var(&ty, 0) {
            return Err(UnifyError::BinderEscape { ty });
        }
        if self.occurs_in(id, &ty) {
            return Err(UnifyError::InfiniteType { variable: id, ty });
        }
        if let (Type::TypeVar(target_id), Some(span)) = (&ty, self.var_spans.get(&id).cloned()) {
            self.var_spans.entry(*target_id).or_insert(span);
        }
        self.substitutions.insert(id, ty);
        self.substitution_generation = self.substitution_generation.wrapping_add(1);
        if !crate::type_services::visit::type_any(&self.substitutions[&id], |nested| {
            matches!(nested, Type::TypeVar(_))
        }) {
            self.rigid_substitution_generation = self.rigid_substitution_generation.wrapping_add(1);
        }
        self.changed_type_vars.push(id);
        if let Type::TypeVar(target_id) = self.substitutions[&id].clone() {
            self.changed_type_vars.push(target_id);
        }
        Ok(())
    }

    pub(crate) fn bind_pending_type_var(
        &mut self,
        id: TypeVarId,
        ty: &Type,
    ) -> Result<(), UnifyError> {
        match self.resolve(&Type::TypeVar(id)) {
            Type::TypeVar(current) => self.bind_type_var(current, ty.clone()),
            current => self.unify(&current, ty),
        }
    }

    pub(crate) fn normalize_resolved_type(&self, ty: &Type) -> Result<Type, String> {
        TypeNormalizer::new(&self.normalization_env)
            .normalize(&self.resolve(ty))
            .map_err(|error| error.to_string())
    }

    pub(super) fn canonicalize_type_var_alias(&mut self, original: &Type, other: &Type) {
        let Type::TypeVar(id) = original else {
            return;
        };
        let Ok(normalized) =
            TypeNormalizer::new(&self.normalization_env).normalize(&self.resolve(other))
        else {
            return;
        };
        if matches!(normalized, Type::TypeVar(_)) || self.occurs_in(*id, &normalized) {
            return;
        }
        if self.substitutions.contains_key(id) && self.substitutions.get(id) != Some(&normalized) {
            self.substitutions.insert(*id, normalized);
            self.substitution_generation = self.substitution_generation.wrapping_add(1);
            self.changed_type_vars.push(*id);
        }
    }

    pub(super) fn try_unify_constructor_spine(
        &mut self,
        pattern: &Type,
        actual: &Type,
    ) -> Option<Result<(), UnifyError>> {
        let Type::Apply {
            constructor,
            args: pattern_args,
        } = pattern
        else {
            return None;
        };
        let Type::TypeVar(constructor_var) = constructor.as_ref() else {
            return None;
        };
        if matches!(
            actual,
            Type::TypeVar(id) if id == constructor_var
        ) || matches!(
            actual,
            Type::Apply { constructor, .. }
                if matches!(constructor.as_ref(), Type::TypeVar(id) if id == constructor_var)
        ) {
            return None;
        }
        if let Type::TypeVar(actual_var) = actual {
            return self
                .occurs_in(*actual_var, pattern)
                .then_some(Err(Self::constructor_inference_error(pattern, actual)));
        }
        if matches!(
            actual,
            Type::Apply {
                constructor,
                ..
            } if matches!(constructor.as_ref(), Type::TypeVar(_))
        ) {
            return None;
        }
        let Some((actual_constructor, actual_args)) = Self::rigid_spine(actual) else {
            return Some(Err(Self::constructor_inference_error(pattern, actual)));
        };
        if actual_args.len() < pattern_args.len() {
            return Some(Err(Self::constructor_inference_error(pattern, actual)));
        }

        let mut probe = self.clone_for_probe();
        if actual_args.len() == pattern_args.len() {
            if probe
                .bind_type_var(*constructor_var, actual_constructor)
                .is_err()
            {
                return Some(Err(Self::constructor_inference_error(pattern, actual)));
            }
            for (pattern_arg, actual_arg) in pattern_args.iter().zip(actual_args.iter()) {
                if probe.unify(pattern_arg, actual_arg).is_err() {
                    return Some(Err(Self::constructor_inference_error(pattern, actual)));
                }
            }
            *self = probe;
            return Some(Ok(()));
        }
        let pattern_kinds = pattern_args
            .iter()
            .map(|arg| {
                TypeNormalizer::new(&probe.normalization_env)
                    .kind_of(&probe.resolve(arg))
                    .ok()
            })
            .collect::<Option<Vec<_>>>();
        let Some(pattern_kinds) = pattern_kinds else {
            return Some(Err(Self::constructor_inference_error(pattern, actual)));
        };
        let mut hole_indices = std::collections::HashMap::new();
        let mut section_params = Vec::new();
        let section_args = pattern_args
            .iter()
            .zip(pattern_kinds.iter())
            .map(|(pattern_arg, kind)| {
                let hole_index = if let Type::TypeVar(id) = pattern_arg {
                    if let Some(index) = hole_indices.get(id).copied() {
                        index
                    } else {
                        let index = section_params.len() as u32;
                        hole_indices.insert(*id, index);
                        section_params.push(kind.clone());
                        index
                    }
                } else {
                    let index = section_params.len() as u32;
                    section_params.push(kind.clone());
                    index
                };
                Type::BoundVar {
                    depth: 0,
                    index: hole_index,
                    kind: kind.clone(),
                }
            })
            .chain(actual_args[pattern_args.len()..].iter().cloned())
            .collect::<Vec<_>>();
        let section_body = if section_args.len() == actual_args.len() {
            match actual {
                Type::Struct { id, .. } => Type::Struct {
                    id: *id,
                    args: section_args,
                },
                Type::Enum { id, .. } => Type::Enum {
                    id: *id,
                    args: section_args,
                },
                _ => Type::Apply {
                    constructor: Box::new(actual_constructor),
                    args: section_args,
                },
            }
        } else {
            Type::Apply {
                constructor: Box::new(actual_constructor),
                args: section_args,
            }
        };
        let section = if pattern_args.is_empty() {
            section_body
        } else {
            Type::Lambda {
                params: section_params,
                body: Box::new(section_body),
            }
        };
        for (pattern_arg, actual_arg) in pattern_args.iter().zip(actual_args.iter()) {
            if probe.unify(pattern_arg, actual_arg).is_err() {
                return Some(Err(Self::constructor_inference_error(pattern, actual)));
            }
        }
        if probe.bind_type_var(*constructor_var, section).is_err() {
            return Some(Err(Self::constructor_inference_error(pattern, actual)));
        }
        for (pattern_arg, actual_arg) in pattern_args.iter().zip(actual_args.iter()) {
            if probe.unify(pattern_arg, actual_arg).is_err() {
                return Some(Err(Self::constructor_inference_error(pattern, actual)));
            }
        }
        *self = probe;
        Some(Ok(()))
    }

    fn rigid_spine(ty: &Type) -> Option<(Type, Vec<Type>)> {
        match ty {
            Type::Struct { id, args } => Some((
                Type::Constructor {
                    id: *id,
                    flavor: crate::types::NominalTypeKind::Struct,
                },
                args.clone(),
            )),
            Type::Enum { id, args } => Some((
                Type::Constructor {
                    id: *id,
                    flavor: crate::types::NominalTypeKind::Enum,
                },
                args.clone(),
            )),
            Type::Apply { constructor, args }
                if !matches!(
                    constructor.as_ref(),
                    Type::TypeVar(_) | Type::BoundVar { .. }
                ) =>
            {
                Some((constructor.as_ref().clone(), args.clone()))
            }
            Type::Constructor { .. } | Type::Generic(_) | Type::Projection { .. } => {
                Some((ty.clone(), Vec::new()))
            }
            _ => None,
        }
    }

    fn constructor_inference_error(pattern: &Type, actual: &Type) -> UnifyError {
        UnifyError::ConstructorInference {
            pattern: pattern.clone(),
            actual: actual.clone(),
        }
    }

    fn has_escaping_bound_var(ty: &Type, binder_depth: u32) -> bool {
        match ty {
            Type::BoundVar { depth, .. } => *depth >= binder_depth,
            Type::Slice(inner) | Type::Pointer(inner) => {
                Self::has_escaping_bound_var(inner, binder_depth)
            }
            Type::Array(inner, _) | Type::Reference { inner, .. } => {
                Self::has_escaping_bound_var(inner, binder_depth)
            }
            Type::Tuple(items)
            | Type::Struct { args: items, .. }
            | Type::Enum { args: items, .. } => items
                .iter()
                .any(|item| Self::has_escaping_bound_var(item, binder_depth)),
            Type::Function {
                params,
                ret,
                captures,
                ..
            } => {
                params
                    .iter()
                    .any(|param| Self::has_escaping_bound_var(param, binder_depth))
                    || Self::has_escaping_bound_var(ret, binder_depth)
                    || captures
                        .iter()
                        .any(|capture| Self::has_escaping_bound_var(&capture.ty, binder_depth))
            }
            Type::Projection { ty, trait_args, .. } => {
                Self::has_escaping_bound_var(ty, binder_depth)
                    || trait_args
                        .iter()
                        .any(|arg| Self::has_escaping_bound_var(arg, binder_depth))
            }
            Type::Apply { constructor, args } => {
                Self::has_escaping_bound_var(constructor, binder_depth)
                    || args
                        .iter()
                        .any(|arg| Self::has_escaping_bound_var(arg, binder_depth))
            }
            Type::Lambda { body, .. } => {
                Self::has_escaping_bound_var(body, binder_depth.saturating_add(1))
            }
            _ => false,
        }
    }
}

//! Receiver inference for unresolved operator operands.

use crate::hir::*;
use crate::lower::Lowerer;
use crate::selection::SelectionDiagnostic;
use crate::types::Type;

impl Lowerer {
    pub(super) fn type_accepts_inference_vars(expected: &Type, actual: &Type) -> bool {
        match (expected, actual) {
            (Type::TypeVar(_), _) | (_, Type::TypeVar(_)) => true,
            (
                Type::Function {
                    params: expected_args,
                    ret: expected_ret,
                    ..
                },
                Type::Function {
                    params: actual_args,
                    ret: actual_ret,
                    ..
                },
            ) => {
                expected_args.len() == actual_args.len()
                    && expected_args
                        .iter()
                        .zip(actual_args)
                        .all(|(expected, actual)| {
                            Self::type_accepts_inference_vars(expected, actual)
                        })
                    && Self::type_accepts_inference_vars(expected_ret, actual_ret)
            }
            (
                Type::Struct {
                    id: expected_id,
                    args: expected_args,
                },
                Type::Struct {
                    id: actual_id,
                    args: actual_args,
                },
            )
            | (
                Type::Enum {
                    id: expected_id,
                    args: expected_args,
                },
                Type::Enum {
                    id: actual_id,
                    args: actual_args,
                },
            ) => {
                expected_id == actual_id
                    && expected_args.len() == actual_args.len()
                    && expected_args
                        .iter()
                        .zip(actual_args)
                        .all(|(expected, actual)| {
                            Self::type_accepts_inference_vars(expected, actual)
                        })
            }
            (
                Type::Reference {
                    mutable: expected_mutable,
                    inner: expected_inner,
                },
                Type::Reference {
                    mutable: actual_mutable,
                    inner: actual_inner,
                },
            ) => {
                (!expected_mutable || *actual_mutable)
                    && Self::type_accepts_inference_vars(expected_inner, actual_inner)
            }
            (Type::Slice(expected), Type::Slice(actual))
            | (Type::Pointer(expected), Type::Pointer(actual)) => {
                Self::type_accepts_inference_vars(expected, actual)
            }
            (Type::Array(expected, expected_len), Type::Array(actual, actual_len)) => {
                (expected_len == actual_len
                    || matches!(expected_len, crate::types::ArrayLen::Var(_))
                    || matches!(actual_len, crate::types::ArrayLen::Var(_)))
                    && Self::type_accepts_inference_vars(expected, actual)
            }
            _ => expected == actual,
        }
    }

    pub(super) fn type_has_unresolved_parameter(ty: &Type) -> bool {
        crate::type_services::visit::type_any(ty, |nested| {
            matches!(nested, Type::TypeVar(_) | Type::Generic(_))
        })
    }

    pub(super) fn infer_unresolved_binary_operator_receiver(
        &mut self,
        left: &HirExpr,
        right: &HirExpr,
        method_name: &str,
    ) -> Result<bool, SelectionDiagnostic> {
        let right_ty = self.resolve_projection_type(&self.engine.resolve(&right.ty));
        let mut candidates = Vec::new();
        for (_, imp) in self.items.impl_defs_in_order() {
            let Some(crate::hir::HirImplReceiverPattern::Exact(receiver_ty)) =
                Some(&imp.receiver_pattern)
            else {
                continue;
            };
            if !receiver_ty.is_concrete() {
                continue;
            }

            let Some(method) = imp.methods.get(method_name) else {
                continue;
            };
            let param_start = if method.is_method { 1 } else { 0 };
            let Some(arg_param) = method.params.get(param_start) else {
                continue;
            };
            let expected_arg_ty = self.resolve_projection_type(&arg_param.ty);
            let Some(arg_unify_ty) =
                Self::operator_candidate_arg_unify_type(&expected_arg_ty, &right_ty)
            else {
                continue;
            };

            let candidate = (imp.id, receiver_ty.clone(), arg_unify_ty);
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }

        let mut candidate_shapes = candidates
            .iter()
            .map(|(_, receiver_ty, arg_ty)| (receiver_ty.clone(), arg_ty.clone()))
            .collect::<Vec<_>>();
        candidate_shapes.sort_by_key(|candidate| format!("{candidate:?}"));
        candidate_shapes.dedup();
        let candidate = if let [candidate] = candidate_shapes.as_slice() {
            candidate
        } else {
            let default = match self.engine.resolve(&left.ty) {
                Type::TypeVar(id) => self
                    .constraint_store
                    .literal_default_type_for_representative(id, |var| {
                        self.engine
                            .unresolved_type_var_representative_for_dependency(var)
                    }),
                _ => None,
            };
            if let Some(default) = default.as_ref() {
                if let Some(candidate) = candidate_shapes
                    .iter()
                    .find(|(receiver_ty, _)| receiver_ty == default)
                {
                    candidate
                } else {
                    return Self::ambiguous_operator_candidates(
                        &self.engine,
                        &left.ty,
                        method_name,
                        &candidates,
                    );
                }
            } else {
                return Self::ambiguous_operator_candidates(
                    &self.engine,
                    &left.ty,
                    method_name,
                    &candidates,
                );
            }
        };
        let (receiver_ty, expected_arg_ty) = candidate;

        Ok(self.engine.unify(&left.ty, receiver_ty).is_ok()
            && self.engine.unify(&right.ty, expected_arg_ty).is_ok())
    }

    fn operator_candidate_arg_unify_type(expected: &Type, actual: &Type) -> Option<Type> {
        if let Type::Reference {
            mutable: false,
            inner,
        } = expected
        {
            if matches!(actual, Type::TypeVar(_)) || inner.as_ref() == actual {
                return Some(inner.as_ref().clone());
            }
        }

        if matches!(actual, Type::TypeVar(_)) || expected == actual {
            return Some(expected.clone());
        }

        None
    }

    pub(super) fn infer_unresolved_unary_operator_receiver(
        &mut self,
        inner: &HirExpr,
        method_name: &str,
    ) -> Result<bool, SelectionDiagnostic> {
        let mut candidates = Vec::new();
        for (_, imp) in self.items.impl_defs_in_order() {
            let Some(crate::hir::HirImplReceiverPattern::Exact(receiver_ty)) =
                Some(&imp.receiver_pattern)
            else {
                continue;
            };
            if !receiver_ty.is_concrete() {
                continue;
            }

            let Some(method) = imp.methods.get(method_name) else {
                continue;
            };
            let param_start = if method.is_method { 1 } else { 0 };
            if method.params.len() == param_start {
                let candidate = (imp.id, receiver_ty.clone());
                if !candidates.contains(&candidate) {
                    candidates.push(candidate);
                }
            }
        }

        let mut receiver_types = candidates
            .iter()
            .map(|(_, receiver_ty)| receiver_ty.clone())
            .collect::<Vec<_>>();
        receiver_types.sort_by_key(|receiver_ty| format!("{receiver_ty:?}"));
        receiver_types.dedup();
        let receiver_ty = if let [receiver_ty] = receiver_types.as_slice() {
            receiver_ty
        } else {
            let default = match self.engine.resolve(&inner.ty) {
                Type::TypeVar(id) => self
                    .constraint_store
                    .literal_default_type_for_representative(id, |var| {
                        self.engine
                            .unresolved_type_var_representative_for_dependency(var)
                    }),
                _ => None,
            };
            if let Some(default) = default.as_ref() {
                if let Some(receiver_ty) = receiver_types.iter().find(|ty| *ty == default) {
                    receiver_ty
                } else {
                    return Self::ambiguous_unary_operator_candidates(
                        &self.engine,
                        &inner.ty,
                        method_name,
                        &candidates,
                    );
                }
            } else {
                return Self::ambiguous_unary_operator_candidates(
                    &self.engine,
                    &inner.ty,
                    method_name,
                    &candidates,
                );
            }
        };

        Ok(self.engine.unify(&inner.ty, receiver_ty).is_ok())
    }
}

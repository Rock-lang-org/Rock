use std::collections::HashMap;

use crate::ids::{DefId, TypeId};
use crate::mono::hir_types::{
    HirCallTarget, HirExpr, HirExprKind, HirFunction, HirImpl, HirMethodCallTarget,
};
use crate::types::{GenericParamDecl, GenericParamId, Type};

use crate::mono::Monomorphizer;

impl Monomorphizer {
    pub(super) fn impl_matches_method_target(
        imp: &HirImpl,
        target: Option<&HirMethodCallTarget>,
    ) -> bool {
        crate::selection::target_matches_impl(imp, target)
    }

    pub(super) fn impl_trait_args_match_selected_target(
        &self,
        imp: &HirImpl,
        recv_ty: &Type,
        target: Option<&HirMethodCallTarget>,
    ) -> bool {
        let Some(target) = target else {
            return true;
        };
        if imp.trait_id.is_none() && target.trait_id().is_none() {
            return true;
        }
        if imp.trait_arg_types.len() != target.trait_args().len() {
            return false;
        }
        let Some(mut substitution) = crate::selection::impl_receiver_pattern_substitution(
            imp,
            recv_ty,
            self.trait_impls.values().flatten(),
        ) else {
            return false;
        };
        if target.impl_id() == Some(imp.id) {
            for binding in &target.owner_substitution {
                if !crate::selection::type_pattern_matches(
                    &Type::Generic(binding.param),
                    &binding.ty,
                    &mut substitution,
                ) {
                    return false;
                }
            }
        }
        imp.trait_arg_types
            .iter()
            .zip(target.trait_args())
            .all(|(expected, actual)| {
                crate::selection::type_pattern_matches(expected, actual, &mut substitution)
            })
    }

    pub(super) fn method_generic_ids_in_order(method: &HirFunction) -> Vec<GenericParamId> {
        method.generic_params.iter().map(|param| param.id).collect()
    }

    pub(super) fn combine_impl_and_method_type_args(
        &mut self,
        method: &HirFunction,
        args: &[HirExpr],
        receiver_type_args: &[TypeId],
        type_generics: &[GenericParamDecl],
        impl_id: DefId,
        target: &HirMethodCallTarget,
    ) -> Option<Vec<TypeId>> {
        let mut type_args = if target.impl_id() == Some(impl_id) {
            type_generics
                .iter()
                .map(|decl| {
                    let param = decl.id;
                    target
                        .owner_substitution
                        .iter()
                        .find(|binding| binding.param == param)
                        .map(|binding| self.intern_type(&binding.ty))
                })
                .collect::<Option<Vec<_>>>()?
        } else {
            receiver_type_args.to_vec()
        };
        let method_ids = Self::method_generic_ids_in_order(method)
            .into_iter()
            .filter(|id| !type_generics.iter().any(|decl| decl.id == *id))
            .collect::<Vec<_>>();
        let trait_member_id = match &target.target {
            crate::hir::HirSelectedMethodTarget::ImplMethod {
                selected_trait: Some(selected_trait),
                ..
            } => Some(selected_trait.member_id),
            crate::hir::HirSelectedMethodTarget::TraitMethod { member_id, .. } => Some(*member_id),
            crate::hir::HirSelectedMethodTarget::ImplMethod {
                selected_trait: None,
                ..
            } => None,
        };
        let trait_method_bindings = trait_member_id
            .map(|member_id| {
                target
                    .method_substitution
                    .iter()
                    .filter(|binding| binding.param.owner == member_id)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut inferred = type_generics
            .iter()
            .zip(receiver_type_args.iter().copied())
            .map(|(decl, ty)| (decl.id, ty))
            .collect::<HashMap<_, _>>();
        let param_start = usize::from(method.is_method);
        for (param, arg) in method
            .params
            .iter()
            .skip(param_start)
            .zip(args.iter().skip(param_start))
        {
            self.extract_generics_from_type(&param.ty, &arg.ty, &method_ids, &mut inferred);
        }
        self.infer_native_callable_bound_arguments(method, &method_ids, &mut inferred);
        for (index, id) in method_ids.into_iter().enumerate() {
            let binding = target
                .method_substitution
                .iter()
                .find(|binding| binding.param == id)
                .map(|binding| self.intern_type(&binding.ty))
                .or_else(|| inferred.get(&id).copied())
                .or_else(|| {
                    trait_method_bindings
                        .get(index)
                        .map(|binding| self.intern_type(&binding.ty))
                });
            if let Some(binding) = binding {
                type_args.push(binding);
            } else if Some(id.owner) != target.trait_id() {
                return None;
            }
        }
        Some(type_args)
    }

    pub(super) fn rewrite_to_existing_instance_call(
        &self,
        expr: &mut HirExpr,
        args: &[HirExpr],
        instance_id: crate::mono::InstanceId,
    ) -> bool {
        let body = self.instances.pre_mir_body(instance_id).cloned();
        let Some(record) = self.instances.record(instance_id) else {
            return false;
        };
        let display_name = record.symbols.source_name.clone();
        let func = body.or_else(|| record.declared.clone());
        let (param_types, ret_type, safety) = if let Some(func) = func {
            (
                func.params.iter().map(|param| param.ty.clone()).collect(),
                func.ret_type,
                crate::types::FunctionSafety::from_is_unsafe(func.is_unsafe),
            )
        } else {
            (
                args.iter().map(|arg| arg.ty.clone()).collect(),
                expr.ty.clone(),
                crate::types::FunctionSafety::Safe,
            )
        };

        expr.kind = HirExprKind::Call(
            Box::new(self.instance_callable_expr(
                display_name,
                instance_id,
                Type::function_with_safety(param_types, ret_type.clone(), safety),
                expr.span.clone(),
            )),
            args.to_vec(),
            Some(HirCallTarget::Instance(instance_id)),
        );
        expr.ty = ret_type;
        true
    }

    /// Extract type arguments from a receiver type based on impl type parameters
    pub(super) fn extract_type_args_from_selected_target(
        &mut self,
        imp: &HirImpl,
        target: &HirMethodCallTarget,
    ) -> Option<Vec<TypeId>> {
        imp.type_generics
            .iter()
            .map(|decl| {
                target
                    .owner_substitution
                    .iter()
                    .find(|binding| binding.param == decl.id)
                    .map(|binding| binding.ty.clone())
                    .filter(|ty| !matches!(ty, Type::Generic(_) | Type::TypeVar(_)))
                    .map(|ty| self.intern_type(&ty))
            })
            .collect()
    }

    pub(super) fn extract_type_args_from_receiver_pattern(
        &mut self,
        imp: &HirImpl,
        recv_ty: &Type,
        trait_args: &[Type],
    ) -> Option<Vec<TypeId>> {
        let mut substitution = crate::selection::impl_receiver_pattern_substitution(
            imp,
            recv_ty,
            self.trait_impls.values().flatten(),
        )?;
        for (expected, actual) in imp.trait_arg_types.iter().zip(trait_args) {
            if !crate::selection::type_pattern_matches(expected, actual, &mut substitution) {
                return None;
            }
        }
        imp.type_generics
            .iter()
            .map(|decl| {
                substitution
                    .get(&decl.id)
                    .cloned()
                    .filter(|ty| !matches!(ty, Type::Generic(_) | Type::TypeVar(_)))
                    .map(|ty| self.intern_type(&ty))
            })
            .collect()
    }
}

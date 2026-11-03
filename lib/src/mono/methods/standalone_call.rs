use crate::ids::{CrateId, DefId, LocalDefId};
use crate::mono::hir_types::{HirCallTarget, HirExpr, HirExprKind};
use crate::types::Type;

use crate::mono::Monomorphizer;

impl Monomorphizer {
    /// Monomorphize a standalone generic impl method call (e.g. Vec T -> Vec_I64_push)
    pub(in crate::mono) fn monomorphize_standalone_method_call(
        &mut self,
        _method_name: &str,
        args: &[HirExpr],
        expr: &mut HirExpr,
    ) -> Result<(), crate::mono::MonoError> {
        let span = expr.span.clone();
        let error = |kind| crate::mono::MonoError {
            kind,
            span: span.clone(),
        };
        let selected_target = match &expr.kind {
            HirExprKind::MethodCall(_, _, _, _, target) => target.clone(),
            _ => {
                return Err(error(crate::mono::MonoErrorKind::InvalidBindings {
                    target: crate::hir::HirSelectedMethodTarget::TraitMethod {
                        trait_id: DefId::new(CrateId(0), LocalDefId(u32::MAX)),
                        member_id: DefId::new(CrateId(0), LocalDefId(u32::MAX - 1)),
                        trait_args: Vec::new(),
                        dispatch: crate::hir::HirTraitDispatchKind::TraitBound,
                    },
                }))
            }
        };
        if let (Some(impl_id), Some(method_id)) =
            (selected_target.impl_id(), selected_target.method_id())
        {
            let substitution = selected_target
                .owner_substitution
                .iter()
                .chain(selected_target.method_substitution.iter())
                .filter(|binding| !matches!(binding.ty, Type::Generic(_) | Type::TypeVar(_)))
                .map(|binding| self.intern_type(&binding.ty))
                .collect::<Vec<_>>();
            let instance_id = self.instances.records().find_map(|record| {
                let matches_origin = matches!(
                    record.origin,
                    crate::mono::InstanceOrigin::ImplMethod {
                        owner: crate::mono::InstanceImplOwner::Named(owner),
                        method,
                    } if owner == impl_id && method == method_id
                );
                (matches_origin
                    && (record.substitution == substitution
                        || (substitution.is_empty() && record.substitution.is_empty())))
                .then_some(record.id)
            });
            if let Some(instance_id) = instance_id {
                if self.rewrite_to_existing_instance_call(expr, args, instance_id) {
                    return Ok(());
                }
            }
        }
        let exact_impl = selected_target.impl_id().and_then(|impl_id| {
            let method_id = selected_target.method_id()?;
            self.generic_impls.values().find_map(|imp| {
                (imp.id == impl_id).then(|| {
                    imp.methods
                        .iter()
                        .find_map(|(resolved_method_name, method)| {
                            (method.id == method_id).then_some((
                                imp.clone(),
                                resolved_method_name.clone(),
                                method.clone(),
                            ))
                        })
                })?
            })
        });
        let found = exact_impl.and_then(|(imp, resolved_method_name, method)| {
            let receiver_ty = &args.first()?.ty;
            if !self.impl_receiver_pattern_matches(&imp, receiver_ty)
                || !self.impl_trait_args_match_selected_target(
                    &imp,
                    receiver_ty,
                    Some(&selected_target),
                )
            {
                return None;
            }
            let type_args = if !imp.type_generics.is_empty() || !method.generic_params.is_empty() {
                let receiver_type_args =
                    self.extract_type_args_from_selected_target(&imp, &selected_target)?;
                if receiver_type_args.len() != imp.type_generics.len() {
                    return None;
                }
                self.combine_impl_and_method_type_args(
                    &method,
                    args,
                    &receiver_type_args,
                    &imp.type_generics,
                    imp.id,
                    &selected_target,
                )?
            } else {
                Vec::new()
            };

            Some((
                method.clone(),
                type_args,
                imp.type_generics.clone(),
                imp.type_name.clone(),
                imp.clone(),
                resolved_method_name,
            ))
        });

        if let Some((method, type_args, type_generics, impl_type_name, imp, resolved_method_name)) =
            found
        {
            if method.generic_params.is_empty() && type_generics.is_empty() {
                let instance_id =
                    self.register_impl_method_instance(&imp, &resolved_method_name, &method);
                if !self.rewrite_to_existing_instance_call(expr, args, instance_id) {
                    return Err(error(crate::mono::MonoErrorKind::MissingInstance {
                        origin: self.method_instance_origin(&imp, &method),
                    }));
                }
                return Ok(());
            }
            let receiver_suffix = Self::receiver_mono_suffix(&args[0].ty);
            let type_suffix = if type_args.is_empty() {
                receiver_suffix
            } else {
                let structural_type_args = type_args
                    .iter()
                    .map(|id| self.type_for(*id))
                    .collect::<Vec<_>>();
                format!(
                    "{}_{}",
                    receiver_suffix,
                    structural_type_args
                        .iter()
                        .map(|t| Self::type_to_mono_suffix(t))
                        .collect::<Vec<_>>()
                        .join("_")
                )
            };
            let specialized_name = format!(
                "{}_{}_{}",
                impl_type_name, type_suffix, resolved_method_name
            );
            let origin = self.method_instance_origin(&imp, &method);
            let substitution = type_args.clone();
            let instance_key = crate::mono::InstanceKey::new(origin.clone(), substitution.clone());
            let backend_symbol = self.backend_symbol_for_origin(&origin, &substitution);

            if let Some(instance_id) = self.instances.get(&instance_key) {
                if self.rewrite_to_existing_instance_call(expr, args, instance_id) {
                    return Ok(());
                }
            }

            let instance_id =
                self.instances
                    .intern(instance_key, |id| crate::mono::InstanceRecord {
                        id,
                        origin: origin.clone(),
                        substitution: substitution.clone(),
                        symbols: crate::mono::InstanceSymbols::new(
                            format!("{}::{}", impl_type_name, resolved_method_name),
                            backend_symbol.clone(),
                        ),
                        declared: None,
                        provided_by_object: false,
                        is_specialization: true,
                    });

            let specialized_func = self.specialize_impl_method(
                &method,
                &type_args,
                &type_generics,
                &specialized_name,
                imp.id,
                imp.trait_id,
            );
            let concrete_ret_ty = specialized_func.ret_type.clone();
            self.instances
                .insert_pre_mir_body(instance_id, specialized_func.clone());
            let concrete_ret_ty = self
                .instances
                .pre_mir_body(instance_id)
                .map(|body| body.ret_type.clone())
                .unwrap_or(concrete_ret_ty);

            let call_args = args.to_vec();
            let param_types = specialized_func
                .params
                .iter()
                .map(|param| param.ty.clone())
                .collect();
            let callee = self.instance_callable_expr(
                format!("{}::{}", impl_type_name, resolved_method_name),
                instance_id,
                Type::function_with_safety(
                    param_types,
                    concrete_ret_ty.clone(),
                    crate::types::FunctionSafety::from_is_unsafe(specialized_func.is_unsafe),
                ),
                expr.span.clone(),
            );
            expr.kind = HirExprKind::Call(
                Box::new(callee),
                call_args,
                Some(HirCallTarget::Instance(instance_id)),
            );
            expr.ty = concrete_ret_ty;
            Ok(())
        } else {
            Err(error(if let Some(impl_id) = selected_target.impl_id() {
                crate::mono::MonoErrorKind::ReceiverMismatch {
                    impl_id,
                    receiver: args.first().map(|arg| arg.ty.clone()).unwrap_or(Type::Unit),
                    pattern: self.impl_receiver_pattern_by_id(impl_id).cloned(),
                }
            } else {
                crate::mono::MonoErrorKind::InvalidBindings {
                    target: selected_target.target.clone(),
                }
            }))
        }
    }
}

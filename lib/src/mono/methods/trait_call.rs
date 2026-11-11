use crate::ids::{CrateId, DefId, LocalDefId};
use crate::mono::hir_types::{HirCallTarget, HirExpr, HirExprKind};
use crate::types::Type;

use crate::mono::Monomorphizer;

impl Monomorphizer {
    /// Monomorphize a trait method call by finding the impl and specializing it
    pub(in crate::mono) fn monomorphize_trait_method_call(
        &mut self,
        method_name: &str,
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
        let trait_id = selected_target.trait_id().ok_or_else(|| {
            error(crate::mono::MonoErrorKind::InvalidBindings {
                target: selected_target.target.clone(),
            })
        })?;
        let member_id = selected_target.method_id().ok_or_else(|| {
            error(crate::mono::MonoErrorKind::InvalidBindings {
                target: selected_target.target.clone(),
            })
        })?;
        let mut matching = Vec::new();
        let trait_impls = selected_target
            .trait_id()
            .and_then(|trait_id| self.trait_impls.get(&trait_id).cloned())
            .unwrap_or_default();
        for imp in trait_impls {
            if !Self::impl_matches_method_target(&imp, Some(&selected_target)) {
                continue;
            }
            let selected_impl = selected_target
                .impl_id()
                .is_some_and(|impl_id| impl_id == imp.id);
            if !self.impl_receiver_pattern_matches(&imp, &args[0].ty) {
                continue;
            }
            if !self.impl_trait_args_match_selected_target(
                &imp,
                &args[0].ty,
                Some(&selected_target),
            ) {
                continue;
            }
            let method = self.method_for_selected_target(&imp, &selected_target);

            if let Some(method) = method {
                let type_args =
                    if !method.generic_params.is_empty() || !imp.type_generics.is_empty() {
                        let receiver_type_args = if selected_impl {
                            self.extract_type_args_from_selected_target(&imp, &selected_target)
                                .unwrap_or_default()
                        } else {
                            self.extract_type_args_from_receiver_pattern(
                                &imp,
                                &args[0].ty,
                                selected_target.trait_args(),
                            )
                            .unwrap_or_default()
                        };

                        if receiver_type_args.len() != imp.type_generics.len() {
                            continue;
                        }

                        let Some(type_args) = self.combine_impl_and_method_type_args(
                            method,
                            args,
                            &receiver_type_args,
                            &imp.type_generics,
                            imp.id,
                            &selected_target,
                        ) else {
                            continue;
                        };
                        type_args
                    } else {
                        Vec::new()
                    };
                matching.push((
                    method.clone(),
                    type_args,
                    imp.type_generics.clone(),
                    imp.type_name.clone(),
                    imp.clone(),
                ));
            }
        }

        matching.sort_by_key(|candidate| candidate.4.id);
        matching.dedup_by_key(|candidate| candidate.4.id);
        if matching.len() > 1 {
            return Err(error(crate::mono::MonoErrorKind::AmbiguousImpls {
                trait_id,
                member_id,
                impls: matching.iter().map(|candidate| candidate.4.id).collect(),
            }));
        }
        let found = matching.pop();

        if let Some((method, type_args, type_generics, impl_type_name, imp)) = found {
            let origin = self.method_instance_origin(&imp, &method);
            let substitution = type_args.clone();
            let needs_specialization =
                !method.generic_params.is_empty() || !type_generics.is_empty();
            let instance_key = crate::mono::InstanceKey::new(origin.clone(), substitution.clone());
            let backend_symbol = String::new();
            if let Some(instance_id) = self.instances.get(&instance_key) {
                if self.rewrite_to_existing_instance_call(expr, args, instance_id) {
                    return Ok(());
                }
            }
            if !needs_specialization {
                let instance_id = self.instances.get(&instance_key).unwrap_or_else(|| {
                    self.register_impl_method_instance(&imp, &method.name, &method)
                });
                if !self.rewrite_to_existing_instance_call(expr, args, instance_id) {
                    return Err(error(crate::mono::MonoErrorKind::MissingInstance {
                        origin,
                    }));
                }
                return Ok(());
            }

            let instance_id =
                self.instances
                    .intern(instance_key, |id| crate::mono::InstanceRecord {
                        id,
                        origin: origin.clone(),
                        substitution: substitution.clone(),
                        symbols: crate::mono::InstanceSymbols::new(
                            format!("{}::{}", impl_type_name, method_name),
                            backend_symbol.clone(),
                        ),
                        declared: None,
                        provided_by_object: false,
                        is_specialization: true,
                    });

            let receiver_suffix = Self::receiver_mono_suffix(&args[0].ty);
            let specialized_name = format!(
                "{}_{}_{}_mono_{}",
                impl_type_name, receiver_suffix, method_name, self.counter
            );
            self.counter += 1;

            let specialized_func = self.specialize_impl_method(
                &method,
                &type_args,
                &type_generics,
                &specialized_name,
                imp.id,
                imp.trait_id,
            );

            self.instances
                .insert_pre_mir_body(instance_id, specialized_func.clone());

            let mut call_args = vec![args[0].clone()];
            for arg in &args[1..] {
                call_args.push(arg.clone());
            }

            let param_types = specialized_func
                .params
                .iter()
                .map(|param| param.ty.clone())
                .collect();
            let specialized_ret_ty = specialized_func.ret_type.clone();

            let callee = self.instance_callable_expr(
                format!("{}::{}", impl_type_name, method_name),
                instance_id,
                Type::function_with_safety(
                    param_types,
                    specialized_ret_ty.clone(),
                    crate::types::FunctionSafety::from_is_unsafe(specialized_func.is_unsafe),
                ),
                expr.span.clone(),
            );
            expr.kind = HirExprKind::Call(
                Box::new(callee),
                call_args,
                Some(HirCallTarget::Instance(instance_id)),
            );
            expr.ty = specialized_ret_ty;
            Ok(())
        } else {
            Err(error(crate::mono::MonoErrorKind::NoMatchingImpl {
                trait_id,
                member_id,
            }))
        }
    }
}

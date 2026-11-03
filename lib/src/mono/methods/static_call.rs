use crate::mono::hir_types::{HirCallTarget, HirExpr, HirMethodCallTarget};
use crate::types::Type;

use crate::mono::Monomorphizer;

impl Monomorphizer {
    pub(in crate::mono) fn monomorphize_static_method_call(
        &mut self,
        owner_ty: &Type,
        selected_target: &HirMethodCallTarget,
        args: &[HirExpr],
        span: crate::lexer::Span,
    ) -> Result<(HirExpr, HirCallTarget, Type), crate::mono::MonoError> {
        let error = |kind| crate::mono::MonoError {
            kind,
            span: span.clone(),
        };
        let candidate_impls = if let Some(impl_id) = selected_target.impl_id() {
            vec![self
                .generic_impls
                .get(&impl_id)
                .cloned()
                .or_else(|| {
                    self.trait_impls
                        .values()
                        .flat_map(|impls| impls.iter())
                        .find(|imp| imp.id == impl_id)
                        .cloned()
                })
                .ok_or_else(|| error(crate::mono::MonoErrorKind::MissingImpl { impl_id }))?]
        } else {
            let trait_id = selected_target.trait_id().ok_or_else(|| {
                error(crate::mono::MonoErrorKind::InvalidBindings {
                    target: selected_target.target.clone(),
                })
            })?;
            self.trait_impls.get(&trait_id).cloned().unwrap_or_default()
        };

        let mut matching = Vec::new();
        let mut applicable_without_method = Vec::new();
        for imp in candidate_impls {
            if !Self::impl_matches_method_target(&imp, Some(selected_target)) {
                continue;
            }
            if !self.impl_receiver_pattern_matches(&imp, owner_ty) {
                if selected_target.impl_id() == Some(imp.id) {
                    return Err(error(crate::mono::MonoErrorKind::ReceiverMismatch {
                        impl_id: imp.id,
                        receiver: owner_ty.clone(),
                        pattern: Some(imp.receiver_pattern.clone()),
                    }));
                }
                continue;
            }
            if !self.impl_trait_args_match_selected_target(&imp, owner_ty, Some(selected_target)) {
                if selected_target.impl_id() == Some(imp.id) {
                    return Err(error(crate::mono::MonoErrorKind::TraitArgsMismatch {
                        impl_id: imp.id,
                        expected: imp.trait_arg_types.clone(),
                        selected: selected_target.trait_args().to_vec(),
                    }));
                }
                continue;
            }

            let Some(_method) = self.method_for_selected_target(&imp, selected_target) else {
                applicable_without_method.push(imp.id);
                continue;
            };
            matching.push(imp);
        }
        matching.sort_by_key(|imp| imp.id);
        matching.dedup_by_key(|imp| imp.id);
        if matching.len() > 1 {
            return Err(error(crate::mono::MonoErrorKind::AmbiguousImpls {
                trait_id: selected_target.trait_id().expect("trait selection"),
                member_id: selected_target.method_id().expect("method selection"),
                impls: matching.iter().map(|imp| imp.id).collect(),
            }));
        }
        let imp = matching.pop().ok_or_else(|| {
            if let (Some(impl_id), Some(method_id)) =
                (selected_target.impl_id(), selected_target.method_id())
            {
                if applicable_without_method.contains(&impl_id) {
                    error(crate::mono::MonoErrorKind::MissingMethod { impl_id, method_id })
                } else {
                    error(crate::mono::MonoErrorKind::ReceiverMismatch {
                        impl_id,
                        receiver: owner_ty.clone(),
                        pattern: self.impl_receiver_pattern_by_id(impl_id).cloned(),
                    })
                }
            } else {
                error(crate::mono::MonoErrorKind::NoMatchingImpl {
                    trait_id: selected_target.trait_id().expect("trait selection"),
                    member_id: selected_target.method_id().expect("method selection"),
                })
            }
        })?;
        let method = self
            .method_for_selected_target(&imp, selected_target)
            .ok_or_else(|| {
                error(crate::mono::MonoErrorKind::MissingMethod {
                    impl_id: imp.id,
                    method_id: selected_target.method_id().expect("method selection"),
                })
            })?;
        let type_name = imp.type_name.clone();
        let selected_impl = selected_target
            .impl_id()
            .is_some_and(|impl_id| impl_id == imp.id);

        let has_method_generics = Self::method_generic_ids_in_order(method)
            .into_iter()
            .any(|id| !imp.type_generics.iter().any(|decl| decl.id == id));
        let type_args = if has_method_generics || !imp.type_generics.is_empty() {
            let receiver_type_args = if selected_impl {
                self.extract_type_args_from_selected_target(&imp, selected_target)
            } else {
                self.extract_type_args_from_receiver_pattern(
                    &imp,
                    owner_ty,
                    selected_target.trait_args(),
                )
            };
            let receiver_type_args = receiver_type_args.ok_or_else(|| {
                error(crate::mono::MonoErrorKind::InvalidBindings {
                    target: selected_target.target.clone(),
                })
            })?;
            if receiver_type_args.len() != imp.type_generics.len() {
                return Err(error(crate::mono::MonoErrorKind::InvalidBindings {
                    target: selected_target.target.clone(),
                }));
            }
            self.combine_impl_and_method_type_args(
                method,
                args,
                &receiver_type_args,
                &imp.type_generics,
                imp.id,
                selected_target,
            )
            .ok_or_else(|| {
                error(crate::mono::MonoErrorKind::InvalidBindings {
                    target: selected_target.target.clone(),
                })
            })?
        } else {
            Vec::new()
        };

        let display_name = format!("{}::{}", type_name, method.name);
        let needs_specialization = has_method_generics || !imp.type_generics.is_empty();

        if !needs_specialization {
            let param_types = method.params.iter().map(|param| param.ty.clone()).collect();
            let ret_type = method.ret_type.clone();
            let func_ty = Type::function_with_safety(
                param_types,
                ret_type.clone(),
                crate::types::FunctionSafety::from_is_unsafe(method.is_unsafe),
            );
            let instance_key = crate::mono::InstanceKey::new(
                self.method_instance_origin(&imp, method),
                Vec::new(),
            );
            let instance_id = self
                .instances
                .get(&instance_key)
                .unwrap_or_else(|| self.register_impl_method_instance(&imp, &method.name, method));
            let callee = self.instance_callable_expr(display_name, instance_id, func_ty, span);
            return Ok((callee, HirCallTarget::Instance(instance_id), ret_type));
        }

        let type_suffix = if type_args.is_empty() {
            "none".to_string()
        } else {
            type_args
                .iter()
                .map(|id| self.type_for(*id))
                .map(|ty| Self::type_to_mono_suffix(&ty))
                .collect::<Vec<_>>()
                .join("_")
        };
        let specialized_name = format!("{}_{}_{}", type_name, type_suffix, method.name);
        let origin = self.method_instance_origin(&imp, method);
        let substitution = type_args.clone();
        let instance_key = crate::mono::InstanceKey::new(origin.clone(), substitution.clone());
        let backend_symbol = self.backend_symbol_for_origin(&origin, &substitution);

        let (instance_id, body) = if let Some(instance_id) = self.instances.get(&instance_key) {
            let body = self
                .instances
                .pre_mir_body(instance_id)
                .cloned()
                .or_else(|| {
                    self.instances
                        .record(instance_id)
                        .and_then(|record| record.declared.clone())
                })
                .ok_or_else(|| {
                    error(crate::mono::MonoErrorKind::MissingInstance {
                        origin: origin.clone(),
                    })
                })?;
            (instance_id, body)
        } else {
            let specialized_func = self.specialize_impl_method(
                method,
                &type_args,
                &imp.type_generics,
                &specialized_name,
                imp.id,
                imp.trait_id,
            );
            let instance_id =
                self.instances
                    .intern(instance_key, |id| crate::mono::InstanceRecord {
                        id,
                        origin: origin.clone(),
                        substitution: substitution.clone(),
                        symbols: crate::mono::InstanceSymbols::new(
                            display_name.clone(),
                            backend_symbol.clone(),
                        ),
                        declared: None,
                        provided_by_object: false,
                        is_specialization: true,
                    });
            self.instances
                .insert_pre_mir_body(instance_id, specialized_func.clone());
            (instance_id, specialized_func)
        };

        let param_types = body.params.iter().map(|param| param.ty.clone()).collect();
        let ret_type = body.ret_type.clone();
        let func_ty = Type::function_with_safety(
            param_types,
            ret_type.clone(),
            crate::types::FunctionSafety::from_is_unsafe(body.is_unsafe),
        );
        let callee = self.instance_callable_expr(display_name, instance_id, func_ty, span);
        Ok((callee, HirCallTarget::Instance(instance_id), ret_type))
    }
}

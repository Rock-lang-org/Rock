use std::collections::HashMap;

use super::hir_types::{
    HirBlock, HirCallTarget, HirExpr, HirExprKind, HirFunction, HirImpl, HirMethodCallTarget,
    HirParam, HirStmt,
};
use crate::ids::{CrateId, DefId, LocalDefId, TypeId};
use crate::types::{GenericParamDecl, GenericParamId, Type};

use super::Monomorphizer;

fn drop_diagnostic(
    message: String,
    origin_span: Option<&crate::lexer::Span>,
) -> crate::diagnostic::Diagnostic {
    match origin_span {
        Some(span) => crate::diagnostic::Diagnostic::new(message, span.clone())
            .with_code(crate::diagnostic::DiagnosticCode::Mono),
        None => crate::diagnostic::Diagnostic::for_internal(message)
            .with_code(crate::diagnostic::DiagnosticCode::Mono),
    }
}

impl Monomorphizer {
    pub(super) fn monomorphize_drop_for_type(
        &mut self,
        ty: &Type,
        origin_span: Option<crate::lexer::Span>,
    ) {
        let origin_span = origin_span.or_else(|| self.source_span_for_type(ty));
        self.monomorphize_drop_fields_for_type(ty);

        let Some(drop_trait_id) = self.drop_trait_id() else {
            return;
        };
        let Some(drop_method_id) = self.drop_method_id else {
            return;
        };
        let mut matching_impls = self
            .trait_impls
            .get(&drop_trait_id)
            .into_iter()
            .flat_map(|impls| impls.iter())
            .filter(|imp| imp.trait_id == Some(drop_trait_id))
            .filter(|imp| self.drop_receiver_matches(imp, ty))
            .cloned()
            .collect::<Vec<_>>();
        matching_impls.sort_by_key(|imp| imp.id);
        matching_impls.dedup_by_key(|imp| imp.id);
        let [imp] = matching_impls.as_slice() else {
            if matching_impls.len() > 1 {
                self.diagnostics.push(drop_diagnostic(
                    format!(
                        "ambiguous Drop implementation for type {}",
                        self.display_type_for_diagnostic(ty)
                    ),
                    origin_span.as_ref(),
                ));
            }
            return;
        };
        let Some(method_id) = self
            .effective_trait_methods
            .get(&(imp.id, drop_method_id))
            .copied()
        else {
            self.diagnostics.push(drop_diagnostic(
                format!("selected Drop implementation has no resolved @drop method",),
                origin_span.as_ref(),
            ));
            return;
        };
        let Some(method) = imp
            .methods
            .values()
            .find(|method| method.id == method_id)
            .cloned()
        else {
            self.diagnostics.push(drop_diagnostic(
                format!("selected Drop implementation resolves @drop to a missing method",),
                origin_span.as_ref(),
            ));
            return;
        };

        let type_args = if !imp.type_generics.is_empty() || !method.generic_params.is_empty() {
            let Some(receiver_type_args) =
                self.extract_type_args_from_receiver_pattern(imp, ty, &[])
            else {
                self.diagnostics.push(drop_diagnostic(
                    format!(
                        "selected Drop implementation could not resolve complete receiver substitution for type {}",
                        self.display_type_for_diagnostic(ty)
                    ),
                    origin_span.as_ref(),
                ));
                return;
            };
            if receiver_type_args.len() != imp.type_generics.len() {
                self.diagnostics.push(drop_diagnostic(
                    format!(
                        "selected Drop implementation could not resolve complete receiver substitution for type {}",
                        self.display_type_for_diagnostic(ty)
                    ),
                    origin_span.as_ref(),
                ));
                return;
            }
            receiver_type_args
        } else {
            Vec::new()
        };

        let origin = self.method_instance_origin(imp, &method);
        let substitution = type_args.clone();
        let instance_key = crate::mono::InstanceKey::new(origin.clone(), substitution.clone());
        let backend_symbol = self.backend_symbol_for_origin(&origin, &substitution);
        if let Some(instance_id) = self.instances.get(&instance_key) {
            let receiver = self
                .instances
                .pre_mir_body(instance_id)
                .and_then(|function| function.params.first())
                .map(|param| param.ty.clone())
                .or_else(|| {
                    self.instances
                        .record(instance_id)
                        .and_then(|record| record.declared.as_ref())
                        .and_then(|function| function.params.first())
                        .map(|param| param.ty.clone())
                })
                .unwrap_or_else(|| ty.clone());
            let receiver_ty = self.intern_type(&receiver);
            self.generated_drop_instances.insert(
                receiver_ty,
                crate::mono::GeneratedMethodInstance {
                    receiver_ty,
                    trait_id: drop_trait_id,
                    member_id: drop_method_id,
                    instance_id,
                    origin_span,
                },
            );
            return;
        }
        let in_progress_key = (imp.id, substitution.clone());
        if !self
            .drop_monomorphization_in_progress
            .insert(in_progress_key.clone())
        {
            return;
        }

        let type_suffix = if type_args.is_empty() {
            Self::receiver_mono_suffix(ty)
        } else {
            type_args
                .iter()
                .map(|id| Self::type_to_mono_suffix(&self.type_for(*id)))
                .collect::<Vec<_>>()
                .join("_")
        };
        let specialized_name = format!("{}_{}_drop", imp.type_name, type_suffix);
        let specialized_func = self.specialize_impl_method(
            &method,
            &type_args,
            &imp.type_generics,
            &specialized_name,
            imp.id,
            imp.trait_id,
        );
        let concrete_receiver = specialized_func
            .params
            .first()
            .map(|param| param.ty.clone())
            .unwrap_or_else(|| ty.clone());
        let receiver_ty = self.intern_type(&concrete_receiver);
        self.drop_monomorphization_in_progress
            .remove(&in_progress_key);

        let instance_id = self
            .instances
            .intern(instance_key, |id| crate::mono::InstanceRecord {
                id,
                origin: origin.clone(),
                substitution: substitution.clone(),
                symbols: crate::mono::InstanceSymbols::new(
                    format!("{}::drop", imp.type_name),
                    backend_symbol.clone(),
                ),
                declared: None,
                provided_by_object: false,
                is_specialization: true,
            });
        self.instances
            .insert_pre_mir_body(instance_id, specialized_func);
        self.generated_drop_instances.insert(
            receiver_ty,
            crate::mono::GeneratedMethodInstance {
                receiver_ty,
                trait_id: drop_trait_id,
                member_id: drop_method_id,
                instance_id,
                origin_span,
            },
        );
    }

    pub(super) fn drop_trait_id(&self) -> Option<DefId> {
        self.drop_trait_id
    }

    fn impl_matches_method_target(imp: &HirImpl, target: Option<&HirMethodCallTarget>) -> bool {
        crate::selection::target_matches_impl(imp, target)
    }

    fn impl_trait_args_match_selected_target(
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
        imp.trait_arg_types
            .iter()
            .zip(target.trait_args())
            .all(|(expected, actual)| {
                crate::selection::type_pattern_matches(expected, actual, &mut substitution)
            })
    }

    fn method_generic_ids_in_order(method: &HirFunction) -> Vec<GenericParamId> {
        method.generic_params.iter().map(|param| param.id).collect()
    }

    fn combine_impl_and_method_type_args(
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

    fn rewrite_to_existing_instance_call(
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

    /// Monomorphize a standalone generic impl method call (e.g. Vec T -> Vec_I64_push)
    pub(super) fn monomorphize_standalone_method_call(
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

    pub(super) fn monomorphize_static_method_call(
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

    /// Monomorphize a trait method call by finding the impl and specializing it
    pub(super) fn monomorphize_trait_method_call(
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
            let backend_symbol = self.backend_symbol_for_origin(&origin, &substitution);
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

    /// Extract type arguments from a receiver type based on impl type parameters
    fn extract_type_args_from_selected_target(
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

    fn extract_type_args_from_receiver_pattern(
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

    /// Specialize an impl method by substituting type parameters
    fn specialize_impl_method(
        &mut self,
        method: &HirFunction,
        type_args: &[TypeId],
        type_generics: &[GenericParamDecl],
        specialized_name: &str,
        impl_id: DefId,
        _trait_id: Option<DefId>,
    ) -> HirFunction {
        let mut substitution = HashMap::new();
        let impl_type_arg_count = type_generics.len().min(type_args.len());
        for (i, decl) in type_generics.iter().enumerate() {
            if i < type_args.len() {
                substitution.insert(decl.id, type_args[i]);
            }
        }

        let method_type_arg_count = type_args.len().saturating_sub(impl_type_arg_count);
        let method_generic_candidates: Vec<_> = Self::method_generic_ids_in_order(method)
            .into_iter()
            .filter(|id| !type_generics.iter().any(|decl| decl.id == *id))
            .collect();
        let method_generic_ids = method_generic_candidates[method_generic_candidates
            .len()
            .saturating_sub(method_type_arg_count)..]
            .to_vec();

        for (i, generic_id) in method_generic_ids.iter().enumerate() {
            if impl_type_arg_count + i < type_args.len() {
                substitution.insert(*generic_id, type_args[impl_type_arg_count + i]);
            }
        }

        let specialized_impl_type = if let Some(self_param) = method.params.first() {
            self.substitute_type_with_map(&self_param.ty, &substitution)
        } else {
            match self.impl_receiver_pattern_by_id(impl_id).cloned() {
                Some(crate::hir::HirImplReceiverPattern::Exact(ty)) => {
                    self.substitute_type_with_map(&ty, &substitution)
                }
                Some(crate::hir::HirImplReceiverPattern::Constructor(ty)) => {
                    self.substitute_type_with_map(&ty, &substitution)
                }
                Some(crate::hir::HirImplReceiverPattern::SliceFamily { element }) => Type::Slice(
                    Box::new(self.substitute_type_with_map(&element, &substitution)),
                ),
                None => panic!(
                    "accepted impl {:?} is missing canonical receiver authority",
                    impl_id
                ),
            }
        };

        let params: Vec<HirParam> = method
            .params
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let ty = if i == 0 && p.name == "self" && method.is_method {
                    specialized_impl_type.clone()
                } else {
                    self.substitute_type_with_map(&p.ty, &substitution)
                };
                HirParam {
                    name: p.name.clone(),
                    local_id: p.local_id,
                    ty,
                    mutable: p.mutable,
                    is_ref: p.is_ref,
                }
            })
            .collect();

        let ret_type = self.substitute_type_with_map(&method.ret_type, &substitution);
        let mut body = self.substitute_block(&method.body, &substitution);

        self.update_self_types(&mut body, &specialized_impl_type);

        let old_type_args = self.current_type_args.clone();
        let old_var_types = self.var_types.clone();
        self.current_type_args = type_args.to_vec();
        self.var_types.clear();
        let mut params_for_body = params.clone();
        self.process_params(Some(method.id), &mut params_for_body);
        self.process_block(&mut body);
        self.current_type_args = old_type_args;
        self.var_types = old_var_types;

        HirFunction {
            id: DefId::new(CrateId(0), LocalDefId(0)),
            name: specialized_name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: crate::hir::HirGenericBounds::new(),
            params,
            ret_type,
            body,
            is_curried: method.is_curried,
            is_method: method.is_method,
            self_receiver: method.self_receiver,
            is_unsafe: method.is_unsafe,
        }
    }

    /// Update all `self` variable expressions in a block to have the correct type
    fn update_self_types(&self, block: &mut HirBlock, self_type: &Type) {
        for stmt in &mut block.stmts {
            self.update_self_types_stmt(stmt, self_type);
        }
    }

    fn update_self_types_stmt(&self, stmt: &mut HirStmt, self_type: &Type) {
        match stmt {
            HirStmt::Let { value, .. } => {
                self.update_self_types_expr(value, self_type);
            }
            HirStmt::Expr(expr) => {
                self.update_self_types_expr(expr, self_type);
            }
            HirStmt::Return(expr) => {
                if let Some(e) = expr {
                    self.update_self_types_expr(e, self_type);
                }
            }
            HirStmt::Break(expr) => {
                if let Some(e) = expr {
                    self.update_self_types_expr(e, self_type);
                }
            }
            HirStmt::Continue => {}
        }
    }

    fn update_self_types_expr(&self, expr: &mut HirExpr, self_type: &Type) {
        if let HirExprKind::Var(name) = &expr.kind {
            if name == "self" {
                expr.ty = self_type.clone();
            }
        }

        match &mut expr.kind {
            HirExprKind::Var(_)
            | HirExprKind::ResolvedVar(_)
            | HirExprKind::IntLiteral(_)
            | HirExprKind::FloatLiteral(_)
            | HirExprKind::BoolLiteral(_)
            | HirExprKind::StringLiteral(_)
            | HirExprKind::CharLiteral(_)
            | HirExprKind::Unit => {}
            HirExprKind::BinOp(_, lhs, rhs) => {
                self.update_self_types_expr(lhs, self_type);
                self.update_self_types_expr(rhs, self_type);
            }
            HirExprKind::UnaryOp(_, inner) => {
                self.update_self_types_expr(inner, self_type);
            }
            HirExprKind::Call(func, args, _) => {
                self.update_self_types_expr(func, self_type);
                for arg in args {
                    self.update_self_types_expr(arg, self_type);
                }
            }
            HirExprKind::MethodCall(recv, _, args, _, _) => {
                self.update_self_types_expr(recv, self_type);
                for arg in args {
                    self.update_self_types_expr(arg, self_type);
                }
            }
            HirExprKind::Try { expr, .. } => {
                self.update_self_types_expr(expr, self_type);
            }
            HirExprKind::FieldAccess(inner, _, _) | HirExprKind::TupleIndex(inner, _) => {
                self.update_self_types_expr(inner, self_type);
            }
            HirExprKind::ArrayLiteral(elems) | HirExprKind::TupleLiteral(elems) => {
                for e in elems {
                    self.update_self_types_expr(e, self_type);
                }
            }
            HirExprKind::ArrayRepeat(value, _) => {
                self.update_self_types_expr(value, self_type);
            }
            HirExprKind::StructLiteral(_, _, fields) => {
                for field in fields {
                    self.update_self_types_expr(&mut field.value, self_type);
                }
            }
            HirExprKind::EnumVariant(_, _, args, _) => {
                for e in args {
                    self.update_self_types_expr(e, self_type);
                }
            }
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.update_self_types_expr(condition, self_type);
                self.update_self_types(then_branch, self_type);
                if let Some(e) = else_branch {
                    self.update_self_types(e, self_type);
                }
            }
            HirExprKind::Match { scrutinee, arms } => {
                self.update_self_types_expr(scrutinee, self_type);
                for arm in arms {
                    if let Some(g) = &mut arm.guard {
                        self.update_self_types_expr(g, self_type);
                    }
                    self.update_self_types(&mut arm.body, self_type);
                }
            }
            HirExprKind::While { condition, body } => {
                self.update_self_types_expr(condition, self_type);
                self.update_self_types(body, self_type);
            }
            HirExprKind::For { iter, body, .. } => {
                self.update_self_types_expr(iter, self_type);
                self.update_self_types(body, self_type);
            }
            HirExprKind::Loop(body) | HirExprKind::Block(body) | HirExprKind::UnsafeBlock(body) => {
                self.update_self_types(body, self_type);
            }
            HirExprKind::Lambda { body, .. } => {
                self.update_self_types(body, self_type);
            }
            HirExprKind::Ref(_, inner)
            | HirExprKind::Deref(inner)
            | HirExprKind::Cast(inner, _) => {
                self.update_self_types_expr(inner, self_type);
            }
            HirExprKind::Assign(lhs, rhs) => {
                self.update_self_types_expr(lhs.as_mut(), self_type);
                self.update_self_types_expr(rhs.as_mut(), self_type);
            }
            HirExprKind::Intrinsic { args, .. } => {
                for arg in args {
                    self.update_self_types_expr(arg, self_type);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::hir::{
        HirImplOwner, HirImplReceiverPattern, HirMethodCallTarget, HirParam, HirVarTarget,
    };
    use crate::ids::{CrateId, DefId, LocalDefId};
    use crate::lexer::Span;
    use crate::mono::hir_types::{HirBlock, HirExpr, HirExprKind, HirFunction, HirImpl, HirStmt};
    use crate::types::ReceiverMode;
    use crate::types::Type;
    use crate::types::{GenericParamDecl, GenericParamId};

    use crate::mono::{InstanceImplOwner, InstanceOrigin};

    use super::Monomorphizer;

    fn empty_body(ty: Type) -> HirBlock {
        HirBlock { stmts: vec![], ty }
    }

    fn enum_ty(type_name: &str, args: Vec<Type>) -> Type {
        let local = match type_name {
            "Option" | "stdlib::option::Option" => 10,
            "Result" => 11,
            _ => 12,
        };
        Type::Enum {
            id: DefId::new(CrateId(0), LocalDefId(local)),
            args,
        }
    }

    fn def_id(index: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(index))
    }

    fn type_generic(owner: DefId, name: &str) -> Vec<GenericParamDecl> {
        vec![GenericParamDecl::type_param(
            GenericParamId { owner, index: 0 },
            name,
        )]
    }

    #[test]
    fn drop_materialization_reports_incomplete_receiver_substitution() {
        let mut mono = Monomorphizer::new();
        let drop_trait_id = def_id(40);
        let drop_member_id = def_id(41);
        let impl_id = def_id(42);
        let method_id = def_id(43);
        let owner_id = def_id(44);
        let method = HirFunction {
            id: method_id,
            name: "drop".to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: Vec::new(),
            ret_type: Type::Unit,
            body: empty_body(Type::Unit),
            is_curried: false,
            is_method: true,
            self_receiver: Some(ReceiverMode::Move),
            is_unsafe: false,
        };
        let imp = HirImpl {
            id: impl_id,
            owner: crate::hir::HirImplOwner::Named("Wrapper".to_string()),
            type_name: "Wrapper".to_string(),
            type_generics: type_generic(impl_id, "T"),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: owner_id,
                args: Vec::new(),
            }),
            trait_name: Some("Drop".to_string()),
            trait_id: Some(drop_trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("drop".to_string(), method)]),
        };
        mono.drop_trait_id = Some(drop_trait_id);
        mono.drop_method_id = Some(drop_member_id);
        mono.effective_trait_methods
            .insert((impl_id, drop_member_id), method_id);
        mono.trait_impls.insert(drop_trait_id, vec![imp]);

        mono.monomorphize_drop_for_type(
            &Type::Struct {
                id: owner_id,
                args: Vec::new(),
            },
            None,
        );

        assert!(mono.diagnostics.0.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("could not resolve complete receiver substitution")
        }));
    }

    #[test]
    fn drop_materialization_rejects_differently_specific_matching_impls() {
        let mut mono = Monomorphizer::new();
        let drop_trait_id = def_id(45);
        let drop_member_id = def_id(46);
        let generic_impl_id = def_id(47);
        let generic_method_id = def_id(48);
        let concrete_impl_id = def_id(49);
        let concrete_method_id = def_id(50);
        let owner_id = def_id(51);
        let drop_method = |id| HirFunction {
            id,
            name: "drop".to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: Vec::new(),
            ret_type: Type::Unit,
            body: empty_body(Type::Unit),
            is_curried: false,
            is_method: true,
            self_receiver: Some(ReceiverMode::Move),
            is_unsafe: false,
        };
        let generic_impl = HirImpl {
            id: generic_impl_id,
            owner: crate::hir::HirImplOwner::Named("T".to_string()),
            type_name: "T".to_string(),
            type_generics: type_generic(generic_impl_id, "T"),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Generic(GenericParamId {
                owner: generic_impl_id,
                index: 0,
            })),
            trait_name: Some("Drop".to_string()),
            trait_id: Some(drop_trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("drop".to_string(), drop_method(generic_method_id))]),
        };
        let concrete_impl = HirImpl {
            id: concrete_impl_id,
            owner: crate::hir::HirImplOwner::Named("Wrapper".to_string()),
            type_name: "Wrapper".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: owner_id,
                args: Vec::new(),
            }),
            trait_name: Some("Drop".to_string()),
            trait_id: Some(drop_trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([("drop".to_string(), drop_method(concrete_method_id))]),
        };
        mono.drop_trait_id = Some(drop_trait_id);
        mono.drop_method_id = Some(drop_member_id);
        mono.effective_trait_methods
            .insert((generic_impl_id, drop_member_id), generic_method_id);
        mono.effective_trait_methods
            .insert((concrete_impl_id, drop_member_id), concrete_method_id);
        mono.trait_impls
            .insert(drop_trait_id, vec![generic_impl, concrete_impl]);

        mono.monomorphize_drop_for_type(
            &Type::Struct {
                id: owner_id,
                args: Vec::new(),
            },
            None,
        );

        assert!(mono.diagnostics.0.iter().any(|diagnostic| {
            diagnostic.message.contains("ambiguous Drop implementation")
                && !diagnostic.message.contains("DefId")
                && diagnostic.code == Some(crate::diagnostic::DiagnosticCode::Mono)
        }));
        assert_eq!(mono.instances.len(), 0);
    }

    fn overlapping_trait_impl(
        impl_id: DefId,
        method_id: DefId,
        trait_id: DefId,
        receiver_pattern: HirImplReceiverPattern,
        method_name: &str,
        self_receiver: Option<ReceiverMode>,
    ) -> HirImpl {
        let receiver_ty = match &receiver_pattern {
            HirImplReceiverPattern::Exact(ty) | HirImplReceiverPattern::Constructor(ty) => {
                ty.clone()
            }
            HirImplReceiverPattern::SliceFamily { element } => {
                Type::Slice(Box::new(element.clone()))
            }
        };
        let params = self_receiver
            .map(|_| {
                vec![HirParam {
                    name: "self".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: receiver_ty,
                    mutable: false,
                    is_ref: false,
                }]
            })
            .unwrap_or_default();
        let method = HirFunction {
            id: method_id,
            name: method_name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params,
            ret_type: Type::Unit,
            body: empty_body(Type::Unit),
            is_curried: false,
            is_method: self_receiver.is_some(),
            self_receiver,
            is_unsafe: false,
        };
        HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Owner".to_string()),
            type_name: "Owner".to_string(),
            type_generics: if let HirImplReceiverPattern::Exact(Type::Generic(id)) =
                &receiver_pattern
            {
                vec![GenericParamDecl::type_param(*id, "T")]
            } else {
                Vec::new()
            },
            receiver_pattern,
            trait_name: Some("Trait".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([(method_name.to_string(), method)]),
        }
    }

    fn install_overlapping_trait_impls(
        mono: &mut Monomorphizer,
        trait_id: DefId,
        member_id: DefId,
        generic_impl_id: DefId,
        generic_method_id: DefId,
        concrete_impl_id: DefId,
        concrete_method_id: DefId,
        owner_ty: &Type,
        method_name: &str,
        self_receiver: Option<ReceiverMode>,
    ) {
        let generic_impl = overlapping_trait_impl(
            generic_impl_id,
            generic_method_id,
            trait_id,
            HirImplReceiverPattern::Exact(Type::Generic(GenericParamId {
                owner: generic_impl_id,
                index: 0,
            })),
            method_name,
            self_receiver,
        );
        let concrete_impl = overlapping_trait_impl(
            concrete_impl_id,
            concrete_method_id,
            trait_id,
            HirImplReceiverPattern::Exact(owner_ty.clone()),
            method_name,
            self_receiver,
        );
        mono.effective_trait_methods
            .insert((generic_impl_id, member_id), generic_method_id);
        mono.effective_trait_methods
            .insert((concrete_impl_id, member_id), concrete_method_id);
        mono.trait_impls
            .insert(trait_id, vec![generic_impl, concrete_impl]);
    }

    fn assert_ambiguous_impl_error(
        error: crate::mono::MonoError,
        trait_id: DefId,
        member_id: DefId,
        expected_impls: &[DefId],
    ) {
        let crate::mono::MonoErrorKind::AmbiguousImpls {
            trait_id: actual_trait,
            member_id: actual_member,
            impls,
        } = error.kind
        else {
            panic!("expected ambiguous impl error, got {:?}", error.kind);
        };
        assert_eq!(actual_trait, trait_id);
        assert_eq!(actual_member, member_id);
        assert_eq!(impls, expected_impls);
    }

    #[test]
    fn static_method_materialization_rejects_differently_specific_matching_impls() {
        let mut mono = Monomorphizer::new();
        let trait_id = def_id(52);
        let member_id = def_id(53);
        let generic_impl_id = def_id(54);
        let generic_method_id = def_id(55);
        let concrete_impl_id = def_id(56);
        let concrete_method_id = def_id(57);
        let owner_ty = Type::Struct {
            id: def_id(58),
            args: Vec::new(),
        };
        install_overlapping_trait_impls(
            &mut mono,
            trait_id,
            member_id,
            generic_impl_id,
            generic_method_id,
            concrete_impl_id,
            concrete_method_id,
            &owner_ty,
            "make",
            None,
        );
        let target = HirMethodCallTarget::trait_method(
            trait_id,
            member_id,
            Vec::new(),
            crate::hir::HirTraitDispatchKind::TraitBound,
        );

        let error = mono
            .monomorphize_static_method_call(&owner_ty, &target, &[], Span::test())
            .expect_err("overlapping static impls must be ambiguous");

        assert_ambiguous_impl_error(
            error,
            trait_id,
            member_id,
            &[generic_impl_id, concrete_impl_id],
        );
        assert_eq!(mono.instances.len(), 0);
    }

    #[test]
    fn constructor_trait_selection_mono_resolves_deferred_authority_to_instance() {
        let mut mono = Monomorphizer::new();
        let trait_id = def_id(580);
        let member_id = def_id(581);
        let impl_id = def_id(582);
        let method_id = def_id(583);
        let owner_ty = Type::Constructor {
            id: def_id(584),
            flavor: crate::types::NominalTypeKind::Enum,
        };
        let imp = overlapping_trait_impl(
            impl_id,
            method_id,
            trait_id,
            HirImplReceiverPattern::Constructor(owner_ty.clone()),
            "pure",
            None,
        );
        mono.effective_trait_methods
            .insert((impl_id, member_id), method_id);
        mono.trait_impls.insert(trait_id, vec![imp]);
        let target = HirMethodCallTarget::trait_method(
            trait_id,
            member_id,
            Vec::new(),
            crate::hir::HirTraitDispatchKind::TraitBound,
        );

        let (_, call_target, _) = mono
            .monomorphize_static_method_call(&owner_ty, &target, &[], Span::test())
            .expect("deferred constructor authority should resolve");

        assert!(matches!(
            call_target,
            crate::hir::HirCallTarget::Instance(_)
        ));
        assert_eq!(mono.instances.len(), 1);
    }

    #[test]
    fn trait_method_materialization_rejects_differently_specific_matching_impls() {
        let mut mono = Monomorphizer::new();
        let trait_id = def_id(59);
        let member_id = def_id(60);
        let generic_impl_id = def_id(61);
        let generic_method_id = def_id(62);
        let concrete_impl_id = def_id(63);
        let concrete_method_id = def_id(64);
        let owner_ty = Type::Struct {
            id: def_id(65),
            args: Vec::new(),
        };
        install_overlapping_trait_impls(
            &mut mono,
            trait_id,
            member_id,
            generic_impl_id,
            generic_method_id,
            concrete_impl_id,
            concrete_method_id,
            &owner_ty,
            "value",
            Some(ReceiverMode::Move),
        );
        let target = HirMethodCallTarget::trait_method(
            trait_id,
            member_id,
            Vec::new(),
            crate::hir::HirTraitDispatchKind::TraitBound,
        );
        let receiver = HirExpr {
            kind: HirExprKind::Var("owner".to_string()),
            ty: owner_ty.clone(),
            span: Span::test(),
        };
        let mut expr = HirExpr {
            kind: HirExprKind::MethodCall(
                Box::new(receiver.clone()),
                "value".to_string(),
                Vec::new(),
                Some(ReceiverMode::Move),
                target,
            ),
            ty: Type::Unit,
            span: Span::test(),
        };

        let error = mono
            .monomorphize_trait_method_call("value", &[receiver], &mut expr)
            .expect_err("overlapping trait impls must be ambiguous");

        assert_ambiguous_impl_error(
            error,
            trait_id,
            member_id,
            &[generic_impl_id, concrete_impl_id],
        );
        assert_eq!(mono.instances.len(), 0);
        assert!(matches!(expr.kind, HirExprKind::MethodCall(..)));
    }

    #[test]
    fn selected_method_type_args_use_authority_sidecar_order() {
        let mut mono = Monomorphizer::new();
        let method_id = def_id(30);
        let t_id = GenericParamId {
            owner: method_id,
            index: 0,
        };
        let u_id = GenericParamId {
            owner: method_id,
            index: 1,
        };
        let method = HirFunction {
            id: method_id,
            name: "pair".to_string(),
            generic_params: vec![
                GenericParamDecl::type_param(u_id, "U"),
                GenericParamDecl::type_param(t_id, "T"),
            ],
            generic_bounds: HashMap::new().into(),
            params: vec![
                HirParam {
                    name: "left".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::Generic(t_id),
                    mutable: false,
                    is_ref: false,
                },
                HirParam {
                    name: "right".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::Generic(u_id),
                    mutable: false,
                    is_ref: false,
                },
            ],
            ret_type: Type::Unit,
            body: empty_body(Type::Unit),
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };
        let impl_id = def_id(31);
        let mut target = HirMethodCallTarget::impl_method(impl_id, method_id, None);
        target.method_substitution = vec![
            crate::hir::HirTypeBinding {
                param: t_id,
                ty: Type::I32,
            },
            crate::hir::HirTypeBinding {
                param: u_id,
                ty: Type::Bool,
            },
        ];
        let type_args = mono
            .combine_impl_and_method_type_args(&method, &[], &[], &[], impl_id, &target)
            .expect("complete selected authority");

        assert_eq!(
            type_args
                .iter()
                .map(|id| mono.type_for(*id))
                .collect::<Vec<_>>(),
            vec![Type::Bool, Type::I32]
        );

        target.method_substitution.pop();
        assert!(mono
            .combine_impl_and_method_type_args(&method, &[], &[], &[], impl_id, &target)
            .is_none());
    }

    #[test]
    fn trait_method_bindings_map_member_owned_generics_to_impl_method_generics() {
        let mut mono = Monomorphizer::new();
        let trait_id = def_id(32);
        let trait_member_id = def_id(33);
        let impl_id = def_id(34);
        let impl_method_id = def_id(35);
        let first_impl_param = GenericParamId {
            owner: impl_method_id,
            index: 0,
        };
        let second_impl_param = GenericParamId {
            owner: impl_method_id,
            index: 1,
        };
        let method = HirFunction {
            id: impl_method_id,
            name: "convert".to_string(),
            generic_params: vec![
                GenericParamDecl::type_param(first_impl_param, "T"),
                GenericParamDecl::type_param(second_impl_param, "U"),
            ],
            generic_bounds: HashMap::new().into(),
            params: vec![],
            ret_type: Type::Unit,
            body: empty_body(Type::Unit),
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };
        let mut target = HirMethodCallTarget::trait_method(
            trait_id,
            trait_member_id,
            vec![],
            crate::hir::HirTraitDispatchKind::TraitBound,
        );
        target.method_substitution = vec![
            crate::hir::HirTypeBinding {
                param: GenericParamId {
                    owner: trait_member_id,
                    index: 0,
                },
                ty: Type::I32,
            },
            crate::hir::HirTypeBinding {
                param: GenericParamId {
                    owner: trait_member_id,
                    index: 1,
                },
                ty: Type::Bool,
            },
        ];

        let type_args = mono
            .combine_impl_and_method_type_args(&method, &[], &[], &[], impl_id, &target)
            .expect("member-owned trait bindings");

        assert_eq!(
            type_args
                .iter()
                .map(|id| mono.type_for(*id))
                .collect::<Vec<_>>(),
            vec![Type::I32, Type::Bool]
        );
    }

    fn option_map_method(impl_id: DefId, method_id: DefId) -> HirFunction {
        let impl_generic = GenericParamId {
            owner: impl_id,
            index: 0,
        };
        let method_generic = GenericParamId {
            owner: method_id,
            index: 0,
        };
        HirFunction {
            id: method_id,
            name: "map".to_string(),
            generic_params: vec![
                GenericParamDecl::type_param(impl_generic, "T"),
                GenericParamDecl::type_param(method_generic, "U"),
            ],
            generic_bounds: HashMap::new().into(),
            params: vec![
                HirParam {
                    name: "self".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: enum_ty("Option", vec![Type::Generic(impl_generic)]),
                    mutable: false,
                    is_ref: false,
                },
                HirParam {
                    name: "f".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: Type::function(
                        vec![Type::Generic(impl_generic)],
                        Type::Generic(method_generic),
                    ),
                    mutable: false,
                    is_ref: false,
                },
            ],
            ret_type: enum_ty("Option", vec![Type::Generic(method_generic)]),
            body: empty_body(Type::Unit),
            is_curried: false,
            is_method: true,
            self_receiver: Some(ReceiverMode::Move),
            is_unsafe: false,
        }
    }

    fn map_method_for_type(type_name: &str, impl_id: DefId, method_id: DefId) -> HirFunction {
        let mut method = option_map_method(impl_id, method_id);
        method.params[0].ty = enum_ty(
            type_name,
            vec![Type::Generic(GenericParamId {
                owner: impl_id,
                index: 0,
            })],
        );
        method.ret_type = enum_ty(
            type_name,
            vec![Type::Generic(GenericParamId {
                owner: method_id,
                index: 0,
            })],
        );
        method
    }

    fn map_expr_for_type(type_name: &str) -> (HirExpr, HirExpr, HirExpr) {
        let recv = HirExpr {
            kind: HirExprKind::Var("value".to_string()),
            ty: enum_ty(type_name, vec![Type::I64]),
            span: Span::test(),
        };
        let func_arg = HirExpr {
            kind: HirExprKind::Var("inc".to_string()),
            ty: Type::function(vec![Type::I64], Type::I64),
            span: Span::test(),
        };
        let expr = HirExpr {
            kind: HirExprKind::MethodCall(
                Box::new(recv.clone()),
                "map".to_string(),
                vec![func_arg.clone()],
                Some(ReceiverMode::Move),
                HirMethodCallTarget::trait_method(
                    def_id(999),
                    def_id(998),
                    Vec::new(),
                    crate::hir::HirTraitDispatchKind::TraitBound,
                ),
            ),
            ty: enum_ty(type_name, vec![Type::I64]),
            span: Span::test(),
        };

        (recv, func_arg, expr)
    }

    fn generic_map_impl(type_name: &str, impl_id: DefId, method_id: DefId) -> HirImpl {
        HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named(type_name.to_string()),
            type_name: type_name.to_string(),
            type_generics: type_generic(impl_id, "T"),
            receiver_pattern: vec![Type::Generic(GenericParamId {
                owner: impl_id,
                index: 0,
            })]
            .into(),
            trait_name: None,
            trait_id: None,
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "map".to_string(),
                map_method_for_type(type_name, impl_id, method_id),
            )]),
        }
    }

    fn seed_generic_enum_receiver_pattern(
        mono: &mut Monomorphizer,
        impl_id: DefId,
        type_name: &str,
    ) {
        mono.set_impl_receiver_pattern_for_test(
            impl_id,
            crate::hir::HirImplReceiverPattern::Exact(enum_ty(
                type_name,
                vec![Type::Generic(GenericParamId {
                    owner: impl_id,
                    index: 0,
                })],
            )),
        );
    }

    #[test]
    fn monomorphize_standalone_method_call_without_selected_target_does_not_specialize_by_name() {
        let mut mono = Monomorphizer::new();
        let impl_id = def_id(90);
        let method_id = def_id(91);
        mono.generic_impls
            .insert(impl_id, generic_map_impl("Option", impl_id, method_id));
        let (recv, func_arg, mut expr) = map_expr_for_type("Option");

        let error = mono
            .monomorphize_standalone_method_call("map", &[recv, func_arg], &mut expr)
            .expect_err("a targetless standalone method must be rejected");
        assert!(matches!(
            error.kind,
            crate::mono::MonoErrorKind::InvalidBindings { .. }
        ));

        assert!(
            matches!(expr.kind, HirExprKind::MethodCall(..)),
            "targetless standalone methods must not specialize by type and method name"
        );
        assert!(mono.instances.records().next().is_none());
    }

    #[test]
    fn monomorphize_trait_method_call_without_selected_target_does_not_specialize_by_name() {
        let mut mono = Monomorphizer::new();
        let impl_id = def_id(92);
        let trait_id = def_id(93);
        let method_id = def_id(94);
        let method = option_println_method(impl_id, method_id);
        mono.trait_impls.insert(
            trait_id,
            vec![HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Option".to_string()),
                type_name: "Option".to_string(),
                type_generics: type_generic(impl_id, "T"),
                receiver_pattern: vec![Type::Generic(GenericParamId {
                    owner: impl_id,
                    index: 0,
                })]
                .into(),
                trait_name: Some("Show".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("println".to_string(), method)]),
            }],
        );
        let recv = HirExpr {
            kind: HirExprKind::Var("some".to_string()),
            ty: enum_ty("Option", vec![Type::I64]),
            span: Span::test(),
        };
        let mut expr = HirExpr {
            kind: HirExprKind::MethodCall(
                Box::new(recv.clone()),
                "println".to_string(),
                vec![],
                Some(ReceiverMode::Move),
                HirMethodCallTarget::trait_method(
                    def_id(999),
                    def_id(998),
                    Vec::new(),
                    crate::hir::HirTraitDispatchKind::TraitBound,
                ),
            ),
            ty: Type::I32,
            span: Span::test(),
        };

        let _ = mono.monomorphize_trait_method_call("println", &[recv], &mut expr);

        assert!(
            matches!(expr.kind, HirExprKind::MethodCall(..)),
            "targetless trait methods must not specialize by receiver and method name"
        );
        assert!(mono.instances.records().next().is_none());
    }

    #[test]
    fn monomorphize_trait_method_call_rejects_target_without_trait_identity() {
        let mut mono = Monomorphizer::new();
        let impl_id = def_id(96);
        let method_id = def_id(97);
        let method = option_println_method(impl_id, method_id);
        mono.trait_impls.insert(
            def_id(98),
            vec![HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Option".to_string()),
                type_name: "Option".to_string(),
                type_generics: type_generic(impl_id, "T"),
                receiver_pattern: vec![Type::Generic(GenericParamId {
                    owner: impl_id,
                    index: 0,
                })]
                .into(),
                trait_name: Some("Show".to_string()),
                trait_id: Some(def_id(98)),
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("println".to_string(), method)]),
            }],
        );
        let recv = qualified_option_receiver();
        let mut expr = HirExpr {
            kind: HirExprKind::MethodCall(
                Box::new(recv.clone()),
                "println".to_string(),
                vec![],
                Some(ReceiverMode::Move),
                HirMethodCallTarget::impl_method(impl_id, method_id, None),
            ),
            ty: Type::I32,
            span: Span::test(),
        };

        let error = mono
            .monomorphize_trait_method_call("println", &[recv], &mut expr)
            .expect_err("an impl target without trait identity must be rejected");

        assert!(matches!(
            error.kind,
            crate::mono::MonoErrorKind::InvalidBindings { .. }
        ));

        assert!(matches!(expr.kind, HirExprKind::MethodCall(..)));
        assert!(mono.instances.records().next().is_none());
    }

    #[test]
    fn monomorphize_selected_index_and_index_mut_methods() {
        let mut mono = Monomorphizer::new();
        let cell_id = def_id(110);
        let index_trait_id = def_id(111);
        let index_mut_trait_id = def_id(112);
        let index_impl_id = def_id(113);
        let index_mut_impl_id = def_id(114);
        let index_member_id = def_id(115);
        let index_mut_member_id = def_id(116);
        let index_method_id = def_id(117);
        let index_mut_method_id = def_id(118);
        let cell_ty = Type::Struct {
            id: cell_id,
            args: Vec::new(),
        };
        let method = |id: DefId, name: &str, mutable: bool| HirFunction {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: vec![
                HirParam {
                    name: "self".to_string(),
                    local_id: crate::ids::HirLocalId(0),
                    ty: cell_ty.clone(),
                    mutable: false,
                    is_ref: false,
                },
                HirParam {
                    name: "key".to_string(),
                    local_id: crate::ids::HirLocalId(1),
                    ty: Type::I64,
                    mutable: false,
                    is_ref: false,
                },
            ],
            ret_type: Type::Reference {
                mutable,
                inner: Box::new(Type::I64),
            },
            body: empty_body(Type::Reference {
                mutable,
                inner: Box::new(Type::I64),
            }),
            is_curried: false,
            is_method: true,
            self_receiver: Some(ReceiverMode::Move),
            is_unsafe: false,
        };
        let indexed_impl =
            |impl_id: DefId, trait_id: DefId, type_name: &str, method: HirFunction| HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named(type_name.to_string()),
                type_name: type_name.to_string(),
                type_generics: Vec::new(),
                receiver_pattern: HirImplReceiverPattern::Exact(cell_ty.clone()),
                trait_name: Some(type_name.to_string()),
                trait_id: Some(trait_id),
                trait_generics: Vec::new(),
                trait_arg_types: vec![Type::I64],
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::from([(method.name.clone(), method)]),
            };

        mono.effective_trait_methods
            .insert((index_impl_id, index_member_id), index_method_id);
        mono.effective_trait_methods.insert(
            (index_mut_impl_id, index_mut_member_id),
            index_mut_method_id,
        );
        mono.trait_impls.insert(
            index_trait_id,
            vec![indexed_impl(
                index_impl_id,
                index_trait_id,
                "Index",
                method(index_method_id, "read_at", false),
            )],
        );
        mono.trait_impls.insert(
            index_mut_trait_id,
            vec![indexed_impl(
                index_mut_impl_id,
                index_mut_trait_id,
                "IndexMut",
                method(index_mut_method_id, "write_at", true),
            )],
        );

        let receiver = HirExpr {
            kind: HirExprKind::Var("cell".to_string()),
            ty: cell_ty,
            span: Span::test(),
        };
        let key = HirExpr {
            kind: HirExprKind::IntLiteral(0),
            ty: Type::I64,
            span: Span::test(),
        };
        let mut index_expr = HirExpr {
            kind: HirExprKind::MethodCall(
                Box::new(receiver.clone()),
                "read_at".to_string(),
                vec![key.clone()],
                Some(ReceiverMode::Move),
                HirMethodCallTarget::trait_method(
                    index_trait_id,
                    index_member_id,
                    vec![Type::I64],
                    crate::hir::HirTraitDispatchKind::TraitBound,
                ),
            ),
            ty: Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            },
            span: Span::test(),
        };
        let mut index_mut_expr = HirExpr {
            kind: HirExprKind::MethodCall(
                Box::new(receiver.clone()),
                "write_at".to_string(),
                vec![key.clone()],
                Some(ReceiverMode::Move),
                HirMethodCallTarget::trait_method(
                    index_mut_trait_id,
                    index_mut_member_id,
                    vec![Type::I64],
                    crate::hir::HirTraitDispatchKind::TraitBound,
                ),
            ),
            ty: Type::Reference {
                mutable: true,
                inner: Box::new(Type::I64),
            },
            span: Span::test(),
        };

        mono.monomorphize_trait_method_call(
            "read_at",
            &[receiver.clone(), key.clone()],
            &mut index_expr,
        )
        .expect("selected Index method should materialize");
        mono.monomorphize_trait_method_call("write_at", &[receiver, key], &mut index_mut_expr)
            .expect("selected IndexMut method should materialize");

        assert!(matches!(index_expr.kind, HirExprKind::Call(..)));
        assert!(matches!(index_mut_expr.kind, HirExprKind::Call(..)));
        let origins = mono
            .instances
            .records()
            .map(|record| record.origin.clone())
            .collect::<Vec<_>>();
        assert!(origins.contains(&InstanceOrigin::ImplMethod {
            owner: InstanceImplOwner::Named(index_impl_id),
            method: index_method_id,
        }));
        assert!(origins.contains(&InstanceOrigin::ImplMethod {
            owner: InstanceImplOwner::Named(index_mut_impl_id),
            method: index_mut_method_id,
        }));
    }

    fn option_println_method(impl_id: DefId, method_id: DefId) -> HirFunction {
        let impl_generic = GenericParamId {
            owner: impl_id,
            index: 0,
        };
        HirFunction {
            id: method_id,
            name: "println".to_string(),
            generic_params: vec![GenericParamDecl::type_param(impl_generic, "T")],
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "self".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: enum_ty("Option", vec![Type::Generic(impl_generic)]),
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::I32,
            body: empty_body(Type::Unit),
            is_curried: false,
            is_method: true,
            self_receiver: Some(ReceiverMode::Move),
            is_unsafe: false,
        }
    }

    fn byte_slice_println_method() -> HirFunction {
        HirFunction {
            id: DefId::new(CrateId(0), LocalDefId(0)),
            name: "println".to_string(),
            generic_params: vec![],
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "self".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::Slice(Box::new(Type::U8))),
                },
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::I32,
            body: empty_body(Type::Unit),
            is_curried: false,
            is_method: true,
            self_receiver: Some(ReceiverMode::Move),
            is_unsafe: false,
        }
    }

    fn borrowed_slice_println_method(mutable: bool) -> HirFunction {
        HirFunction {
            id: DefId::new(CrateId(0), LocalDefId(0)),
            name: "println".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: def_id(0),
                    index: 0,
                },
                "T",
            )],
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "self".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Reference {
                    mutable,
                    inner: Box::new(Type::Slice(Box::new(Type::Generic(GenericParamId {
                        owner: def_id(0),
                        index: 0,
                    })))),
                },
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::I32,
            body: empty_body(Type::Unit),
            is_curried: false,
            is_method: true,
            self_receiver: Some(ReceiverMode::Move),
            is_unsafe: false,
        }
    }

    fn qualified_option_receiver() -> HirExpr {
        HirExpr {
            kind: HirExprKind::Var("some".to_string()),
            ty: enum_ty("stdlib::option::Option", vec![Type::I64]),
            span: Span::test(),
        }
    }

    fn seed_owner(mono: &mut Monomorphizer, name: &str) {
        let local = match name {
            "Option" | "stdlib::option::Option" => 10,
            "Result" => 11,
            _ => 0,
        };
        let def_id = crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(local));
        mono.resolver.item_paths.insert(name.to_string(), def_id);
        mono.resolver
            .item_names_by_id
            .insert(def_id, name.to_string());
    }

    fn selected_impl_target(
        impl_id: DefId,
        method_id: DefId,
        owner_ty: Type,
    ) -> HirMethodCallTarget {
        let mut target = HirMethodCallTarget::impl_method(impl_id, method_id, None);
        target.owner_substitution = vec![crate::hir::HirTypeBinding {
            param: GenericParamId {
                owner: impl_id,
                index: 0,
            },
            ty: owner_ty,
        }];
        target
    }

    fn selected_map_target(
        impl_id: DefId,
        method_id: DefId,
        owner_ty: Type,
        method_ty: Type,
    ) -> HirMethodCallTarget {
        let mut target = selected_impl_target(impl_id, method_id, owner_ty);
        target.method_substitution = vec![crate::hir::HirTypeBinding {
            param: GenericParamId {
                owner: method_id,
                index: 0,
            },
            ty: method_ty,
        }];
        target
    }

    #[test]
    fn monomorphize_same_named_methods_on_different_impls_use_distinct_method_origins() {
        let mut mono = Monomorphizer::new();
        let option_impl_id = DefId::new(CrateId(0), LocalDefId(10));
        let result_impl_id = DefId::new(CrateId(0), LocalDefId(11));
        let option_method_id = DefId::new(CrateId(0), LocalDefId(12));
        let result_method_id = DefId::new(CrateId(0), LocalDefId(13));
        mono.generic_impls.insert(
            option_impl_id,
            generic_map_impl("Option", option_impl_id, option_method_id),
        );
        mono.generic_impls.insert(
            result_impl_id,
            generic_map_impl("Result", result_impl_id, result_method_id),
        );
        seed_generic_enum_receiver_pattern(&mut mono, option_impl_id, "Option");
        seed_generic_enum_receiver_pattern(&mut mono, result_impl_id, "Result");

        let (option_recv, option_func_arg, mut option_expr) = map_expr_for_type("Option");
        if let HirExprKind::MethodCall(_, _, _, _, target) = &mut option_expr.kind {
            *target = selected_map_target(option_impl_id, option_method_id, Type::I64, Type::I64);
        }
        let _ = mono.monomorphize_standalone_method_call(
            "map",
            &[option_recv, option_func_arg],
            &mut option_expr,
        );
        let (result_recv, result_func_arg, mut result_expr) = map_expr_for_type("Result");
        if let HirExprKind::MethodCall(_, _, _, _, target) = &mut result_expr.kind {
            *target = selected_map_target(result_impl_id, result_method_id, Type::I64, Type::I64);
        }
        let _ = mono.monomorphize_standalone_method_call(
            "map",
            &[result_recv, result_func_arg],
            &mut result_expr,
        );

        let origins = mono
            .instances
            .records()
            .map(|record| record.origin.clone())
            .collect::<Vec<_>>();
        assert!(origins.contains(&InstanceOrigin::ImplMethod {
            owner: InstanceImplOwner::Named(option_impl_id),
            method: option_method_id,
        }));
        assert!(origins.contains(&InstanceOrigin::ImplMethod {
            owner: InstanceImplOwner::Named(result_impl_id),
            method: result_method_id,
        }));
        assert_eq!(origins.len(), 2);
    }

    #[test]
    fn monomorphize_same_owner_distinct_methods_use_distinct_method_origins() {
        let mut mono = Monomorphizer::new();
        let impl_id = DefId::new(CrateId(0), LocalDefId(20));
        let map_method_id = DefId::new(CrateId(0), LocalDefId(21));
        let fold_method_id = DefId::new(CrateId(0), LocalDefId(22));
        let mut fold_method = map_method_for_type("Option", impl_id, fold_method_id);
        fold_method.name = "fold".to_string();
        mono.generic_impls.insert(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Option".to_string()),
                type_name: "Option".to_string(),
                type_generics: type_generic(impl_id, "T"),
                receiver_pattern: vec![Type::Generic(GenericParamId {
                    owner: impl_id,
                    index: 0,
                })]
                .into(),
                trait_name: None,
                trait_id: None,
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([
                    (
                        "map".to_string(),
                        map_method_for_type("Option", impl_id, map_method_id),
                    ),
                    ("fold".to_string(), fold_method),
                ]),
            },
        );
        seed_generic_enum_receiver_pattern(&mut mono, impl_id, "Option");

        let (map_recv, map_func_arg, mut map_expr) = map_expr_for_type("Option");
        if let HirExprKind::MethodCall(_, _, _, _, target) = &mut map_expr.kind {
            *target = selected_map_target(impl_id, map_method_id, Type::I64, Type::I64);
        }
        let _ = mono.monomorphize_standalone_method_call(
            "map",
            &[map_recv, map_func_arg],
            &mut map_expr,
        );
        let (fold_recv, fold_func_arg, mut fold_expr) = map_expr_for_type("Option");
        if let HirExprKind::MethodCall(_, _, _, _, target) = &mut fold_expr.kind {
            *target = selected_map_target(impl_id, fold_method_id, Type::I64, Type::I64);
        }
        let _ = mono.monomorphize_standalone_method_call(
            "fold",
            &[fold_recv, fold_func_arg],
            &mut fold_expr,
        );

        let origins = mono
            .instances
            .records()
            .map(|record| record.origin.clone())
            .collect::<Vec<_>>();
        assert!(origins.contains(&InstanceOrigin::ImplMethod {
            owner: InstanceImplOwner::Named(impl_id),
            method: map_method_id,
        }));
        assert!(origins.contains(&InstanceOrigin::ImplMethod {
            owner: InstanceImplOwner::Named(impl_id),
            method: fold_method_id,
        }));
        assert_eq!(origins.len(), 2);
    }

    #[test]
    fn standalone_method_call_uses_canonical_receiver_pattern_for_qualified_receiver() {
        let mut mono = Monomorphizer::new();
        seed_owner(&mut mono, "Option");
        let impl_id = DefId::new(CrateId(0), LocalDefId(60));
        let method_id = DefId::new(CrateId(0), LocalDefId(61));
        let method = option_map_method(impl_id, method_id);
        mono.generic_impls.insert(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Option".to_string()),
                type_name: "Option".to_string(),
                type_generics: type_generic(impl_id, "T"),
                receiver_pattern: vec![Type::Generic(GenericParamId {
                    owner: impl_id,
                    index: 0,
                })]
                .into(),
                trait_name: None,
                trait_id: None,
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("map".to_string(), method)]),
            },
        );
        seed_generic_enum_receiver_pattern(&mut mono, impl_id, "Option");

        let recv = qualified_option_receiver();
        let func_arg = HirExpr {
            kind: HirExprKind::Var("inc".to_string()),
            ty: Type::function(vec![Type::I64], Type::I64),
            span: Span::test(),
        };
        let mut expr = HirExpr {
            kind: HirExprKind::MethodCall(
                Box::new(recv.clone()),
                "map".to_string(),
                vec![func_arg.clone()],
                Some(ReceiverMode::Move),
                selected_map_target(impl_id, method_id, Type::I64, Type::I64),
            ),
            ty: enum_ty("stdlib::option::Option", vec![Type::I64]),
            span: Span::test(),
        };

        let _ = mono.monomorphize_standalone_method_call("map", &[recv, func_arg], &mut expr);

        assert!(matches!(expr.kind, HirExprKind::Call(_, _, _)));
    }

    #[test]
    fn trait_method_call_uses_canonical_receiver_pattern_for_qualified_receiver() {
        let mut mono = Monomorphizer::new();
        seed_owner(&mut mono, "Option");
        let impl_id = DefId::new(CrateId(0), LocalDefId(62));
        let trait_id = DefId::new(CrateId(0), LocalDefId(63));
        let method_id = DefId::new(CrateId(0), LocalDefId(64));
        let method = option_println_method(impl_id, method_id);
        mono.trait_impls.insert(
            trait_id,
            vec![HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Option".to_string()),
                type_name: "Option".to_string(),
                type_generics: type_generic(impl_id, "T"),
                receiver_pattern: vec![Type::Generic(GenericParamId {
                    owner: impl_id,
                    index: 0,
                })]
                .into(),
                trait_name: Some("Show".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("println".to_string(), method)]),
            }],
        );
        seed_generic_enum_receiver_pattern(&mut mono, impl_id, "Option");

        let recv = qualified_option_receiver();
        let mut selected_target = selected_impl_target(impl_id, method_id, Type::I64);
        if let crate::hir::HirSelectedMethodTarget::ImplMethod { selected_trait, .. } =
            &mut selected_target.target
        {
            *selected_trait = Some(crate::hir::HirSelectedTraitMember {
                trait_id,
                member_id: method_id,
                trait_args: Vec::new(),
            });
        }
        let mut expr = HirExpr {
            kind: HirExprKind::MethodCall(
                Box::new(recv.clone()),
                "println".to_string(),
                vec![],
                Some(ReceiverMode::Move),
                selected_target,
            ),
            ty: Type::I32,
            span: Span::test(),
        };

        let _ = mono.monomorphize_trait_method_call("println", &[recv], &mut expr);

        assert!(matches!(expr.kind, HirExprKind::Call(_, _, _)));
    }

    #[test]
    fn selected_trait_impl_method_call_rejects_wrong_method_id() {
        let mut mono = Monomorphizer::new();
        let impl_id = DefId::new(CrateId(0), LocalDefId(710));
        let trait_id = DefId::new(CrateId(0), LocalDefId(711));
        let actual_method_id = DefId::new(CrateId(0), LocalDefId(712));
        let selected_method_id = DefId::new(CrateId(0), LocalDefId(713));
        let method = option_println_method(impl_id, actual_method_id);
        mono.trait_impls.insert(
            trait_id,
            vec![HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Option".to_string()),
                type_name: "Option".to_string(),
                type_generics: type_generic(impl_id, "T"),
                receiver_pattern: vec![Type::Generic(GenericParamId {
                    owner: impl_id,
                    index: 0,
                })]
                .into(),
                trait_name: Some("Show".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("println".to_string(), method)]),
            }],
        );
        let recv = qualified_option_receiver();
        let mut expr = HirExpr {
            kind: HirExprKind::MethodCall(
                Box::new(recv.clone()),
                "println".to_string(),
                vec![],
                Some(ReceiverMode::Move),
                HirMethodCallTarget::impl_method(
                    impl_id,
                    selected_method_id,
                    Some(crate::hir::HirSelectedTraitMember {
                        trait_id,
                        member_id: selected_method_id,
                        trait_args: Vec::new(),
                    }),
                ),
            ),
            ty: Type::I32,
            span: Span::test(),
        };

        let _ = mono.monomorphize_trait_method_call("println", &[recv], &mut expr);

        assert!(matches!(expr.kind, HirExprKind::MethodCall(..)));
        assert!(mono.instances.records().next().is_none());
    }

    #[test]
    fn selected_trait_method_call_without_impl_id_specializes_matching_trait_impl_method() {
        let mut mono = Monomorphizer::new();
        let impl_id = DefId::new(CrateId(0), LocalDefId(720));
        let trait_id = DefId::new(CrateId(0), LocalDefId(721));
        let actual_method_id = DefId::new(CrateId(0), LocalDefId(722));
        let trait_method_id = DefId::new(CrateId(0), LocalDefId(723));
        let method = option_println_method(impl_id, actual_method_id);
        mono.effective_trait_methods
            .insert((impl_id, trait_method_id), actual_method_id);
        mono.set_impl_receiver_pattern_for_test(
            impl_id,
            HirImplReceiverPattern::Exact(enum_ty(
                "Option",
                vec![Type::Generic(GenericParamId {
                    owner: impl_id,
                    index: 0,
                })],
            )),
        );
        mono.trait_impls.insert(
            trait_id,
            vec![HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Option".to_string()),
                type_name: "Option".to_string(),
                type_generics: type_generic(impl_id, "T"),
                receiver_pattern: vec![Type::Generic(GenericParamId {
                    owner: impl_id,
                    index: 0,
                })]
                .into(),
                trait_name: Some("Show".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("println".to_string(), method)]),
            }],
        );

        let recv = qualified_option_receiver();
        let mut expr = HirExpr {
            kind: HirExprKind::MethodCall(
                Box::new(recv.clone()),
                "println".to_string(),
                vec![],
                Some(ReceiverMode::Move),
                HirMethodCallTarget::trait_method(
                    trait_id,
                    trait_method_id,
                    Vec::new(),
                    crate::hir::HirTraitDispatchKind::TraitBound,
                ),
            ),
            ty: Type::I32,
            span: Span::test(),
        };

        let _ = mono.monomorphize_trait_method_call("println", &[recv], &mut expr);

        let HirExprKind::Call(callee, _, _) = &expr.kind else {
            panic!("trait-bound method call should lower to a direct call");
        };
        let HirExprKind::ResolvedVar(reference) = &callee.kind else {
            panic!("trait-bound method callee should be resolved");
        };
        let HirVarTarget::Instance(instance_id) = reference.target else {
            panic!("trait-bound method callee should target an instance");
        };
        let record = mono
            .instances
            .record(instance_id)
            .expect("instance is recorded");
        assert_eq!(
            record.origin,
            InstanceOrigin::ImplMethod {
                owner: InstanceImplOwner::Named(impl_id),
                method: actual_method_id,
            }
        );
    }

    #[test]
    fn test_monomorphize_trait_method_call_does_not_treat_str_as_u8_slice() {
        let mut mono = Monomorphizer::new();
        seed_owner(&mut mono, "&[U8]");
        mono.trait_impls.insert(
            def_id(400),
            vec![HirImpl {
                id: DefId::new(CrateId(0), LocalDefId(0)),
                owner: HirImplOwner::BuiltinSlice,
                type_name: "&[U8]".to_string(),
                type_generics: vec![],
                receiver_pattern: vec![Type::U8].into(),
                trait_name: Some("Bytes".to_string()),
                trait_id: Some(def_id(400)),
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("println".to_string(), byte_slice_println_method())]),
            }],
        );

        let recv = HirExpr {
            kind: HirExprKind::Var("text".to_string()),
            ty: Type::Reference {
                mutable: false,
                inner: Box::new(Type::Str),
            },
            span: Span::test(),
        };
        let mut expr = HirExpr {
            kind: HirExprKind::MethodCall(
                Box::new(recv.clone()),
                "println".to_string(),
                vec![],
                Some(ReceiverMode::Move),
                HirMethodCallTarget::trait_method(
                    def_id(999),
                    def_id(998),
                    Vec::new(),
                    crate::hir::HirTraitDispatchKind::TraitBound,
                ),
            ),
            ty: Type::I32,
            span: Span::test(),
        };

        let _ = mono.monomorphize_trait_method_call("println", &[recv], &mut expr);

        assert!(matches!(expr.kind, HirExprKind::MethodCall(..)));
    }

    #[test]
    fn test_collect_impls_keeps_object_backed_concrete_trait_impls_available_for_dispatch() {
        let mut mono = Monomorphizer::new();
        let concrete_impl = HirImpl {
            id: DefId::new(CrateId(0), LocalDefId(0)),
            owner: HirImplOwner::BuiltinSlice,
            type_name: "&[U8]".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![Type::U8].into(),
            trait_name: Some("Show".to_string()),
            trait_id: Some(def_id(500)),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("println".to_string(), byte_slice_println_method())]),
        };
        mono.collect_impls(&[concrete_impl]);

        assert_eq!(mono.trait_impls[&def_id(500)].len(), 1);
    }

    #[test]
    fn collect_impls_keeps_same_named_traits_separate_by_def_id() {
        let mut mono = Monomorphizer::new();
        let first_trait_id = def_id(610);
        let second_trait_id = def_id(611);
        let first_method = option_println_method(def_id(614), def_id(612));
        let second_method = option_println_method(def_id(615), def_id(613));

        mono.collect_impls(&[
            HirImpl {
                id: def_id(614),
                owner: HirImplOwner::Named("Option".to_string()),
                type_name: "Option".to_string(),
                type_generics: type_generic(def_id(614), "T"),
                receiver_pattern: vec![Type::Generic(GenericParamId {
                    owner: def_id(614),
                    index: 0,
                })]
                .into(),
                trait_name: Some("Show".to_string()),
                trait_id: Some(first_trait_id),
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("println".to_string(), first_method)]),
            },
            HirImpl {
                id: def_id(615),
                owner: HirImplOwner::Named("Option".to_string()),
                type_name: "Option".to_string(),
                type_generics: type_generic(def_id(615), "T"),
                receiver_pattern: vec![Type::Generic(GenericParamId {
                    owner: def_id(615),
                    index: 0,
                })]
                .into(),
                trait_name: Some("Show".to_string()),
                trait_id: Some(second_trait_id),
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("println".to_string(), second_method)]),
            },
        ]);

        assert_eq!(mono.trait_impls.len(), 2);
    }

    #[test]
    fn test_monomorphize_trait_method_call_preserves_borrowed_slice_self_type() {
        let mut mono = Monomorphizer::new();
        let impl_id = DefId::new(CrateId(0), LocalDefId(0));
        let trait_id = def_id(600);
        let method_id = def_id(601);
        let mut method = borrowed_slice_println_method(false);
        method.id = method_id;
        mono.trait_impls.insert(
            trait_id,
            vec![HirImpl {
                id: impl_id,
                owner: HirImplOwner::BuiltinSlice,
                type_name: "&[T]".to_string(),
                type_generics: type_generic(impl_id, "T"),
                receiver_pattern: vec![Type::Generic(GenericParamId {
                    owner: def_id(0),
                    index: 0,
                })]
                .into(),
                trait_name: Some("Show".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![],
                trait_arg_types: vec![],
                associated_types: vec![],
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("println".to_string(), method)]),
            }],
        );
        mono.set_impl_receiver_pattern_for_test(
            impl_id,
            crate::hir::HirImplReceiverPattern::SliceFamily {
                element: Type::Generic(GenericParamId {
                    owner: impl_id,
                    index: 0,
                }),
            },
        );

        let recv = HirExpr {
            kind: HirExprKind::Var("bytes".to_string()),
            ty: Type::Reference {
                mutable: false,
                inner: Box::new(Type::Slice(Box::new(Type::U8))),
            },
            span: Span::test(),
        };
        let mut selected_target = selected_impl_target(impl_id, method_id, Type::U8);
        if let crate::hir::HirSelectedMethodTarget::ImplMethod { selected_trait, .. } =
            &mut selected_target.target
        {
            *selected_trait = Some(crate::hir::HirSelectedTraitMember {
                trait_id,
                member_id: method_id,
                trait_args: Vec::new(),
            });
        }
        let mut expr = HirExpr {
            kind: HirExprKind::MethodCall(
                Box::new(recv.clone()),
                "println".to_string(),
                vec![],
                Some(ReceiverMode::Move),
                selected_target,
            ),
            ty: Type::I32,
            span: Span::test(),
        };

        let _ = mono.monomorphize_trait_method_call("println", &[recv], &mut expr);

        let body = mono
            .instances
            .records()
            .find_map(|record| mono.instances.pre_mir_body(record.id))
            .expect("expected monomorphized borrowed slice method");

        assert_eq!(
            body.params[0].ty,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::Slice(Box::new(Type::U8))),
            }
        );
    }

    #[test]
    fn specialize_impl_method_preserves_concrete_nominal_self_type_without_impl_args() {
        let mut mono = Monomorphizer::new();
        let foo_id = def_id(77);
        let method = HirFunction {
            id: def_id(78),
            name: "id".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: def_id(0),
                    index: 0,
                },
                "U",
            )],
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "self".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Struct {
                    id: foo_id,
                    args: Vec::new(),
                },
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::I64,
            body: HirBlock {
                stmts: vec![HirStmt::Expr(HirExpr {
                    kind: HirExprKind::Var("self".to_string()),
                    ty: Type::Generic(crate::types::GenericParamId {
                        owner: crate::ids::DefId::new(
                            crate::ids::CrateId(0),
                            crate::ids::LocalDefId(0),
                        ),
                        index: 0,
                    }),
                    span: Span::test(),
                })],
                ty: Type::I64,
            },
            is_curried: false,
            is_method: true,
            self_receiver: Some(ReceiverMode::Move),
            is_unsafe: false,
        };

        let i64_id = mono.intern_type(&Type::I64);
        let specialized = mono.specialize_impl_method(
            &method,
            &[i64_id],
            &[],
            "Foo_id_I64",
            DefId::new(CrateId(0), LocalDefId(0)),
            None,
        );

        let HirStmt::Expr(HirExpr { ty, .. }) = &specialized.body.stmts[0] else {
            panic!("expected self expression");
        };
        assert_eq!(
            ty,
            &Type::Struct {
                id: foo_id,
                args: Vec::new(),
            }
        );
    }

    #[test]
    fn specialize_impl_method_preserves_nested_nominal_receiver_shape() {
        let mut mono = Monomorphizer::new();
        let impl_id = def_id(79);
        let option_id = def_id(80);
        let generic = Type::Generic(crate::types::GenericParamId {
            owner: impl_id,
            index: 0,
        });
        let receiver = Type::Enum {
            id: option_id,
            args: vec![Type::Enum {
                id: option_id,
                args: vec![generic],
            }],
        };
        let method = HirFunction {
            id: def_id(81),
            name: "flatten".to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "self".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: receiver.clone(),
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::I64,
            body: HirBlock {
                stmts: vec![HirStmt::Expr(HirExpr {
                    kind: HirExprKind::Var("self".to_string()),
                    ty: receiver,
                    span: Span::test(),
                })],
                ty: Type::I64,
            },
            is_curried: false,
            is_method: true,
            self_receiver: Some(ReceiverMode::Move),
            is_unsafe: false,
        };
        let i64_id = mono.intern_type(&Type::I64);

        let specialized = mono.specialize_impl_method(
            &method,
            &[i64_id],
            &[GenericParamDecl::type_param(
                GenericParamId {
                    owner: impl_id,
                    index: 0,
                },
                "T",
            )],
            "Option_Option_I64_flatten",
            impl_id,
            None,
        );
        let expected = Type::Enum {
            id: option_id,
            args: vec![Type::Enum {
                id: option_id,
                args: vec![Type::I64],
            }],
        };
        let HirStmt::Expr(HirExpr { ty, .. }) = &specialized.body.stmts[0] else {
            panic!("expected self expression");
        };

        assert_eq!(specialized.params[0].ty, expected);
        assert_eq!(ty, &expected);
    }
}

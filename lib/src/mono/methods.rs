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
mod tests;

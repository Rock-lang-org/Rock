//! Static method values lowered to lambdas.

use crate::hir::*;
use crate::lexer::Span;
use crate::lower::Lowerer;
use crate::types::{GenericParamId, Type};
use std::collections::{HashMap, HashSet};

impl Lowerer {
    pub(crate) fn infer_static_trait_argument_coercions(
        &mut self,
        callee: &HirExpr,
        args: &[HirExpr],
    ) {
        let HirExprKind::Lambda {
            params,
            body,
            captures,
        } = &callee.kind
        else {
            return;
        };
        let [HirStmt::Expr(HirExpr {
            kind: HirExprKind::Call(_, forwarded, Some(HirCallTarget::StaticMethod(target))),
            ..
        })] = body.stmts.as_slice()
        else {
            return;
        };
        if !captures.is_empty() || params.len() != args.len() || forwarded.len() != params.len()
            || !forwarded.iter().zip(params).all(|(arg, param)| matches!(&arg.kind,
                HirExprKind::ResolvedVar(reference) if reference.target == HirVarTarget::Local(param.local_id)))
        {
            return;
        }
        let Some(trait_id) = target.method.trait_id() else {
            return;
        };
        if target.method.impl_id().is_some() {
            return;
        }
        let owner = self.engine.resolve(&target.owner_ty);
        let mut direct = self.engine.clone_for_probe();
        if params
            .iter()
            .zip(args)
            .any(|(param, arg)| direct.unify(&param.ty, &arg.ty).is_err())
        {
            return;
        }
        let direct_args = target
            .method
            .trait_args()
            .iter()
            .map(|ty| direct.resolve(ty))
            .collect::<Vec<_>>();
        if !matches!(
            self.selection_service()
                .select_trait_impl_strict(&owner, trait_id, &direct_args),
            Err(crate::selection::SelectionDiagnostic::NoImplementation { .. })
        ) {
            return;
        }

        // Keep the expected parameter open until overload selection has considered
        // the normal array-reference-to-slice coercion. Exact impls take precedence.
        let mut coerced = self.engine.clone_for_probe();
        let mut changed = false;
        for (param, arg) in params.iter().zip(args) {
            let mut actual = self.engine.resolve(&arg.ty);
            if let Type::Reference { mutable, inner } = &actual {
                if let Type::Array(element, _) = inner.as_ref() {
                    actual = Type::Reference {
                        mutable: *mutable,
                        inner: Box::new(Type::Slice(element.clone())),
                    };
                    changed = true;
                }
            }
            if coerced.unify(&param.ty, &actual).is_err() {
                return;
            }
        }
        let trait_args = target
            .method
            .trait_args()
            .iter()
            .map(|ty| coerced.resolve(ty))
            .collect::<Vec<_>>();
        if changed
            && self
                .selection_service()
                .select_trait_impl_strict(&owner, trait_id, &trait_args)
                .is_ok()
        {
            self.engine.commit_probe(coerced.clone_for_commit());
        }
    }

    pub(super) fn static_method_value_lambda(
        &mut self,
        callee: HirExpr,
        target: HirStaticMethodTarget,
    ) -> HirExpr {
        let span = callee.span.clone();
        let Type::Function {
            params,
            ret,
            safety,
            ..
        } = callee.ty.clone()
        else {
            self.diagnostics.push_with_span(
                "selected static method value is not callable".to_string(),
                span.clone(),
            );
            return self.error_expression_at(span);
        };
        let lambda_params = params
            .iter()
            .enumerate()
            .map(|(index, ty)| HirParam {
                name: format!("__static_arg_{index}"),
                local_id: self.fresh_local_id(),
                ty: ty.clone(),
                mutable: false,
                is_ref: false,
            })
            .collect::<Vec<_>>();
        let args = lambda_params
            .iter()
            .map(|param| HirExpr {
                ty: param.ty.clone(),
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: param.name.clone(),
                    target: HirVarTarget::Local(param.local_id),
                }),
                span: span.clone(),
            })
            .collect::<Vec<_>>();
        let method_info = target
            .method
            .impl_id()
            .and_then(|impl_id| self.items.impl_def(impl_id))
            .and_then(|imp| {
                let method_id = target.method.method_id()?;
                imp.methods.iter().find_map(|(name, method)| {
                    (method.id == method_id).then_some((name.clone(), method.clone()))
                })
            });
        let method_info = method_info.or_else(|| {
            target.method.method_id().and_then(|method_id| {
                self.items
                    .impl_defs_in_order()
                    .flat_map(|(_, imp)| imp.methods.iter())
                    .find_map(|(name, method)| {
                        (method.id == method_id).then_some((name.clone(), method.clone()))
                    })
            })
        });
        let call = if let Some((method_name, method)) =
            method_info.filter(|(_, method)| method.self_receiver.is_some())
        {
            let Some(receiver) = args.first().cloned() else {
                self.diagnostics.push_with_span(
                    "selected receiver method value has no receiver parameter".to_string(),
                    span.clone(),
                );
                return self.error_expression_at(span.clone());
            };
            HirExpr {
                ty: ret.as_ref().clone(),
                kind: HirExprKind::MethodCall(
                    Box::new(receiver),
                    method_name.clone(),
                    args.into_iter().skip(1).collect(),
                    method.self_receiver,
                    Some(target.method.clone()),
                ),
                span: span.clone(),
            }
        } else {
            HirExpr {
                ty: ret.as_ref().clone(),
                kind: HirExprKind::Call(
                    Box::new(callee),
                    args,
                    Some(HirCallTarget::StaticMethod(target)),
                ),
                span: span.clone(),
            }
        };
        let body = HirBlock {
            ty: ret.as_ref().clone(),
            stmts: vec![HirStmt::Expr(call)],
        };
        let captures = self.collect_lambda_captures(&body, &lambda_params);

        HirExpr {
            ty: Self::lambda_function_type(params, ret.as_ref().clone(), safety, &captures),
            kind: HirExprKind::Lambda {
                params: lambda_params,
                body,
                captures,
            },
            span,
        }
    }

    pub(super) fn instantiate_static_method_value(
        &mut self,
        resolved: crate::lower::resolution::LowerResolvedStaticMethod,
        span: Span,
    ) -> Result<(HirExpr, HirStaticMethodTarget), String> {
        let resolved_ty = self.instantiate_resolved_value_type(&resolved.value, span.clone());
        let (ty, fresh_substitution) = self.freshen_type_vars(resolved_ty, span.clone());
        let mut substitution = HashMap::new();
        Self::infer_generic_subst_from_types(&resolved.value.ty, &ty, &mut substitution);
        let mut method_generic_params = if !resolved.method_generic_params.is_empty() {
            resolved.method_generic_params.clone()
        } else if resolved
            .target
            .impl_id()
            .is_some_and(|impl_id| impl_id.crate_id == self.root_crate_id)
            && resolved.target.method_id().is_some()
        {
            let method_id = resolved.target.method_id().expect("checked above");
            self.items
                .impl_defs_in_order()
                .find_map(|(_, imp)| {
                    imp.methods
                        .values()
                        .find(|method| method.id == method_id)
                        .map(|method| method.generic_params.iter().map(|param| param.id).collect())
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        method_generic_params.retain(|param| !resolved.owner_generic_params.contains(param));
        let mut raw_type_vars = Vec::new();
        crate::type_services::visit::visit_type(&resolved.value.ty, &mut |nested: &Type| {
            if let Type::TypeVar(id) = nested {
                if !raw_type_vars.contains(id) {
                    raw_type_vars.push(*id);
                }
            }
        });
        if method_generic_params.is_empty()
            && resolved
                .target
                .impl_id()
                .is_some_and(|impl_id| impl_id.crate_id == self.root_crate_id)
        {
            if let Some(method_id) = resolved.target.method_id() {
                method_generic_params = raw_type_vars
                    .iter()
                    .enumerate()
                    .map(|(index, _)| GenericParamId {
                        owner: method_id,
                        index: index as u32,
                    })
                    .collect();
            }
        }
        for (param, id) in method_generic_params.iter().zip(raw_type_vars) {
            if let Some(ty) = fresh_substitution.get(&id) {
                substitution.entry(*param).or_insert_with(|| ty.clone());
            }
        }
        let receiver_pattern = match &resolved.receiver_pattern {
            HirImplReceiverPattern::Exact(ty) | HirImplReceiverPattern::Constructor(ty) => {
                ty.clone()
            }
            HirImplReceiverPattern::SliceFamily { element } => {
                Type::Slice(Box::new(element.clone()))
            }
        };
        let owner_relation = Self::infer_static_owner_signature_relation(
            &receiver_pattern,
            &resolved.value.ty,
            &resolved.owner_generic_params,
            &resolved.value.name,
        )?;

        let mut target = resolved.target;
        target.owner_substitution = resolved
            .owner_generic_params
            .iter()
            .map(|&param| {
                substitution
                    .get(&param)
                    .cloned()
                    .or_else(|| {
                        owner_relation
                            .get(&param)
                            .map(|relation_ty| relation_ty.substitute_generics(&substitution))
                    })
                    .map(|ty| HirTypeBinding { param, ty })
                    .ok_or_else(|| {
                        format!(
                            "static method `{}` could not infer an owner generic parameter",
                            resolved.value.name
                        )
                    })
            })
            .collect::<Result<_, _>>()?;
        for binding in &mut target.owner_substitution {
            binding.ty = binding.ty.substitute(&fresh_substitution);
        }
        target.method_substitution = method_generic_params
            .iter()
            .map(|&param| {
                substitution
                    .get(&param)
                    .cloned()
                    .map(|ty| HirTypeBinding { param, ty })
                    .ok_or_else(|| {
                        format!(
                            "static method `{}` could not infer a method generic parameter",
                            resolved.value.name
                        )
                    })
            })
            .collect::<Result<_, _>>()?;
        let mut target_substitution = HashMap::new();
        for binding in target
            .owner_substitution
            .iter()
            .chain(&target.method_substitution)
        {
            match target_substitution.get(&binding.param) {
                Some(previous) if previous != &binding.ty => {
                    return Err(format!(
                        "static method `{}` has conflicting substitutions: {} vs {}",
                        resolved.value.name,
                        self.display_type(previous),
                        self.display_type(&binding.ty)
                    ));
                }
                Some(_) => {}
                None => {
                    target_substitution.insert(binding.param, binding.ty.clone());
                }
            }
        }
        if let Some(trait_args) = target.trait_args_mut() {
            for trait_arg in trait_args {
                *trait_arg = trait_arg.substitute_generics(&target_substitution);
            }
        }

        let owner_substitution = target
            .owner_substitution
            .iter()
            .map(|binding| (binding.param, binding.ty.clone()))
            .collect::<HashMap<_, _>>();
        let owner_ty = match resolved.receiver_pattern {
            HirImplReceiverPattern::Exact(ty) | HirImplReceiverPattern::Constructor(ty) => {
                ty.substitute_generics(&owner_substitution)
            }
            HirImplReceiverPattern::SliceFamily { element } => {
                Type::Slice(Box::new(element.substitute_generics(&owner_substitution)))
            }
        };
        self.register_static_owner_defaults(&owner_ty, &ty);
        let function_id = match resolved.value.target {
            Some(HirVarTarget::Function(function_id)) => function_id,
            _ => {
                return Err(format!(
                    "static method `{}` has no function identity",
                    resolved.value.name
                ));
            }
        };
        let callee = HirExpr {
            ty,
            kind: HirExprKind::ResolvedVar(HirVarRef {
                name: resolved.value.name,
                target: HirVarTarget::Function(function_id),
            }),
            span,
        };

        Ok((
            callee,
            HirStaticMethodTarget {
                owner_ty,
                method: target,
            },
        ))
    }

    fn freshen_type_vars(
        &mut self,
        ty: Type,
        span: Span,
    ) -> (Type, HashMap<crate::ids::TypeVarId, Type>) {
        let mut ids = HashSet::new();
        crate::type_services::visit::visit_type(&ty, &mut |nested: &Type| {
            if let Type::TypeVar(id) = nested {
                ids.insert(*id);
            }
        });
        let substitution = ids
            .into_iter()
            .map(|id| {
                let kind = self.engine.kind_of_type_var(id);
                (id, self.engine.fresh_type_var_at_kind(span.clone(), kind))
            })
            .collect::<HashMap<_, _>>();
        (ty.substitute(&substitution), substitution)
    }
}

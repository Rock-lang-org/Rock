use crate::hir::{HirExpr, HirExprKind};
use crate::lower::Lowerer;
use crate::types::Type;

impl Lowerer {
    pub(crate) fn instantiate_selected_callable_parameters(
        &mut self,
        selected: &mut crate::selection::SelectedMethod,
    ) {
        if selected.impl_def.is_none() {
            return;
        }
        let Some(function) = &mut selected.function else {
            return;
        };
        let mut variables = std::collections::HashSet::new();
        for ty in function
            .params
            .iter()
            .skip(usize::from(function.is_method))
            .map(|p| &p.ty)
            .chain(std::iter::once(&function.ret_type))
        {
            crate::type_services::visit::visit_type(ty, &mut |ty: &Type| {
                if let Type::TypeVar(id) = ty {
                    variables.insert(*id);
                }
            });
        }
        if !variables.iter().any(|id| {
            self.engine.get_bounds(*id).iter().any(|bound| {
                self.language_items
                    .callable_protocols()
                    .any(|(_, trait_id, _)| trait_id == bound.trait_id)
            })
        }) {
            return;
        }
        let span = selected.receiver.span.clone();
        let mut substitution = std::collections::HashMap::new();
        let bounds = variables
            .iter()
            .map(|id| (*id, self.engine.get_bounds(*id)))
            .collect::<Vec<_>>();
        for (_, bounds) in &bounds {
            for bound in bounds {
                for arg in &bound.type_args {
                    crate::type_services::visit::visit_type(arg, &mut |ty: &Type| {
                        if let Type::TypeVar(id) = ty {
                            variables.insert(*id);
                        }
                    });
                }
            }
        }
        let mut representatives = std::collections::HashMap::new();
        for id in variables {
            let ty = self.engine.resolve(&Type::TypeVar(id));
            let replacement = match ty {
                Type::TypeVar(root) => representatives
                    .entry(root)
                    .or_insert_with(|| {
                        let kind = self.engine.kind_of_type_var(root);
                        self.engine.fresh_type_var_at_kind(span.clone(), kind)
                    })
                    .clone(),
                ty => ty.substitute_generics(&selected.owner_substitution),
            };
            substitution.insert(id, replacement);
        }
        substitution.extend(representatives);
        for (id, bounds) in bounds {
            let Some(Type::TypeVar(instance)) = substitution.get(&id) else {
                continue;
            };
            for bound in bounds {
                let bound = crate::types::TraitBound {
                    trait_id: bound.trait_id,
                    type_args: bound
                        .type_args
                        .iter()
                        .map(|ty| {
                            ty.substitute(&substitution)
                                .substitute_generics(&selected.owner_substitution)
                        })
                        .collect(),
                };
                self.engine.add_bound(*instance, bound.clone());
                self.constraint_store.add_trait(
                    Type::TypeVar(*instance),
                    bound,
                    span.clone(),
                    "inferred method callable argument",
                );
            }
        }
        for param in &mut function.params {
            param.ty = param.ty.substitute(&substitution);
        }
        function.ret_type = function.ret_type.substitute(&substitution);
        for param in &mut selected.substituted_params {
            param.ty = param.ty.substitute(&substitution);
        }
        selected.return_type = selected.return_type.substitute(&substitution);
    }

    pub(crate) fn callable_method_candidate(
        &mut self,
        receiver: HirExpr,
        member_name: Option<&str>,
    ) -> Option<crate::selection::SelectedMethod> {
        if let Type::TypeVar(id) = self.engine.resolve(&receiver.ty) {
            if self.engine.get_bounds(id).is_empty() {
                return None;
            }
        }
        let span = receiver.span.clone();
        let mut candidates = self.receiver_adjustment_candidates(receiver);
        for candidate in &mut candidates {
            candidate.expr.ty =
                self.resolve_projection_type(&self.engine.resolve(&candidate.expr.ty));
            candidate.can_autoref_mut |= self.expr_is_mutable_lvalue(&candidate.expr)
                || matches!(candidate.expr.ty, Type::Reference { mutable: true, .. })
                || !matches!(
                    candidate.expr.kind,
                    HirExprKind::Var(_)
                        | HirExprKind::ResolvedVar(_)
                        | HirExprKind::FieldAccess(..)
                        | HirExprKind::TupleIndex(..)
                        | HirExprKind::Deref(_)
                );
        }
        let result = self.selection_service().select_callable_method(
            &candidates,
            &self.language_items,
            |ty| match ty {
                Type::TypeVar(id) => self.engine.get_bounds(*id),
                Type::Generic(id) => self
                    .current_impl_bounds()
                    .get(id)
                    .cloned()
                    .unwrap_or_default(),
                _ => Vec::new(),
            },
            member_name,
        );
        match result {
            Ok(selected) => selected,
            Err(error) => {
                self.diagnostics
                    .push_selection_with_span(error.message(), span);
                None
            }
        }
    }

    pub(crate) fn lower_callable_call(
        &mut self,
        selected: crate::selection::SelectedMethod,
        args: Vec<HirExpr>,
    ) -> HirExpr {
        let span = selected.receiver.span.clone();
        let packed = match args.len() {
            0 => HirExpr {
                ty: Type::Unit,
                kind: HirExprKind::Unit,
                span: span.clone(),
            },
            1 => args.into_iter().next().unwrap(),
            _ => HirExpr {
                ty: Type::Tuple(args.iter().map(|arg| arg.ty.clone()).collect()),
                kind: HirExprKind::TupleLiteral(args),
                span: span.clone(),
            },
        };
        let receiver =
            self.apply_receiver_adjustment(selected.receiver.clone(), selected.receiver_adjustment);
        let Some((method, ty, args, target)) =
            self.selected_method_call_types(&selected, vec![packed])
        else {
            return self.error_expression_at(span);
        };
        if method.is_unsafe && !self.is_in_unsafe() {
            self.diagnostics.push_selection_with_span(
                "Call to unsafe callable requires an unsafe block".into(),
                span.clone(),
            );
            return self.error_expression_at(span);
        }
        HirExpr {
            ty,
            kind: HirExprKind::MethodCall(
                Box::new(receiver),
                method.name,
                args,
                method.self_receiver,
                Some(target),
            ),
            span,
        }
    }
}

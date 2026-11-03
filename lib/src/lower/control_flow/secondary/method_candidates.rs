//! Method candidate selection and substitution inference.

use crate::hir::*;
use crate::lexer::Span;
use crate::lower::Lowerer;
use crate::types::{GenericParamId, TraitBound, Type};
use std::collections::HashMap;

impl Lowerer {
    pub(super) fn current_function(&self, id: crate::ids::DefId) -> Option<&HirFunction> {
        self.items.function(id)
    }

    #[cfg(test)]
    fn fixed_array_len_from_type_name(type_name: &str) -> Option<usize> {
        let inner = type_name.strip_prefix('[')?.strip_suffix(']')?;
        let (_, len) = inner.split_once(';')?;
        len.trim().parse().ok()
    }

    #[cfg(test)]
    pub(super) fn receiver_type_name_matches(candidate: &str, lookup: &str) -> bool {
        if candidate == lookup {
            return true;
        }

        fn borrowed_slice_elem(name: &str) -> Option<&str> {
            name.strip_prefix("&[")
                .and_then(|name| name.strip_suffix(']'))
                .filter(|name| !name.contains(';'))
        }

        fn mut_borrowed_slice_elem(name: &str) -> Option<&str> {
            name.strip_prefix("&mut [")
                .and_then(|name| name.strip_suffix(']'))
                .filter(|name| !name.contains(';'))
        }

        let is_generic_elem = |name: &str| {
            !matches!(
                name,
                "I8" | "I16"
                    | "I32"
                    | "I64"
                    | "U8"
                    | "U16"
                    | "U32"
                    | "U64"
                    | "F32"
                    | "F64"
                    | "Bool"
                    | "Char"
                    | "Str"
            )
        };

        if let (Some(candidate_elem), Some(lookup_elem)) =
            (borrowed_slice_elem(candidate), borrowed_slice_elem(lookup))
        {
            return candidate_elem == lookup_elem || is_generic_elem(candidate_elem);
        }

        if let (Some(candidate_elem), Some(lookup_elem)) = (
            mut_borrowed_slice_elem(candidate),
            mut_borrowed_slice_elem(lookup),
        ) {
            return candidate_elem == lookup_elem || is_generic_elem(candidate_elem);
        }

        match (
            Self::fixed_array_len_from_type_name(candidate),
            Self::fixed_array_len_from_type_name(lookup),
        ) {
            (Some(candidate_len), Some(lookup_len)) => candidate_len == lookup_len,
            _ => false,
        }
    }

    pub(crate) fn concrete_method_candidate(
        &mut self,
        mut recv: HirExpr,
        method_name: &str,
    ) -> Option<crate::selection::SelectedMethod> {
        fn collect_type_vars(ty: &Type, vars: &mut Vec<crate::ids::TypeVarId>) {
            crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
                if let Type::TypeVar(var) = nested {
                    if !vars.contains(var) {
                        vars.push(*var);
                    }
                }
            });
        }

        let mut receiver_vars = Vec::new();
        let resolved_receiver = self.engine.resolve(&recv.ty);
        collect_type_vars(&resolved_receiver, &mut receiver_vars);
        if let Type::TypeVar(id) = resolved_receiver {
            if self.engine.get_bounds(id).is_empty() {
                return None;
            }
        }
        for var in receiver_vars {
            if let Some(default) = self
                .constraint_store
                .literal_default_type_for_representative(var, |constrained| {
                    match self.engine.resolve(&Type::TypeVar(constrained)) {
                        Type::TypeVar(representative) => Some(representative),
                        _ => None,
                    }
                })
            {
                let _ = self.engine.unify(&Type::TypeVar(var), &default);
            }
        }
        recv.ty = self.engine.resolve(&recv.ty);
        if let Some(selected) = self.callable_method_candidate(recv.clone(), Some(method_name)) {
            return Some(selected);
        }
        let recv_span = recv.span.clone();
        let receiver_candidates = self.receiver_adjustment_candidates(recv);
        // Prefer proven adjusted candidates without discarding unresolved generic fallbacks.
        let mut deferred_candidate = None;
        for receiver_candidate in &receiver_candidates {
            let mut concrete = self.selection_service().select_concrete_method_candidates(
                std::slice::from_ref(receiver_candidate),
                method_name,
                |ty| self.resolve_projection_type(&self.engine.resolve(ty)),
            );
            let has_proven_candidate = concrete
                .iter()
                .any(|candidate| candidate.pending_impl_bounds.is_empty());
            if has_proven_candidate {
                concrete.retain(|candidate| candidate.pending_impl_bounds.is_empty());
            }
            crate::selection::SelectionService::prefer_exact_receiver(
                &mut concrete,
                receiver_candidate.adjustment,
            );
            match concrete.len() {
                0 => {}
                1 if concrete[0].pending_impl_bounds.is_empty() => return concrete.pop(),
                1 => {
                    if deferred_candidate.is_none() {
                        deferred_candidate = concrete.pop();
                    }
                }
                _ => {
                    if crate::type_services::visit::type_any(
                        &receiver_candidate.expr.ty,
                        |nested| {
                            matches!(nested, Type::TypeVar(_) | Type::Generic(_) | Type::Error)
                        },
                    ) {
                        continue;
                    }
                    let receiver_type = self.display_type(&receiver_candidate.expr.ty);
                    self.diagnostics.push_selection_with_span(
                        format!(
                            "Ambiguous selection for '{}' on type {}: multiple matching implementations",
                            method_name, receiver_type
                        ),
                        recv_span.clone(),
                    );
                    return None;
                }
            }
        }

        if deferred_candidate.is_some() {
            return deferred_candidate;
        }

        for candidate in &receiver_candidates {
            let candidate_ty =
                self.resolve_projection_type(&self.engine.resolve(&candidate.expr.ty));
            let receiver_is_current_trait_self = self.current_trait_self_receiver(&candidate_ty);
            let result = self.selection_service().select_current_trait_method(
                candidate.expr.clone(),
                method_name,
                receiver_is_current_trait_self,
            );
            if let Some(selected) = self.handle_optional_selection(result, recv_span.clone()) {
                return Some(selected);
            }
        }

        if self
            .selection_service()
            .mut_receiver_method_requires_mutable_receiver(
                &receiver_candidates,
                method_name,
                |ty| self.resolve_projection_type(&self.engine.resolve(ty)),
            )
        {
            self.diagnostics.push_selection_with_span(
                format!(
                    "Cannot call mutable receiver method '{}' without a mutable receiver",
                    method_name
                ),
                recv_span,
            );
        }

        None
    }

    pub(crate) fn selected_method_call_types(
        &mut self,
        selected: &crate::selection::SelectedMethod,
        args: Vec<HirExpr>,
    ) -> Option<(HirFunction, Type, Vec<HirExpr>, HirMethodCallTarget)> {
        let mut instantiated = selected.clone();
        self.instantiate_selected_callable_parameters(&mut instantiated);
        let selected = &instantiated;
        let method_func = selected.function.clone()?;
        let recv_resolved = self.engine.resolve(&selected.receiver.ty);
        let operation_span = selected.receiver.span.clone();
        let initial_method_subst = self.infer_selected_method_substitution(
            selected,
            &recv_resolved,
            &method_func,
            &args,
            operation_span.clone(),
        );

        let mut coerced_args = Vec::with_capacity(args.len());
        for (index, arg) in args.into_iter().enumerate() {
            if let Some(param) = selected.substituted_params.get(index) {
                let substituted_ty = if initial_method_subst.is_empty() {
                    param.ty.clone()
                } else {
                    param.ty.substitute_generics(&initial_method_subst)
                };
                coerced_args.push(self.coerce_argument_to_expected(arg, &substituted_ty));
            } else {
                coerced_args.push(arg);
            }
        }

        let method_subst = self.infer_selected_method_substitution(
            selected,
            &recv_resolved,
            &method_func,
            &coerced_args,
            operation_span,
        );
        let ret_ty = self
            .engine
            .resolve(&selected.return_type)
            .substitute_generics(&method_subst);
        let ret_ty = self.resolve_selected_projection_type(selected, &ret_ty);
        self.record_pending_impl_bounds(selected, &method_subst, "method call");
        self.record_function_generic_bounds(
            &method_func,
            &method_subst,
            selected.receiver.span.clone(),
            "method call",
        );
        let target = selected.target_with_substitution(&method_subst, |ty| self.engine.resolve(ty));

        Some((method_func, ret_ty, coerced_args, target))
    }

    pub(crate) fn record_function_generic_bounds(
        &mut self,
        func: &HirFunction,
        subst: &HashMap<GenericParamId, Type>,
        span: Span,
        context: &str,
    ) {
        for (generic_param, bounds) in &func.generic_bounds {
            let ty = subst
                .get(generic_param)
                .cloned()
                .unwrap_or(Type::Generic(*generic_param));
            let ty = self.engine.resolve(&ty);

            for bound in bounds {
                let bound = TraitBound {
                    trait_id: bound.trait_id,
                    type_args: bound
                        .type_args
                        .iter()
                        .map(|arg| self.engine.resolve(&arg.substitute_generics(subst)))
                        .collect(),
                };
                if let Type::TypeVar(id) = ty {
                    self.engine.add_bound(id, bound.clone());
                    self.constraint_store.add_trait(
                        Type::TypeVar(id),
                        bound,
                        span.clone(),
                        context,
                    );
                } else {
                    self.constraint_store
                        .add_trait(ty.clone(), bound, span.clone(), context);
                }
            }
        }
    }

    pub(super) fn fresh_method_generic_subst(
        &mut self,
        method_func: &HirFunction,
        span: Span,
    ) -> HashMap<GenericParamId, Type> {
        method_func
            .generic_params
            .iter()
            .map(|param| {
                (
                    param.id,
                    self.engine
                        .fresh_type_var_at_kind(span.clone(), param.kind.clone()),
                )
            })
            .collect()
    }

    pub(super) fn infer_selected_method_substitution(
        &mut self,
        selected: &crate::selection::SelectedMethod,
        recv_ty: &Type,
        method_func: &HirFunction,
        args: &[HirExpr],
        span: Span,
    ) -> HashMap<GenericParamId, Type> {
        let mut subst = self.infer_method_substitution(recv_ty, method_func, args, span.clone());
        for (param, ty) in &selected.owner_substitution {
            subst.insert(*param, ty.clone());
        }
        for param in &selected.owner_generic_params {
            let kind = self
                .engine
                .kind_of(&Type::Generic(*param))
                .unwrap_or(crate::type_services::kind::Kind::Type);
            subst
                .entry(*param)
                .or_insert_with(|| self.engine.fresh_type_var_at_kind(span.clone(), kind));
        }
        subst
    }

    pub(super) fn resolve_selected_projection_type(
        &self,
        selected: &crate::selection::SelectedMethod,
        ty: &Type,
    ) -> Type {
        let ty = match ty {
            Type::Projection {
                ty: base_ty,
                trait_id,
                assoc_type,
                trait_args,
            } => {
                if selected.target.trait_id() == Some(*trait_id) && assoc_type.owner == *trait_id {
                    if let Some(assoc) = selected
                        .associated_types
                        .iter()
                        .find(|assoc| assoc.id == assoc_type.assoc_type_id)
                    {
                        return self.resolve_selected_projection_type(selected, &assoc.ty);
                    }
                }

                Type::Projection {
                    ty: Box::new(self.resolve_selected_projection_type(selected, base_ty)),
                    trait_id: *trait_id,
                    assoc_type: *assoc_type,
                    trait_args: trait_args
                        .iter()
                        .map(|arg| self.resolve_selected_projection_type(selected, arg))
                        .collect(),
                }
            }
            Type::Reference { mutable, inner } => Type::Reference {
                mutable: *mutable,
                inner: Box::new(self.resolve_selected_projection_type(selected, inner)),
            },
            Type::Pointer(inner) => Type::Pointer(Box::new(
                self.resolve_selected_projection_type(selected, inner),
            )),
            Type::Slice(inner) => Type::Slice(Box::new(
                self.resolve_selected_projection_type(selected, inner),
            )),
            Type::Array(inner, len) => Type::Array(
                Box::new(self.resolve_selected_projection_type(selected, inner)),
                *len,
            ),
            Type::Tuple(elems) => Type::Tuple(
                elems
                    .iter()
                    .map(|elem| self.resolve_selected_projection_type(selected, elem))
                    .collect(),
            ),
            Type::Function {
                params,
                ret,
                safety,
                callable_kind,
                captures,
            } => Type::Function {
                params: params
                    .iter()
                    .map(|param| self.resolve_selected_projection_type(selected, param))
                    .collect(),
                ret: Box::new(self.resolve_selected_projection_type(selected, ret)),
                safety: *safety,
                callable_kind: *callable_kind,
                captures: captures
                    .iter()
                    .map(|capture| crate::types::FunctionCapture {
                        kind: capture.kind,
                        ty: self.resolve_selected_projection_type(selected, &capture.ty),
                    })
                    .collect(),
            },
            Type::Struct { id, args } => Type::Struct {
                id: *id,
                args: args
                    .iter()
                    .map(|arg| self.resolve_selected_projection_type(selected, arg))
                    .collect(),
            },
            Type::Enum { id, args } => Type::Enum {
                id: *id,
                args: args
                    .iter()
                    .map(|arg| self.resolve_selected_projection_type(selected, arg))
                    .collect(),
            },
            _ => ty.clone(),
        };

        self.resolve_projection_type(&ty)
    }

    pub(crate) fn record_pending_impl_bounds(
        &mut self,
        selected: &crate::selection::SelectedMethod,
        method_subst: &HashMap<GenericParamId, Type>,
        context: &str,
    ) {
        for (ty, bound) in &selected.pending_impl_bounds {
            let ty = self.engine.resolve(&ty.substitute_generics(method_subst));
            let bound = TraitBound {
                trait_id: bound.trait_id,
                type_args: bound
                    .type_args
                    .iter()
                    .map(|arg| self.engine.resolve(&arg.substitute_generics(method_subst)))
                    .collect(),
            };
            if let Type::TypeVar(id) = ty {
                self.engine.add_bound(id, bound.clone());
                self.constraint_store.add_trait(
                    Type::TypeVar(id),
                    bound,
                    selected.receiver.span.clone(),
                    context,
                );
            } else {
                self.constraint_store
                    .add_trait(ty, bound, selected.receiver.span.clone(), context);
            }
        }
    }

    fn current_trait_self_receiver(&self, ty: &Type) -> bool {
        self.current_trait.is_some()
            && (matches!(ty, Type::TypeVar(_))
                || matches!(ty, Type::Generic(param) if self.generic_display_name(*param) == Some("Self")))
    }

    pub(crate) fn selection_service(&self) -> crate::selection::SelectionService<'_> {
        crate::selection::SelectionService::new(
            self.items.traits_for_selection(),
            self.items.impls_for_selection(),
            self.language_items
                .sized
                .as_ref()
                .map(|items| items.trait_id),
            self.current_trait_id,
            self.current_impl_bounds(),
        )
        .with_effective_trait_methods(&self.imported_effective_trait_methods)
    }

    pub(super) fn selected_associated_type(
        &self,
        selected: &crate::selection::SelectedMethod,
        trait_def: &HirTrait,
        assoc_type_id: crate::ids::AssocTypeId,
    ) -> Option<Type> {
        let assoc = trait_def
            .associated_types
            .iter()
            .find(|assoc| assoc.id == assoc_type_id)?;
        let ty = selected
            .associated_types
            .iter()
            .find(|item| item.id == assoc.id)
            .map(|item| item.ty.clone())?;
        Some(self.resolve_projection_type(&self.engine.resolve(&ty)))
    }
}

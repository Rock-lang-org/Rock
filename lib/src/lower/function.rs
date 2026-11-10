//! Function header and signature lowering methods

use std::collections::HashSet;

use crate::ast;
use crate::hir::*;
use crate::ids::{DefId, HirLocalId, IdGen, TypeVarId};
use crate::types::{GenericParamDecl, GenericParamId, TraitBound, Type};

use crate::lower::Lowerer;

fn simple_trait_bound(
    trait_bound: &ast::ParseType,
) -> Option<(&ast::ParseTypeInner, &[ast::ParseType])> {
    match trait_bound {
        ast::ParseType::Type(inner) => Some((inner, &inner.generics)),
        ast::ParseType::Application(application) => {
            let ast::ParseType::Type(inner) = application.constructor.as_ref() else {
                return None;
            };
            if !inner.generics.is_empty() {
                return None;
            }
            Some((inner, application.args.as_slice()))
        }
        _ => None,
    }
}

impl Lowerer {
    fn collect_generic_ids_from_type(ty: &Type, generic_params: &mut Vec<GenericParamId>) {
        crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
            if let Type::Generic(param) = nested {
                if !generic_params.contains(param) {
                    generic_params.push(*param);
                }
            }
        });
    }

    fn where_clause_generic_param_id(
        &mut self,
        owner: DefId,
        generic_params: &[String],
        type_param: &str,
        span: crate::lexer::Span,
    ) -> Option<GenericParamId> {
        let Some(index) = generic_params.iter().position(|param| param == type_param) else {
            self.diagnostics.push_with_span(
                format!("unknown generic parameter '{}' in where clause", type_param),
                span,
            );
            return None;
        };

        Some(GenericParamId {
            owner,
            index: index as u32,
        })
    }

    fn remap_generic_param_ids_in_type(
        ty: &mut Type,
        generic_ids: &[GenericParamId],
        new_owner: DefId,
    ) {
        crate::type_services::visit::remap_generic_params_in_place(ty, &mut |mut param| {
            if let Some(index) = generic_ids.iter().position(|id| *id == param) {
                param.owner = new_owner;
                param.index = index as u32;
            }
            param
        });
    }

    fn remap_generic_bounds_owner(
        bounds: &HirGenericBounds,
        generic_ids: &[GenericParamId],
        new_owner: DefId,
    ) -> HirGenericBounds {
        let mut remapped: HirGenericBounds = bounds
            .iter()
            .map(|(param, trait_bounds)| {
                let mut remapped_param = *param;
                if let Some(index) = generic_ids.iter().position(|id| id == param) {
                    remapped_param.owner = new_owner;
                    remapped_param.index = index as u32;
                }

                let mut trait_bounds = trait_bounds.clone();
                for bound in &mut trait_bounds {
                    for ty in &mut bound.type_args {
                        Self::remap_generic_param_ids_in_type(ty, generic_ids, new_owner);
                    }
                }

                (remapped_param, trait_bounds)
            })
            .collect();
        remapped.predicates = bounds.predicates.clone();
        for predicate in &mut remapped.predicates {
            match predicate {
                crate::types::Predicate::Trait { subject, args, .. } => {
                    Self::remap_generic_param_ids_in_type(subject, generic_ids, new_owner);
                    for arg in args {
                        Self::remap_generic_param_ids_in_type(arg, generic_ids, new_owner);
                    }
                }
            }
        }
        remapped
    }

    fn build_function_type(param_types: &[Type], ret_type: Type, is_curried: bool) -> Type {
        if is_curried {
            param_types.iter().rev().fold(ret_type, |acc, param| {
                Type::function(vec![param.clone()], acc)
            })
        } else {
            Type::function(param_types.to_vec(), ret_type)
        }
    }

    fn collect_type_var_ids(ty: &Type, type_vars: &mut HashSet<TypeVarId>) {
        crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
            if let Type::TypeVar(id) = nested {
                type_vars.insert(*id);
            }
        });
    }

    fn receiver_ty_for_self(
        &mut self,
        self_receiver: ast::SelfReceiverMode,
        span: crate::lexer::Span,
    ) -> Type {
        let base = if let Some(index) = self
            .current_generic_params()
            .iter()
            .position(|param| param == "Self")
        {
            self.current_generic_owner()
                .map(|owner| {
                    Type::Generic(GenericParamId {
                        owner,
                        index: index as u32,
                    })
                })
                .unwrap_or_else(|| self.engine.fresh_type_var_at(span.clone()))
        } else {
            self.engine.fresh_type_var_at(span)
        };

        match self_receiver {
            ast::SelfReceiverMode::Shared => Type::Reference {
                mutable: false,
                inner: Box::new(base),
            },
            ast::SelfReceiverMode::Mut => Type::Reference {
                mutable: true,
                inner: Box::new(base),
            },
            ast::SelfReceiverMode::Move => base,
        }
    }

    fn build_self_param_with_local_id(
        &mut self,
        self_receiver: ast::SelfReceiverMode,
        local_id: HirLocalId,
        span: crate::lexer::Span,
    ) -> HirParam {
        HirParam {
            name: "self".to_string(),
            local_id,
            ty: self.receiver_ty_for_self(self_receiver, span),
            mutable: matches!(self_receiver, ast::SelfReceiverMode::Mut),
            is_ref: false,
        }
    }

    pub(crate) fn lower_function_sig_with_id(
        &mut self,
        sig: &ast::FunctionSig,
        signature_id: DefId,
    ) -> HirFunctionSig {
        // Helper to flatten curried function types: A -> B -> C becomes params [A, B], ret C
        fn flatten_curried_type(ty: &Type) -> (Vec<Type>, Type) {
            match ty {
                Type::Function { params, ret, .. } => {
                    // Unparenthesized arrows are already parsed into one Function node with
                    // multiple params. A nested Function here is an explicit higher-order type.
                    (params.clone(), ret.as_ref().clone())
                }
                _ => (vec![], ty.clone()),
            }
        }

        let (lowered_type, receiver_ty, generic_owner, generic_context_params) =
            if let Some(owner) = self.current_generic_owner() {
                let ty = self.lower_parse_type(&sig.sig);
                let receiver_ty = sig.self_receiver.map(|self_receiver| {
                    self.receiver_ty_for_self(self_receiver, sig.name.span.clone())
                });
                (
                    ty,
                    receiver_ty,
                    owner,
                    self.current_generic_params().to_vec(),
                )
            } else {
                let owner = signature_id;
                let params = if sig.self_receiver.is_some() {
                    vec!["Self".to_string()]
                } else {
                    Vec::new()
                };
                let ((ty, receiver_ty), params) =
                    self.with_generic_context(owner, params, |lowerer| {
                        let ty = lowerer.lower_parse_type(&sig.sig);
                        let receiver_ty = sig.self_receiver.map(|self_receiver| {
                            lowerer.receiver_ty_for_self(self_receiver, sig.name.span.clone())
                        });
                        (ty, receiver_ty)
                    });
                (ty, receiver_ty, owner, params)
            };
        let mut generic_param_ids = Vec::new();
        Self::collect_generic_ids_from_type(&lowered_type, &mut generic_param_ids);
        if let Some(receiver_ty) = &receiver_ty {
            Self::collect_generic_ids_from_type(receiver_ty, &mut generic_param_ids);
        }
        let mut const_params = std::collections::HashSet::new();
        lowered_type.collect_array_len_params(&mut const_params);
        let param_kind = |param: &GenericParamId| {
            if const_params.contains(param) {
                crate::type_services::kind::Kind::Usize
            } else {
                crate::type_services::kind::Kind::Type
            }
        };
        let mut public_signature_generic_params = Vec::new();
        let mut hidden_signature_generic_params = Vec::new();
        for param in &generic_param_ids {
            if param.owner == generic_owner {
                if let Some(name) = generic_context_params.get(param.index as usize) {
                    if name != "Self" {
                        public_signature_generic_params
                            .push(GenericParamDecl::new(*param, name.clone(), param_kind(param)));
                    } else {
                        hidden_signature_generic_params
                            .push(GenericParamDecl::new(*param, name.clone(), param_kind(param)));
                    }
                }
            }
        }
        let mut generic_params = public_signature_generic_params;
        generic_params.extend(hidden_signature_generic_params);
        let (mut params, ret) = flatten_curried_type(&lowered_type);
        if let Some(receiver_ty) = receiver_ty {
            params.insert(0, receiver_ty);
        }
        let mut generic_bounds = HirGenericBounds::new();
        for clause in &sig.where_clauses {
            let Some(trait_bound) = clause.trait_bound.as_ref() else {
                continue;
            };
            let trait_bound_span = clause
                .trait_bound
                .as_ref()
                .expect("trait bound checked above")
                .span();
            let Some((trait_bound, type_args)) = simple_trait_bound(trait_bound) else {
                self.diagnostics.push_with_span(
                    "unsupported trait bound shape in where clause".to_string(),
                    clause.subject.span(),
                );
                continue;
            };
            let Some(trait_id) = crate::lower::resolution::LowerResolutionContext::new(self)
                .resolve_trait_id(&trait_bound.name)
            else {
                self.diagnostics.push_with_span(
                    format!("unknown trait '{}' in where clause", trait_bound.name),
                    trait_bound_span,
                );
                continue;
            };
            let declared_param_name = match &clause.subject {
                ast::ParseType::Type(subject) if subject.generics.is_empty() => {
                    Some(subject.name.as_str())
                }
                ast::ParseType::Application(application)
                    if application
                        .args
                        .iter()
                        .all(|arg| matches!(arg, ast::ParseType::Hole(_))) =>
                {
                    match application.constructor.as_ref() {
                        ast::ParseType::Type(subject) if subject.generics.is_empty() => {
                            Some(subject.name.as_str())
                        }
                        _ => None,
                    }
                }
                _ => None,
            };
            let subject = if let Some(name) = declared_param_name {
                self.where_clause_generic_param_id(
                    generic_owner,
                    &generic_context_params,
                    name,
                    clause.subject.span(),
                )
                .map(Type::Generic)
                .unwrap_or(Type::Error)
            } else {
                self.with_generic_context(
                    generic_owner,
                    generic_context_params.clone(),
                    |lowerer| lowerer.lower_parse_type(&clause.subject),
                )
                .0
            };
            let type_args = type_args
                .iter()
                .map(|generic| {
                    self.with_generic_context(
                        generic_owner,
                        generic_context_params.clone(),
                        |lowerer| lowerer.lower_parse_type(generic),
                    )
                    .0
                })
                .collect::<Vec<_>>();
            generic_bounds
                .predicates
                .push(crate::types::Predicate::Trait {
                    subject: subject.clone(),
                    trait_id,
                    args: type_args.clone(),
                });
            if let Type::Generic(param) = subject {
                generic_bounds.entry(param).or_default().push(TraitBound {
                    trait_id,
                    type_args,
                });
            }
        }

        HirFunctionSig {
            id: signature_id,
            name: sig.name.name.clone(),
            generic_params,
            params,
            ret,
            generic_bounds,
            self_receiver: sig.self_receiver.map(receiver_mode),
            is_unsafe: sig.is_unsafe,
        }
    }

    pub(crate) fn lower_function_decl_header_with_sig_and_id(
        &mut self,
        fd: &ast::FunctionDecl,
        sig: &HirFunctionSig,
        id: crate::ids::DefId,
    ) -> HirFunction {
        let lambda = &fd.lambda;
        let is_curried = matches!(lambda.arrow_kind, ast::LambdaArrowKind::Curried);
        let mut func_type_vars = HashSet::new();

        let mut sig_params = sig.params.clone();
        let mut sig_ret = sig.ret.clone();
        let mut hidden_receiver_generic_ids = Vec::new();
        if fd.self_receiver.is_some() {
            if let Some(receiver_ty) = sig.params.first() {
                Self::collect_generic_ids_from_type(receiver_ty, &mut hidden_receiver_generic_ids);
            }
        }
        let mut public_signature_owned_generic_param_ids = Vec::new();
        let mut hidden_signature_owned_generic_param_ids = Vec::new();
        for generic_id in sig
            .generic_params
            .iter()
            .map(|param| param.id)
            .filter(|generic_id| generic_id.owner == sig.id && sig.id != id)
        {
            if hidden_receiver_generic_ids.contains(&generic_id) {
                hidden_signature_owned_generic_param_ids.push(generic_id);
            } else {
                public_signature_owned_generic_param_ids.push(generic_id);
            }
        }
        let mut signature_owned_generic_param_ids = public_signature_owned_generic_param_ids;
        signature_owned_generic_param_ids.extend(hidden_signature_owned_generic_param_ids);
        for ty in &mut sig_params {
            Self::remap_generic_param_ids_in_type(ty, &signature_owned_generic_param_ids, id);
        }
        Self::remap_generic_param_ids_in_type(&mut sig_ret, &signature_owned_generic_param_ids, id);
        let generic_bounds = Self::remap_generic_bounds_owner(
            &sig.generic_bounds,
            &signature_owned_generic_param_ids,
            id,
        );

        let mut used_generic_param_ids = Vec::new();
        for ty in &sig_params {
            Self::collect_generic_ids_from_type(ty, &mut used_generic_param_ids);
        }
        Self::collect_generic_ids_from_type(&sig_ret, &mut used_generic_param_ids);

        let mut generic_params = Vec::new();
        let mut public_entries = Vec::new();
        let mut hidden_entries = Vec::new();
        for generic in &sig.generic_params {
            if hidden_receiver_generic_ids.contains(&generic.id) {
                hidden_entries.push(generic.clone());
            } else {
                public_entries.push(generic.clone());
            }
        }
        public_entries.extend(hidden_entries);
        for generic in public_entries {
            let generic_id = generic.id;
            let remapped = if let Some(index) = signature_owned_generic_param_ids
                .iter()
                .position(|id| id == &generic_id)
            {
                GenericParamId {
                    owner: id,
                    index: index as u32,
                }
            } else {
                generic_id
            };
            if used_generic_param_ids.contains(&remapped) {
                generic_params.push(GenericParamDecl::new(remapped, generic.name, generic.kind));
            }
        }
        let generic_bounds = generic_bounds
            .into_iter()
            .filter(|(param, _)| generic_params.iter().any(|generic| generic.id == *param))
            .collect();

        let mut all_params = Vec::new();
        let mut local_ids = IdGen::<HirLocalId>::new();

        // If this is a method, add the implicit self parameter.
        if let Some(self_receiver) = fd.self_receiver {
            let local_id = local_ids.fresh();
            let mut self_param = self.build_self_param_with_local_id(
                self_receiver,
                local_id,
                fd.lambda.span.clone(),
            );
            if let Some(sig_self_ty) = sig_params.first() {
                self_param.ty = sig_self_ty.clone();
            }
            Self::collect_type_var_ids(&self_param.ty, &mut func_type_vars);
            all_params.push(self_param);
        }

        // Merge parameter names from declaration with types from signature
        let param_start = if fd.self_receiver.is_some() { 1 } else { 0 };
        for (i, param) in lambda.parameters.iter().enumerate() {
            let (name, _decl_ty, mutable, is_ref) =
                self.lower_param_pattern_at(param, fd.lambda.span.clone());
            let ty = if param_start + i < sig.params.len() {
                sig_params[param_start + i].clone()
            } else {
                self.engine.fresh_type_var_at(
                    Self::pattern_binding_span(param).unwrap_or_else(|| fd.lambda.span.clone()),
                )
            };
            Self::collect_type_var_ids(&ty, &mut func_type_vars);
            let local_id = local_ids.fresh();
            all_params.push(HirParam {
                name,
                local_id,
                ty,
                mutable,
                is_ref,
            });
        }

        let outer_param_count = if is_curried && !lambda.parameters.is_empty() {
            if fd.self_receiver.is_some() {
                2
            } else {
                1
            }
        } else {
            all_params.len()
        };
        let params = all_params[..outer_param_count].to_vec();
        let remaining_param_types: Vec<Type> = all_params[outer_param_count..]
            .iter()
            .map(|param| param.ty.clone())
            .collect();
        let ret_type = if is_curried {
            Self::build_function_type(&remaining_param_types, sig_ret.clone(), true)
        } else {
            sig_ret.clone()
        };
        Self::collect_type_var_ids(&ret_type, &mut func_type_vars);
        self.function_type_vars.insert(id, func_type_vars);

        HirFunction {
            id,
            name: fd.name.name.clone(),
            generic_params,
            generic_bounds,
            params,
            ret_type,
            body: HirBlock {
                stmts: vec![],
                ty: Type::Unit,
            },
            is_curried,
            is_method: fd.self_receiver.is_some(),
            self_receiver: fd.self_receiver.map(receiver_mode),
            is_unsafe: fd.is_unsafe,
        }
    }

    pub(crate) fn lower_param_pattern_at(
        &mut self,
        pattern: &ast::Pattern,
        fallback: crate::lexer::Span,
    ) -> (String, Type, bool, bool) {
        match &pattern.kind {
            ast::PatternKind::Reference {
                pattern: inner,
                mutable,
            } => {
                // Reference pattern: &name or &mut name
                // Recursively lower the inner pattern
                let (name, ty, inner_mut, _) = self.lower_param_pattern_at(inner, fallback.clone());
                // The parameter is a reference
                (name, ty, inner_mut || *mutable, true)
            }
            ast::PatternKind::Nested(inner) => self.lower_param_pattern_at(inner, fallback.clone()),
            ast::PatternKind::Ident(ident_pat) => {
                let ty = self.engine.fresh_type_var_at(ident_pat.name.span.clone());
                (ident_pat.name.name.clone(), ty, ident_pat.mut_, false)
            }
            ast::PatternKind::Wildcard => {
                let ty = self.engine.fresh_type_var_at(fallback.clone());
                ("_".to_string(), ty, false, false)
            }
            _ => {
                let ty = self.engine.fresh_type_var_at(fallback);
                // For complex patterns, use a generated name
                let name = pattern
                    .binding
                    .as_ref()
                    .map(|b| b.name.clone())
                    .unwrap_or_else(|| "_arg".to_string());
                (name, ty, false, false)
            }
        }
    }
}

fn receiver_mode(mode: ast::SelfReceiverMode) -> crate::types::ReceiverMode {
    match mode {
        ast::SelfReceiverMode::Shared => crate::types::ReceiverMode::Shared,
        ast::SelfReceiverMode::Mut => crate::types::ReceiverMode::Mut,
        ast::SelfReceiverMode::Move => crate::types::ReceiverMode::Move,
    }
}

#[cfg(test)]
mod tests;
